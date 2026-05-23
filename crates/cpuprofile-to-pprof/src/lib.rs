//! Convert Node V8 `.cpuprofile` (Inspector `Profiler.Profile`) JSON into
//! gzip'd pprof protobuf.
//!
//! Port of the TypeScript implementation that lived in
//! `@mizchi/pprof-tools/cpuprofile-to-pprof`. The cpuprofile schema is a
//! tree of nodes (`nodes[i].children` lists child ids) plus a `samples`
//! array of leaf node ids and a `timeDeltas` array (μs) holding the
//! elapsed time between consecutive samples. We invert children into a
//! parent map, aggregate samples by leaf node, and emit one pprof Sample
//! per unique leaf with both `samples/count` and `cpu/nanoseconds`
//! values.
//!
//! Symbol names go through a caller-supplied demangler. Defaults to
//! [`moonbit_demangle::demangle`]; pass identity (`|s| s.into()`) when
//! profiling non-MoonBit code.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::io::Write;

use anyhow::Result;
use flate2::Compression;
use flate2::write::GzEncoder;
use prost::Message;
use serde::Deserialize;

use firefox_to_pprof::proto;

/// Parsed shape of `Profiler.Profile` (`.cpuprofile` JSON).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProfile {
    /// All profile nodes. Each node carries its `id`, the `callFrame`,
    /// and the ids of its children.
    pub nodes: Vec<CpuNode>,
    /// One node id per sample (the leaf node hit at sampling time).
    pub samples: Vec<i64>,
    /// Microseconds between consecutive samples (parallel to `samples`).
    #[serde(default)]
    pub time_deltas: Vec<i64>,
    /// Profile start time in microseconds since epoch (V8 monotonic).
    #[serde(default)]
    pub start_time: i64,
    /// Profile end time in microseconds since epoch.
    #[serde(default)]
    pub end_time: i64,
}

/// One node in the cpuprofile call tree.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuNode {
    /// V8 node id (referenced from `CpuProfile::samples`).
    pub id: i64,
    /// Function + source location for this node.
    pub call_frame: CallFrame,
    /// Ids of child nodes (one level deeper in the call tree).
    #[serde(default)]
    pub children: Vec<i64>,
}

/// V8 call frame (`CallFrame` in the Inspector domain).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFrame {
    /// Function name as V8 reports it. May be empty for anonymous fns.
    #[serde(default)]
    pub function_name: String,
    /// Script URL (file:// or http:// or empty for native frames).
    #[serde(default)]
    pub url: String,
    /// V8 script id (string in newer versions, number in older). We
    /// only use it as a dedup key so storing the raw JSON form is
    /// sufficient.
    #[serde(default)]
    pub script_id: serde_json::Value,
    /// Zero-based line number, or -1 if unknown.
    #[serde(default = "neg_one")]
    pub line_number: i32,
    /// Zero-based column number, or -1 if unknown.
    #[serde(default = "neg_one")]
    pub column_number: i32,
}

fn neg_one() -> i32 {
    -1
}

/// Demangler hook — `(name) → pretty name`. Defaults to
/// [`moonbit_demangle::demangle`].
pub type DemangleFn = Box<dyn Fn(&str) -> String>;

/// Conversion stats returned alongside the encoded bytes.
#[derive(Debug, Clone, Copy)]
pub struct Stats {
    /// Number of pprof Sample entries emitted (= unique leaf nodes hit).
    pub samples: usize,
    /// Number of unique pprof Function entries.
    pub functions: usize,
    /// Number of unique pprof Location entries.
    pub locations: usize,
}

/// Output of [`Builder::encode`] — gzip'd bytes plus counts.
pub struct EncodedProfile {
    /// gzip-compressed pprof protobuf, ready to be written to disk.
    pub encoded: Vec<u8>,
    /// Conversion stats (sample/function/location counts).
    pub stats: Stats,
}

/// Builds a pprof Profile from a parsed [`CpuProfile`].
///
/// ```no_run
/// use cpuprofile_to_pprof::{CpuProfile, Builder};
/// # fn doctest(json: &str) -> anyhow::Result<()> {
/// let profile: CpuProfile = serde_json::from_str(json)?;
/// let out = Builder::new(profile).encode()?;
/// std::fs::write("out.pb.gz", out.encoded)?;
/// # Ok(()) }
/// ```
pub struct Builder {
    profile: CpuProfile,
    demangle: DemangleFn,
    mapping_filename: Option<String>,
}

impl Builder {
    /// Construct a builder from a parsed cpuprofile. Defaults the
    /// demangler to [`moonbit_demangle::demangle`].
    pub fn new(profile: CpuProfile) -> Self {
        Self {
            profile,
            demangle: Box::new(|s| moonbit_demangle::demangle(s)),
            mapping_filename: None,
        }
    }

    /// Override the symbol demangler. Pass `|s| s.into()` to disable.
    pub fn demangle_with(mut self, f: impl Fn(&str) -> String + 'static) -> Self {
        self.demangle = Box::new(f);
        self
    }

    /// Override the pprof Mapping's filename. Defaults to empty.
    pub fn mapping_filename(mut self, s: impl Into<String>) -> Self {
        self.mapping_filename = Some(s.into());
        self
    }

    /// Encode the profile to gzip'd protobuf bytes.
    pub fn encode(self) -> Result<EncodedProfile> {
        let (profile, stats) = self.build();
        let mut buf = Vec::new();
        profile.encode(&mut buf)?;
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&buf)?;
        Ok(EncodedProfile {
            encoded: gz.finish()?,
            stats,
        })
    }

    fn build(self) -> (proto::Profile, Stats) {
        let Self {
            profile,
            demangle,
            mapping_filename,
        } = self;
        let mut state = State::new(demangle);
        let mapping_filename_id =
            state.intern(mapping_filename.as_deref().unwrap_or(""));

        // Index nodes by id and invert children -> parent.
        let mut node_by_id: HashMap<i64, &CpuNode> = HashMap::with_capacity(profile.nodes.len());
        for n in &profile.nodes {
            node_by_id.insert(n.id, n);
        }
        let mut parent: HashMap<i64, i64> = HashMap::new();
        for n in &profile.nodes {
            for &c in &n.children {
                parent.insert(c, n.id);
            }
        }

        // Aggregate count + microseconds per leaf node id.
        let mut count_by_node: HashMap<i64, i64> = HashMap::new();
        let mut us_by_node: HashMap<i64, i64> = HashMap::new();
        let mut total_us: i64 = 0;
        for (i, &nid) in profile.samples.iter().enumerate() {
            let dt = profile.time_deltas.get(i).copied().unwrap_or(0);
            *count_by_node.entry(nid).or_default() += 1;
            *us_by_node.entry(nid).or_default() += dt;
            total_us += dt;
        }

        // Emit one Sample per unique leaf node.
        let mut samples_emitted = 0usize;
        let mut stack_cache: HashMap<i64, Vec<u64>> = HashMap::new();
        for (&nid, &count) in &count_by_node {
            let us = us_by_node.get(&nid).copied().unwrap_or(0);
            let location_id = stack_for_node(
                nid,
                &node_by_id,
                &parent,
                &mut state,
                &mut stack_cache,
            );
            state.samples.push(proto::Sample {
                location_id,
                value: vec![count, us * 1000],
                label: vec![],
            });
            samples_emitted += 1;
        }

        let stats = Stats {
            samples: samples_emitted,
            functions: state.functions.len(),
            locations: state.locations.len(),
        };

        let period_ns = if profile.samples.is_empty() {
            1
        } else {
            let avg_us = (total_us as f64 / profile.samples.len() as f64).round() as i64;
            (avg_us * 1000).max(1)
        };

        let time_nanos = profile.start_time.saturating_mul(1000);
        let duration_nanos = profile
            .end_time
            .saturating_sub(profile.start_time)
            .saturating_mul(1000);

        (
            state.finish(mapping_filename_id, period_ns, time_nanos, duration_nanos),
            stats,
        )
    }
}

fn stack_for_node(
    leaf: i64,
    by_id: &HashMap<i64, &CpuNode>,
    parent: &HashMap<i64, i64>,
    state: &mut State,
    cache: &mut HashMap<i64, Vec<u64>>,
) -> Vec<u64> {
    if let Some(cached) = cache.get(&leaf) {
        return cached.clone();
    }
    let mut stack: Vec<u64> = Vec::new();
    let mut cur = Some(leaf);
    while let Some(nid) = cur {
        let Some(node) = by_id.get(&nid) else { break };
        stack.push(state.intern_location(node));
        cur = parent.get(&nid).copied();
    }
    cache.insert(leaf, stack.clone());
    stack
}

struct State {
    strings: Vec<String>,
    string_index: HashMap<String, i64>,
    functions: Vec<proto::Function>,
    func_index: HashMap<String, u64>,
    locations: Vec<proto::Location>,
    loc_by_node: HashMap<i64, u64>,
    samples: Vec<proto::Sample>,
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
            loc_by_node: HashMap::new(),
            samples: Vec::new(),
            demangle,
        };
        // Pre-intern fixed strings the final ValueType slots need.
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

    fn intern_function(&mut self, call: &CallFrame) -> u64 {
        let raw = if call.function_name.is_empty() {
            "(anonymous)"
        } else {
            call.function_name.as_str()
        };
        let key = format!(
            "{raw}\x1f{url}\x1f{sid}",
            url = call.url,
            sid = call.script_id,
        );
        if let Some(&id) = self.func_index.get(&key) {
            return id;
        }
        let pretty = (self.demangle)(raw);
        let id = (self.functions.len() + 1) as u64;
        let name = self.intern(&pretty);
        let system_name = self.intern(raw);
        let filename = self.intern(&call.url);
        let start_line = if call.line_number >= 0 {
            call.line_number as i64 + 1
        } else {
            0
        };
        self.functions.push(proto::Function {
            id,
            name,
            system_name,
            filename,
            start_line,
        });
        self.func_index.insert(key, id);
        id
    }

    fn intern_location(&mut self, node: &CpuNode) -> u64 {
        if let Some(&id) = self.loc_by_node.get(&node.id) {
            return id;
        }
        let func_id = self.intern_function(&node.call_frame);
        let line = if node.call_frame.line_number >= 0 {
            node.call_frame.line_number as i64 + 1
        } else {
            0
        };
        let id = (self.locations.len() + 1) as u64;
        self.locations.push(proto::Location {
            id,
            mapping_id: 1,
            address: 0,
            line: vec![proto::Line {
                function_id: func_id,
                line,
                column: 0,
            }],
            is_folded: false,
        });
        self.loc_by_node.insert(node.id, id);
        id
    }

    fn finish(
        self,
        mapping_filename: i64,
        period_ns: i64,
        time_nanos: i64,
        duration_nanos: i64,
    ) -> proto::Profile {
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
            time_nanos,
            duration_nanos,
            period_type: Some(proto::ValueType {
                r#type: cpu_str,
                unit: ns_str,
            }),
            period: period_ns,
            comment: vec![],
            default_sample_type: 0,
            doc_url: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_profile() -> CpuProfile {
        // Tiny three-node tree: root -> child_a -> leaf
        CpuProfile {
            nodes: vec![
                CpuNode {
                    id: 1,
                    call_frame: CallFrame {
                        function_name: "(root)".into(),
                        url: String::new(),
                        script_id: serde_json::Value::String("0".into()),
                        line_number: -1,
                        column_number: -1,
                    },
                    children: vec![2],
                },
                CpuNode {
                    id: 2,
                    call_frame: CallFrame {
                        function_name: "_M0FP26mizchi5bench9ackermann".into(),
                        url: "wasm".into(),
                        script_id: serde_json::Value::String("42".into()),
                        line_number: 0,
                        column_number: 0,
                    },
                    children: vec![3],
                },
                CpuNode {
                    id: 3,
                    call_frame: CallFrame {
                        function_name: "_M0FP26mizchi5bench3fib".into(),
                        url: "wasm".into(),
                        script_id: serde_json::Value::String("42".into()),
                        line_number: 1,
                        column_number: 0,
                    },
                    children: vec![],
                },
            ],
            samples: vec![3, 3, 2],
            time_deltas: vec![1000, 1000, 500],
            start_time: 0,
            end_time: 2500,
        }
    }

    #[test]
    fn builds_and_demangles() {
        let out = Builder::new(synth_profile()).encode().unwrap();
        assert_eq!(out.stats.samples, 2); // 2 unique leaves
        assert_eq!(out.stats.locations, 3); // root + ackermann + fib
        assert_eq!(out.stats.functions, 3);
        assert!(!out.encoded.is_empty());
    }

    #[test]
    fn identity_demangler_passes_raw_names() {
        let out = Builder::new(synth_profile())
            .demangle_with(|s| s.to_string())
            .encode()
            .unwrap();
        // gzipped bytes — just verify the encoder ran with the override.
        assert!(out.encoded.len() > 50);
    }
}
