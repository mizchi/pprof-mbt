//! Convert wasmtime's GuestProfiler JSON (Firefox "processed profile" format)
//! into gzip'd pprof. Keeps the conversion in-process so the runner can emit
//! pprof directly without shelling out to a Node script.
//!
//! The Firefox profile fields we read mirror what
//! `runners/lib/firefox-to-pprof.mjs` reads — see that file for the format
//! reference. Anything irrelevant to CPU sample data we simply ignore via
//! `serde(default)`.

use std::collections::HashMap;
use std::io::Write;

use anyhow::Result;
use flate2::Compression;
use flate2::write::GzEncoder;
use prost::Message;
use serde::Deserialize;

use crate::demangle;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/perftools.profiles.rs"));
}

#[derive(Deserialize)]
pub struct FirefoxProfile {
    #[serde(default)]
    pub libs: Vec<Lib>,
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub meta: Meta,
}

#[derive(Deserialize, Default)]
pub struct Meta {
    #[serde(default = "default_interval")]
    pub interval: f64, // ms
    #[serde(rename = "startTime")]
    #[serde(default)]
    pub start_time: f64, // ms since epoch
}

fn default_interval() -> f64 {
    1.0
}

#[derive(Deserialize)]
pub struct Lib {
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize)]
pub struct Thread {
    #[serde(rename = "frameTable")]
    pub frame_table: FrameTable,
    #[serde(rename = "funcTable")]
    pub func_table: FuncTable,
    #[serde(rename = "stackTable")]
    pub stack_table: StackTable,
    pub samples: Samples,
    #[serde(rename = "stringArray")]
    pub string_array: Vec<String>,
}

/// Firefox's tables are stored as a struct of parallel arrays. We deserialize
/// each column we care about.
#[derive(Deserialize)]
#[allow(dead_code)] // `length` is kept for symmetry with the Firefox schema
pub struct FrameTable {
    pub length: usize,
    pub func: Vec<i64>,
    #[serde(default)]
    pub address: Vec<i64>,
    #[serde(default)]
    pub line: Vec<Option<i64>>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct FuncTable {
    pub length: usize,
    pub name: Vec<i64>,
    #[serde(rename = "fileName")]
    #[serde(default)]
    pub file_name: Vec<Option<i64>>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct StackTable {
    pub length: usize,
    pub prefix: Vec<Option<i64>>,
    pub frame: Vec<i64>,
}

#[derive(Deserialize)]
pub struct Samples {
    pub length: usize,
    pub stack: Vec<Option<i64>>,
    #[serde(rename = "timeDeltas")]
    #[serde(default)]
    pub time_deltas: Vec<f64>,
    #[serde(default)]
    pub weight: Vec<f64>,
}

/// Builds a pprof Profile and returns gzip'd protobuf bytes.
pub fn convert(profile: &FirefoxProfile) -> Result<Vec<u8>> {
    let mut b = ProfileBuilder::new();
    b.run(profile);
    let proto = b.finish(profile);
    let mut buf = Vec::new();
    proto.encode(&mut buf)?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&buf)?;
    Ok(gz.finish()?)
}

struct ProfileBuilder {
    strings: Vec<String>,
    string_index: HashMap<String, i64>,
    functions: Vec<proto::Function>,
    func_index: HashMap<String, u64>, // raw name + file → function id
    locations: Vec<proto::Location>,
    loc_index: HashMap<String, u64>, // canonical key → location id
    frame_to_loc: HashMap<(usize, i64), u64>, // (thread idx, frame idx) → location id
    samples: Vec<proto::Sample>,
    total_ns: i64,
}

impl ProfileBuilder {
    fn new() -> Self {
        let mut me = Self {
            // pprof requires string_table[0] to be the empty string.
            strings: vec![String::new()],
            string_index: HashMap::from([(String::new(), 0)]),
            functions: Vec::new(),
            func_index: HashMap::new(),
            locations: Vec::new(),
            loc_index: HashMap::new(),
            frame_to_loc: HashMap::new(),
            samples: Vec::new(),
            total_ns: 0,
        };
        // Pre-intern the canonical sample-type / period-type strings.
        me.intern("samples");
        me.intern("count");
        me.intern("cpu");
        me.intern("nanoseconds");
        me
    }

    fn intern(&mut self, s: &str) -> i64 {
        if let Some(&id) = self.string_index.get(s) {
            return id;
        }
        let id = self.strings.len() as i64;
        self.strings.push(s.to_string());
        self.string_index.insert(s.to_string(), id);
        id
    }

    fn intern_function(&mut self, raw_name: &str, file: &str) -> u64 {
        let key = format!("{raw_name}\x1f{file}");
        if let Some(&id) = self.func_index.get(&key) {
            return id;
        }
        let pretty = demangle::symbol(raw_name);
        let id = (self.functions.len() + 1) as u64;
        let name = self.intern(&pretty);
        let system_name = self.intern(raw_name);
        let filename = self.intern(file);
        self.functions.push(proto::Function {
            id,
            name,
            system_name,
            filename,
            start_line: 0,
        });
        self.func_index.insert(key, id);
        id
    }

    /// Resolve a thread's frame to a single-line Location. wasmtime profiles
    /// don't carry inline-frame chains, so we always emit one Line per Location.
    fn intern_location(
        &mut self,
        thread_idx: usize,
        thread: &Thread,
        frame_idx: i64,
    ) -> u64 {
        if let Some(&id) = self.frame_to_loc.get(&(thread_idx, frame_idx)) {
            return id;
        }
        let fi = frame_idx as usize;
        let func_idx = thread.frame_table.func[fi] as usize;
        let raw_name = thread
            .string_array
            .get(thread.func_table.name[func_idx] as usize)
            .map(String::as_str)
            .unwrap_or("(anonymous)");
        let file = thread
            .func_table
            .file_name
            .get(func_idx)
            .and_then(|s| s.as_ref())
            .and_then(|&idx| thread.string_array.get(idx as usize))
            .map(String::as_str)
            .unwrap_or("");
        let line_no = thread
            .frame_table
            .line
            .get(fi)
            .and_then(|s| *s)
            .unwrap_or(0);
        let address = thread
            .frame_table
            .address
            .get(fi)
            .copied()
            .unwrap_or(0)
            .max(0) as u64;

        let canonical = format!("{address}\x1f{raw_name}\x1f{file}\x1f{line_no}");
        if let Some(&id) = self.loc_index.get(&canonical) {
            self.frame_to_loc.insert((thread_idx, frame_idx), id);
            return id;
        }

        let func_id = self.intern_function(raw_name, file);
        let id = (self.locations.len() + 1) as u64;
        self.locations.push(proto::Location {
            id,
            mapping_id: 1,
            address,
            line: vec![proto::Line {
                function_id: func_id,
                line: line_no,
                column: 0,
            }],
            is_folded: false,
        });
        self.loc_index.insert(canonical, id);
        self.frame_to_loc.insert((thread_idx, frame_idx), id);
        id
    }

    fn run(&mut self, profile: &FirefoxProfile) {
        let interval_ms = if profile.meta.interval > 0.0 {
            profile.meta.interval
        } else {
            1.0
        };
        for (ti, thread) in profile.threads.iter().enumerate() {
            // Aggregate (count, ns) per stack id; emit one pprof Sample per group.
            let mut aggregate: HashMap<i64, (i64, i64)> = HashMap::new();
            let mut stack_cache: HashMap<i64, Vec<u64>> = HashMap::new();
            for i in 0..thread.samples.length {
                let stk = match thread.samples.stack.get(i).copied().flatten() {
                    Some(s) => s,
                    None => continue,
                };
                let count = thread
                    .samples
                    .weight
                    .get(i)
                    .copied()
                    .map(|w| w.round() as i64)
                    .unwrap_or(1)
                    .max(1);
                let dt = thread
                    .samples
                    .time_deltas
                    .get(i)
                    .copied()
                    .unwrap_or(interval_ms);
                let ns = (dt * 1_000_000.0).round() as i64;
                let entry = aggregate.entry(stk).or_insert((0, 0));
                entry.0 += count;
                entry.1 += ns;
            }
            for (stk, (count, ns)) in aggregate {
                let locs = stack_locations(thread, stk, &mut stack_cache);
                let location_id = locs
                    .iter()
                    .map(|&fi| self.intern_location(ti, thread, fi))
                    .collect();
                self.samples.push(proto::Sample {
                    location_id,
                    value: vec![count, ns],
                    label: vec![],
                });
                self.total_ns += ns;
            }
        }
    }

    fn finish(self, profile: &FirefoxProfile) -> proto::Profile {
        let interval_ns =
            ((profile.meta.interval.max(1.0)) * 1_000_000.0).round() as i64;
        let samples_str = self.string_index[""].wrapping_add(1); // = "samples"
        let count_str = samples_str + 1;
        let cpu_str = samples_str + 2;
        let ns_str = samples_str + 3;
        // Single mapping covering "main.wasm".
        let mapping_name = profile
            .libs
            .first()
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "wasm".to_string());
        let mapping_filename = self
            .string_index
            .get(&mapping_name)
            .copied()
            .unwrap_or_else(|| {
                // Should already exist via intern_function/etc, but be safe.
                self.strings
                    .iter()
                    .position(|s| s == &mapping_name)
                    .map(|i| i as i64)
                    .unwrap_or(0)
            });
        proto::Profile {
            sample_type: vec![
                proto::ValueType {
                    r#type: samples_str,
                    unit: count_str,
                },
                proto::ValueType {
                    r#type: cpu_str,
                    unit: ns_str,
                },
            ],
            sample: self.samples,
            mapping: vec![proto::Mapping {
                id: 1,
                memory_start: 0,
                memory_limit: 0,
                file_offset: 0,
                filename: mapping_filename,
                build_id: 0,
                has_functions: true,
                has_filenames: false,
                has_line_numbers: false,
                has_inline_frames: false,
            }],
            location: self.locations,
            function: self.functions,
            string_table: self.strings,
            drop_frames: 0,
            keep_frames: 0,
            time_nanos: (profile.meta.start_time * 1_000_000.0).round() as i64,
            duration_nanos: self.total_ns,
            period_type: Some(proto::ValueType {
                r#type: cpu_str,
                unit: ns_str,
            }),
            period: interval_ns,
            comment: vec![],
            default_sample_type: 0,
            doc_url: 0,
        }
    }
}

/// Walk a stack-table linked list, leaf first, returning the frame indices.
fn stack_locations(
    thread: &Thread,
    stack_id: i64,
    cache: &mut HashMap<i64, Vec<u64>>,
) -> Vec<i64> {
    if let Some(_cached) = cache.get(&stack_id) {
        // Cache key wasn't useful as frame-idx list — just rebuild; the
        // expensive part (intern_location) is memoised separately.
    }
    let mut acc = Vec::new();
    let mut s = Some(stack_id);
    while let Some(idx) = s {
        let i = idx as usize;
        let frame_idx = thread.stack_table.frame[i];
        acc.push(frame_idx);
        s = thread.stack_table.prefix.get(i).and_then(|p| *p);
    }
    cache.insert(stack_id, acc.iter().map(|&i| i as u64).collect());
    acc
}
