//! Convert Firefox Profiler's [processed-profile JSON][1] into gzip'd
//! [pprof][2].
//!
//! Designed as a thin pipeline: parse the JSON with serde, walk the
//! stack/frame/function tables, intern Function/Location entries with
//! content-based dedup, encode the [`pprof.proto`][3] Profile via
//! [`prost`], gzip, hand back the bytes.
//!
//! ## Producers we've tested against
//!
//! * [samply](https://github.com/mstange/samply) — Linux/macOS native sampler
//! * [wasmtime](https://wasmtime.dev/) `GuestProfiler` — wasm guest sampler
//!
//! Each producer disagrees on (a) how to resolve a frame index to a symbol
//! and (b) how to weigh samples. Callers describe just those bits via the
//! [`FrameResolver`] and [`SampleResolver`] traits — everything else is
//! shared.
//!
//! ## Symbol demangling
//!
//! By default, every resolved function name is passed through
//! [`moonbit_demangle::demangle`] so the output pprof reads as
//! `mizchi::bench::ackermann` rather than `_M0FP26mizchi5bench9ackermann`.
//! Override with [`Builder::demangle_with`] if you're profiling something
//! else.
//!
//! ## Example
//!
//! ```no_run
//! use firefox_to_pprof::{Builder, FirefoxProfile, FrameResolver, ResolvedFrame, SampleResolver, ResolvedSample, Thread};
//! # fn doctest(profile: &FirefoxProfile) -> anyhow::Result<()> {
//! struct MyFrames;
//! impl FrameResolver for MyFrames {
//!     fn resolve(&self, _profile: &FirefoxProfile, _thread: &Thread, _frame_idx: i64) -> Vec<ResolvedFrame> {
//!         vec![ResolvedFrame { name: "stub".into(), ..Default::default() }]
//!     }
//! }
//! struct MySamples;
//! impl SampleResolver for MySamples {
//!     fn resolve(&self, _profile: &FirefoxProfile, thread: &Thread, i: usize) -> ResolvedSample {
//!         ResolvedSample { stack: thread.samples.stack[i], count: 1, ns: 1_000_000 }
//!     }
//! }
//! let bytes = Builder::new(profile, MyFrames, MySamples).encode()?;
//! std::fs::write("out.pb.gz", bytes)?;
//! # Ok(())
//! # }
//! ```
//!
//! [1]: https://github.com/firefox-devtools/profiler/blob/main/docs-developer/processed-profile-format.md
//! [2]: https://github.com/google/pprof/blob/main/proto/profile.proto
//! [3]: https://github.com/google/pprof/blob/main/proto/profile.proto

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::io::Write;

use anyhow::Result;
use flate2::Compression;
use flate2::write::GzEncoder;
use prost::Message;

mod model;
pub use model::*;

mod resolver;
pub use resolver::*;

pub mod samply;

/// Generated pprof protobuf types (`perftools.profiles`). Re-exported so
/// callers that want to manipulate the raw `Profile` (add labels, attach
/// extra mappings, …) can do so without depending on `prost` themselves.
#[allow(missing_docs)] // prost-generated, fields are documented in profile.proto
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/perftools.profiles.rs"));
}

/// Demangler hook — `(name) → pretty name`. Defaults to
/// [`moonbit_demangle::demangle`].
pub type DemangleFn = Box<dyn Fn(&str) -> String>;

/// Builds a pprof Profile from a Firefox-format profile.
///
/// Provide:
///   * the parsed [`FirefoxProfile`]
///   * a [`FrameResolver`] that turns a `(thread, frame_idx)` into one or
///     more resolved frames (multiple = inline chain, leaf first)
///   * a [`SampleResolver`] that turns a `(thread, sample_idx)` into a
///     `(stack, count, ns)` triple
///
/// Then call [`Builder::encode`] for the gzip'd protobuf bytes, or
/// [`Builder::write`] to save to disk.
pub struct Builder<'a, F: FrameResolver, S: SampleResolver> {
    profile: &'a FirefoxProfile,
    frames: F,
    samples: S,
    demangle: DemangleFn,
    mapping_filename: Option<String>,
}

impl<'a, F: FrameResolver, S: SampleResolver> Builder<'a, F, S> {
    /// Construct a new builder. Defaults the demangler to
    /// [`moonbit_demangle::demangle`].
    pub fn new(profile: &'a FirefoxProfile, frames: F, samples: S) -> Self {
        Self {
            profile,
            frames,
            samples,
            demangle: Box::new(|s| moonbit_demangle::demangle(s)),
            mapping_filename: None,
        }
    }

    /// Override the symbol demangler. Use this when profiling non-MoonBit
    /// code (or to disable demangling with `|s| s.to_string()`).
    pub fn demangle_with(mut self, f: impl Fn(&str) -> String + 'static) -> Self {
        self.demangle = Box::new(f);
        self
    }

    /// Override the pprof Mapping's filename. Defaults to `libs[0].name`.
    pub fn mapping_filename(mut self, name: impl Into<String>) -> Self {
        self.mapping_filename = Some(name.into());
        self
    }

    /// Encode the profile to gzip'd protobuf bytes.
    pub fn encode(self) -> Result<Vec<u8>> {
        let proto = self.build();
        let mut buf = Vec::new();
        proto.encode(&mut buf)?;
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&buf)?;
        Ok(gz.finish()?)
    }

    /// Encode and write to `path`.
    pub fn write(self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let bytes = self.encode()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn build(self) -> proto::Profile {
        let Self {
            profile,
            frames,
            samples,
            demangle,
            mapping_filename,
        } = self;
        let mut state = State::new(demangle);
        let mapping_name = mapping_filename
            .or_else(|| profile.libs.first().map(|l| l.name.clone()))
            .unwrap_or_else(|| "unknown".into());
        let mapping_filename_id = state.intern(&mapping_name);

        let interval_ms = if profile.meta.interval > 0.0 {
            profile.meta.interval
        } else {
            1.0
        };
        for (ti, thread) in profile.threads.iter().enumerate() {
            // (count, ns) per stack-id.
            let mut aggregate: HashMap<i64, (i64, i64)> = HashMap::new();
            let mut stack_cache: HashMap<i64, Vec<i64>> = HashMap::new();
            for i in 0..thread.samples.length {
                let rs = samples.resolve(profile, thread, i);
                let Some(stk) = rs.stack else { continue };
                let entry = aggregate.entry(stk).or_insert((0, 0));
                entry.0 += rs.count;
                entry.1 += rs.ns;
            }
            for (stk, (count, ns)) in aggregate {
                let frame_ids = stack_frames(thread, stk, &mut stack_cache);
                let location_id: Vec<u64> = frame_ids
                    .iter()
                    .map(|&fi| state.intern_location(profile, thread, ti, fi, &frames))
                    .collect();
                state.samples.push(proto::Sample {
                    location_id,
                    value: vec![count, ns],
                    label: vec![],
                });
                state.total_ns += ns;
            }
            let _ = interval_ms; // keep available for future per-thread tweaks
        }

        state.finish(profile, mapping_filename_id)
    }
}

struct State {
    strings: Vec<String>,
    string_index: HashMap<String, i64>,
    functions: Vec<proto::Function>,
    func_index: HashMap<String, u64>,
    locations: Vec<proto::Location>,
    loc_canonical: HashMap<String, u64>,
    frame_to_loc: HashMap<(usize, i64), u64>,
    samples: Vec<proto::Sample>,
    total_ns: i64,
    demangle: DemangleFn,
}

impl State {
    fn new(demangle: DemangleFn) -> Self {
        let mut me = Self {
            strings: vec![String::new()],
            string_index: HashMap::from([(String::new(), 0)]),
            functions: Vec::new(),
            func_index: HashMap::new(),
            locations: Vec::new(),
            loc_canonical: HashMap::new(),
            frame_to_loc: HashMap::new(),
            samples: Vec::new(),
            total_ns: 0,
            demangle,
        };
        // Pre-intern the strings the final ValueType slots want, so the
        // builder can refer to them by id.
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
        let pretty = (self.demangle)(raw_name);
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

    fn intern_location<F: FrameResolver>(
        &mut self,
        profile: &FirefoxProfile,
        thread: &Thread,
        thread_idx: usize,
        frame_idx: i64,
        resolver: &F,
    ) -> u64 {
        if let Some(&id) = self.frame_to_loc.get(&(thread_idx, frame_idx)) {
            return id;
        }
        let frames = resolver.resolve(profile, thread, frame_idx);
        // Canonicalise so two distinct frame ids with identical resolved
        // content fold into a single pprof Location.
        let canonical = frames
            .iter()
            .map(|f| {
                format!(
                    "{}\x1f{}\x1f{}\x1f{}",
                    f.mapping_index, f.address, f.name, f.line
                )
            })
            .collect::<Vec<_>>()
            .join("\x1e");
        if let Some(&id) = self.loc_canonical.get(&canonical) {
            self.frame_to_loc.insert((thread_idx, frame_idx), id);
            return id;
        }

        let head_address = frames.first().map(|f| f.address as u64).unwrap_or(0);
        let head_mapping = frames.first().map(|f| f.mapping_index as u64 + 1).unwrap_or(1);
        let lines: Vec<proto::Line> = frames
            .iter()
            .map(|fr| {
                let func_id = self.intern_function(&fr.name, &fr.file);
                proto::Line {
                    function_id: func_id,
                    line: fr.line,
                    column: 0,
                }
            })
            .collect();
        let id = (self.locations.len() + 1) as u64;
        self.locations.push(proto::Location {
            id,
            mapping_id: head_mapping,
            address: head_address,
            line: lines,
            is_folded: false,
        });
        self.loc_canonical.insert(canonical, id);
        self.frame_to_loc.insert((thread_idx, frame_idx), id);
        id
    }

    fn finish(self, profile: &FirefoxProfile, mapping_filename: i64) -> proto::Profile {
        let interval_ns = (profile.meta.interval.max(1.0) * 1_000_000.0).round() as i64;
        let samples_str = self.string_index["samples"];
        let count_str = self.string_index["count"];
        let cpu_str = self.string_index["cpu"];
        let ns_str = self.string_index["nanoseconds"];
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

fn stack_frames(
    thread: &Thread,
    stack_id: i64,
    cache: &mut HashMap<i64, Vec<i64>>,
) -> Vec<i64> {
    if let Some(c) = cache.get(&stack_id) {
        return c.clone();
    }
    let mut acc = Vec::new();
    let mut s = Some(stack_id);
    while let Some(idx) = s {
        let i = idx as usize;
        acc.push(thread.stack_table.frame[i]);
        s = thread.stack_table.prefix.get(i).and_then(|p| *p);
    }
    cache.insert(stack_id, acc.clone());
    acc
}
