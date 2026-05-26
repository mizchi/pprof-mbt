//! Convert Linux `perf script` textual output into gzip'd pprof.
//!
//! Why textual `perf script` and not the binary `perf.data`?
//! The on-disk format is versioned, depends on which features the
//! recording kernel had compiled in, and parsing it well needs to
//! track the same matrix that `perf` itself does. `perf script` is
//! the stable textual contract that ships with every `perf` build,
//! so we shell out to it (or accept its captured output) and parse
//! that instead.
//!
//! Expected input — produced by something like:
//!
//!   perf record -F 999 -g -e cpu-clock -- ./main.exe
//!   perf script -F comm,pid,tid,time,event,ip,sym,dso > script.out
//!
//! Each sample is a header line followed by an indented call stack,
//! samples separated by blank lines:
//!
//!   main.exe 12345/12345    0.000000:    1000000 cpu-clock:
//!           ffff80008a9c0a3c some_function+0x10 (/path/to/lib.so)
//!           ffff80008a9c0b48 another_function+0x20 (/path/to/lib.so)
//!
//!   main.exe 12345/12345    0.001000:    1000000 cpu-clock:
//!           ffff80008a9c0d60 yet_another+0x40 (/path/to/lib.so)
//!
//! Aggregates by (resolved-symbol stack) → (count, sum-of-periods)
//! and emits pprof with two sample types: `samples`/count and
//! `<event>`/<unit> (default `cpu`/`nanoseconds` since the common
//! recipe above produces nanosecond-period cpu-clock samples).

use std::collections::HashMap;
use std::io::Write as _;

use anyhow::{Context as _, Result};
use firefox_to_pprof::proto;
use flate2::Compression;
use flate2::write::GzEncoder;
use prost::Message as _;

/// Knobs for callers that want non-default labelling. The defaults
/// match the `perf record -F 999 -g -e cpu-clock` recipe above.
pub struct ConvertOptions {
    /// Label for the second sample type's `type` field. Default `cpu`.
    pub event_type: String,
    /// Label for the second sample type's `unit` field. Default
    /// `nanoseconds`.
    pub event_unit: String,
    /// Skip running symbols through `moonbit_demangle::demangle`.
    pub no_demangle: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            event_type: "cpu".to_string(),
            event_unit: "nanoseconds".to_string(),
            no_demangle: false,
        }
    }
}

/// Parse the contents of a `perf script` capture and return a
/// gzip-compressed pprof Profile.
pub fn convert(input: &str, opts: &ConvertOptions) -> Result<Vec<u8>> {
    let samples = parse(input)?;
    encode(samples, opts)
}

/// Same as `convert` but takes pre-parsed samples. Useful when the
/// caller wants to inspect samples (e.g. compute stats / surface
/// warnings) before encoding.
pub fn convert_from_samples(
    samples: Vec<Sample>,
    opts: &ConvertOptions,
) -> Result<Vec<u8>> {
    encode(samples, opts)
}

/// Aggregate diagnostics over a parsed perf-script capture. Cheap —
/// O(samples × frames).
#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub sample_count: usize,
    pub frame_count: usize,
    pub unknown_frame_count: usize,
    pub period_sum: u64,
}

impl Stats {
    pub fn from_samples(samples: &[Sample]) -> Self {
        let mut s = Stats::default();
        s.sample_count = samples.len();
        for sample in samples {
            s.frame_count += sample.stack.len();
            s.period_sum += sample.period;
            for frame in &sample.stack {
                if is_unresolved(&frame.symbol) {
                    s.unknown_frame_count += 1;
                }
            }
        }
        s
    }

    /// True when every sample had period=1 (the parser's fallback when
    /// the header line lacked a numeric weight). Strong hint that the
    /// recording was missing `--weight` or that `perf script -F`
    /// dropped the `period` field.
    pub fn period_likely_missing(&self) -> bool {
        self.sample_count > 0 && self.period_sum == self.sample_count as u64
    }

    /// Fraction of frames that came back as `[unknown]` / empty.
    pub fn unknown_ratio(&self) -> f32 {
        if self.frame_count == 0 {
            0.0
        } else {
            self.unknown_frame_count as f32 / self.frame_count as f32
        }
    }
}

fn is_unresolved(sym: &str) -> bool {
    sym.is_empty() || sym == "[unknown]"
}

// ─────────────────────────── parser ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Sample {
    /// Frames bottom-to-top in source order (perf script prints leaf first).
    pub stack: Vec<Frame>,
    /// Sample weight from the perf header, e.g. number of cycles or
    /// nanoseconds depending on the event. 1 if perf omitted it.
    pub period: u64,
}

#[derive(Debug, Clone)]
pub struct Frame {
    /// Symbol name (demangled by caller). `[unknown]` if perf
    /// couldn't resolve it.
    pub symbol: String,
    /// Path of the binary/lib the symbol came from, for the pprof
    /// Mapping. Empty if perf didn't print one.
    pub dso: String,
}

pub fn parse(input: &str) -> Result<Vec<Sample>> {
    let mut samples: Vec<Sample> = Vec::new();
    let mut current: Option<Sample> = None;

    for raw in input.lines() {
        // Stack frame lines start with whitespace; sample headers
        // start at column 0. Blank lines flush the current sample.
        if raw.is_empty() {
            if let Some(s) = current.take() {
                samples.push(s);
            }
            continue;
        }
        if raw.starts_with(|c: char| c.is_whitespace()) {
            if let Some(s) = current.as_mut() {
                if let Some(frame) = parse_frame(raw.trim()) {
                    s.stack.push(frame);
                }
            }
            // Frames before any header — perf shouldn't emit these,
            // but if it does just drop them rather than bail.
            continue;
        }
        // New sample header. Flush any in-flight stack first; perf
        // does emit a blank line between samples, but be defensive.
        if let Some(s) = current.take() {
            samples.push(s);
        }
        current = Some(Sample {
            stack: Vec::new(),
            period: parse_header_period(raw).unwrap_or(1),
        });
    }
    if let Some(s) = current.take() {
        samples.push(s);
    }
    Ok(samples)
}

/// Header format (whitespace-collapsed):
///   <comm> <pid>/<tid> <ts>: <period> <event>:
/// We only care about <period>. It's the second-to-last token in the
/// trailing `<period> <event>:` segment. Older perf may omit it
/// (just `<ts>: <event>:`); return None then.
fn parse_header_period(header: &str) -> Option<u64> {
    // Trim trailing colon on the event name.
    let trimmed = header.trim_end_matches(':').trim_end();
    // Walk from the right: last token = event, second-to-last = period if numeric.
    let toks: Vec<&str> = trimmed.split_whitespace().collect();
    if toks.len() < 2 {
        return None;
    }
    toks[toks.len() - 2].parse::<u64>().ok()
}

/// Frame line, after trim_start, looks like:
///   "<hex-addr> <symbol>+<offset> (<dso>)"
///   "<hex-addr> [unknown] (<dso>)"
///   "<hex-addr> <symbol> (<dso>)"          (offset omitted)
/// We strip `+<hex>` from the symbol and pull the dso from the
/// trailing parens. If the line doesn't match, return None.
fn parse_frame(line: &str) -> Option<Frame> {
    // addr is the first whitespace-separated token; everything after
    // the first space up to the trailing " (dso)" is the symbol+offset.
    let (_addr, rest) = line.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    // Split off the trailing parenthesised dso, if any.
    let (sym_part, dso) = match rest.rfind(" (") {
        Some(idx) => {
            let dso_part = &rest[idx + 2..];
            let dso = dso_part.trim_end_matches(')').to_string();
            (&rest[..idx], dso)
        }
        None => (rest, String::new()),
    };
    // Strip trailing +0x... offset from the symbol.
    let symbol = match sym_part.rfind('+') {
        Some(idx) if sym_part[idx + 1..].starts_with("0x") => {
            sym_part[..idx].to_string()
        }
        _ => sym_part.to_string(),
    };
    Some(Frame { symbol, dso })
}

// ─────────────────────────── encoder ────────────────────────────────────

fn encode(samples: Vec<Sample>, opts: &ConvertOptions) -> Result<Vec<u8>> {
    let mut strings = StringPool::new();
    let samples_str = strings.intern("samples");
    let count_str = strings.intern("count");
    let event_str = strings.intern(&opts.event_type);
    let unit_str = strings.intern(&opts.event_unit);

    // Demangle once per unique raw symbol, cache the result.
    let mut demangled: HashMap<String, String> = HashMap::new();
    let demangle = |raw: &str, cache: &mut HashMap<String, String>| -> String {
        if opts.no_demangle {
            return raw.to_string();
        }
        if let Some(v) = cache.get(raw) {
            return v.clone();
        }
        let pretty = moonbit_demangle::demangle(raw);
        let out = if pretty == raw { raw.to_string() } else { pretty };
        cache.insert(raw.to_string(), out.clone());
        out
    };

    // Mapping per unique dso so pprof's UI can group by binary.
    let mut mapping_id: HashMap<String, u64> = HashMap::new();
    let mut mappings: Vec<proto::Mapping> = Vec::new();
    let mapping_for = |dso: &str,
                           mapping_id: &mut HashMap<String, u64>,
                           mappings: &mut Vec<proto::Mapping>,
                           strings: &mut StringPool|
     -> u64 {
        if let Some(&id) = mapping_id.get(dso) {
            return id;
        }
        let id = (mappings.len() + 1) as u64;
        let filename = strings.intern(dso);
        mappings.push(proto::Mapping {
            id,
            memory_start: 0,
            memory_limit: 0,
            file_offset: 0,
            filename,
            build_id: 0,
            has_functions: !dso.is_empty(),
            has_filenames: false,
            has_line_numbers: false,
            has_inline_frames: false,
        });
        mapping_id.insert(dso.to_string(), id);
        id
    };

    // (symbol, dso) → function/location id. Symbol alone isn't unique
    // when the same name appears in multiple libraries.
    let mut func_id: HashMap<(String, String), u64> = HashMap::new();
    let mut location_id: HashMap<(String, String), u64> = HashMap::new();
    let mut functions: Vec<proto::Function> = Vec::new();
    let mut locations: Vec<proto::Location> = Vec::new();

    // Aggregate: stack (as Vec<location_id>) → (count, sum_period).
    let mut agg: HashMap<Vec<u64>, (i64, i64)> = HashMap::new();
    for s in samples {
        if s.stack.is_empty() {
            continue;
        }
        let mut loc_stack: Vec<u64> = Vec::with_capacity(s.stack.len());
        for frame in &s.stack {
            let demangled_sym = demangle(&frame.symbol, &mut demangled);
            let key = (demangled_sym.clone(), frame.dso.clone());
            let loc = if let Some(&id) = location_id.get(&key) {
                id
            } else {
                let mid = mapping_for(
                    &frame.dso,
                    &mut mapping_id,
                    &mut mappings,
                    &mut strings,
                );
                let fid = if let Some(&id) = func_id.get(&key) {
                    id
                } else {
                    let id = (functions.len() + 1) as u64;
                    let name_str = strings.intern(&demangled_sym);
                    let raw_str = strings.intern(&frame.symbol);
                    let file_str = strings.intern(&frame.dso);
                    functions.push(proto::Function {
                        id,
                        name: name_str,
                        system_name: raw_str,
                        filename: file_str,
                        start_line: 0,
                    });
                    func_id.insert(key.clone(), id);
                    id
                };
                let id = (locations.len() + 1) as u64;
                locations.push(proto::Location {
                    id,
                    mapping_id: mid,
                    address: 0,
                    line: vec![proto::Line {
                        function_id: fid,
                        line: 0,
                        column: 0,
                    }],
                    is_folded: false,
                });
                location_id.insert(key, id);
                id
            };
            loc_stack.push(loc);
        }
        let entry = agg.entry(loc_stack).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += s.period as i64;
    }

    let mut pprof_samples: Vec<proto::Sample> = Vec::with_capacity(agg.len());
    for (loc_stack, (count, period_sum)) in agg {
        pprof_samples.push(proto::Sample {
            location_id: loc_stack,
            value: vec![count, period_sum],
            label: vec![],
        });
    }

    let profile = proto::Profile {
        sample_type: vec![
            proto::ValueType {
                r#type: samples_str,
                unit: count_str,
            },
            proto::ValueType {
                r#type: event_str,
                unit: unit_str,
            },
        ],
        sample: pprof_samples,
        mapping: mappings,
        location: locations,
        function: functions,
        string_table: strings.strings,
        drop_frames: 0,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: 0,
        period_type: Some(proto::ValueType {
            r#type: event_str,
            unit: unit_str,
        }),
        period: 1,
        comment: vec![],
        default_sample_type: event_str,
        doc_url: 0,
    };

    let mut buf = Vec::new();
    profile
        .encode(&mut buf)
        .context("encoding pprof Profile")?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&buf)?;
    Ok(gz.finish()?)
}

struct StringPool {
    strings: Vec<String>,
    index: HashMap<String, i64>,
}

impl StringPool {
    fn new() -> Self {
        Self {
            strings: vec![String::new()],
            index: HashMap::from([(String::new(), 0)]),
        }
    }
    fn intern(&mut self, s: &str) -> i64 {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = self.strings.len() as i64;
        self.strings.push(s.to_string());
        self.index.insert(s.to_string(), id);
        id
    }
}

