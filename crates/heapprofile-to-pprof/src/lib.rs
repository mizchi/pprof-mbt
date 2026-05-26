//! Convert Node V8 `.heapprofile` (Inspector `HeapProfiler.SamplingHeapProfile`)
//! JSON into gzip'd pprof protobuf.
//!
//! V8's sampling allocation profiler hands back a tree of nodes:
//! every node carries a `callFrame`, a `selfSize` (bytes estimated to
//! have been allocated at that frame), an `id`, and a `children` list.
//! Optionally there is a flat `samples` array of `{size, nodeId,
//! ordinal}` rows; when present we use it to derive per-node allocation
//! counts, otherwise we fall back to "1 allocation per node with
//! selfSize > 0".
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

/// Parsed shape of `HeapProfiler.SamplingHeapProfile` (`.heapprofile` JSON).
#[derive(Debug, Deserialize)]
pub struct HeapProfile {
    /// Root of the call tree. `selfSize` on the root itself is usually 0.
    pub head: HeapNode,
    /// Optional flat allocation sample list. When present each entry
    /// references the leaf node by id and carries the sampled size.
    #[serde(default)]
    pub samples: Vec<HeapSample>,
}

/// One node in the sampling heap profile call tree.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapNode {
    /// Function + source location for this frame.
    pub call_frame: CallFrame,
    /// Bytes attributed to allocations whose top frame is this node.
    #[serde(default)]
    pub self_size: i64,
    /// V8 node id (referenced from [`HeapProfile::samples`] when present).
    pub id: i64,
    /// Nested call-tree children.
    #[serde(default)]
    pub children: Vec<HeapNode>,
}

/// One allocation sample as recorded by V8.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapSample {
    /// Bytes sampled for this allocation.
    pub size: i64,
    /// Id of the [`HeapNode`] this sample was attributed to.
    pub node_id: i64,
    /// Monotonic ordinal V8 uses to order samples; we don't use it but
    /// accept it so the schema parses cleanly.
    #[serde(default)]
    pub ordinal: i64,
}

/// V8 call frame (`Runtime.CallFrame`, identical to the cpuprofile shape).
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
    /// Number of pprof Sample entries emitted (= nodes with selfSize > 0).
    pub samples: usize,
    /// Number of unique pprof Function entries.
    pub functions: usize,
    /// Number of unique pprof Location entries.
    pub locations: usize,
    /// Total bytes attributed across every emitted sample.
    pub total_bytes: i64,
    /// Total allocation count across every emitted sample.
    pub total_objects: i64,
}

/// Output of [`Builder::encode`] — gzip'd bytes plus counts.
pub struct EncodedProfile {
    /// gzip-compressed pprof protobuf, ready to be written to disk.
    pub encoded: Vec<u8>,
    /// Conversion stats.
    pub stats: Stats,
}

/// Builds a pprof Profile from a parsed [`HeapProfile`].
///
/// ```no_run
/// use heapprofile_to_pprof::{HeapProfile, Builder};
/// # fn doctest(json: &str) -> anyhow::Result<()> {
/// let profile: HeapProfile = serde_json::from_str(json)?;
/// let out = Builder::new(profile).encode()?;
/// std::fs::write("out.pb.gz", out.encoded)?;
/// # Ok(()) }
/// ```
pub struct Builder {
    profile: HeapProfile,
    demangle: DemangleFn,
    mapping_filename: Option<String>,
}

impl Builder {
    /// Construct a builder from a parsed heapprofile. Defaults the
    /// demangler to [`moonbit_demangle::demangle`].
    pub fn new(profile: HeapProfile) -> Self {
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

        // Per-node object counts derived from the flat samples list when
        // V8 included it. If samples is empty, fall back to "1 alloc per
        // node with selfSize > 0" so the alloc_objects column still
        // ranks identically to alloc_space.
        let mut count_by_node: HashMap<i64, i64> = HashMap::new();
        for s in &profile.samples {
            *count_by_node.entry(s.node_id).or_default() += 1;
        }
        let have_samples = !profile.samples.is_empty();

        // DFS the tree maintaining the current call-path location ids
        // (root-most last, mirroring the cpuprofile builder so the same
        // pprof tooling renders both identically).
        let mut samples_out: Vec<proto::Sample> = Vec::new();
        let mut total_bytes: i64 = 0;
        let mut total_objects: i64 = 0;
        let mut stack: Vec<u64> = Vec::new();
        walk(
            &profile.head,
            &mut stack,
            &mut state,
            &count_by_node,
            have_samples,
            &mut samples_out,
            &mut total_bytes,
            &mut total_objects,
        );

        let stats = Stats {
            samples: samples_out.len(),
            functions: state.functions.len(),
            locations: state.locations.len(),
            total_bytes,
            total_objects,
        };

        state.samples = samples_out;
        (state.finish(mapping_filename_id), stats)
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    node: &HeapNode,
    stack: &mut Vec<u64>,
    state: &mut State,
    count_by_node: &HashMap<i64, i64>,
    have_samples: bool,
    out: &mut Vec<proto::Sample>,
    total_bytes: &mut i64,
    total_objects: &mut i64,
) {
    let loc_id = state.intern_location(node);
    // Stack is leaf-first in pprof (location_id[0] is the innermost
    // frame), so push *before* recursing into children means we'd reverse
    // the order. Instead push here and rely on the order below.
    stack.push(loc_id);

    if node.self_size > 0 {
        let count = if have_samples {
            count_by_node.get(&node.id).copied().unwrap_or(0)
        } else {
            1
        };
        // pprof Sample location_id: leaf first, root last. `stack`
        // currently holds root..=this node, so reverse for emission.
        let mut leaf_first = stack.clone();
        leaf_first.reverse();
        out.push(proto::Sample {
            location_id: leaf_first,
            value: vec![count, node.self_size],
            label: vec![],
        });
        *total_bytes += node.self_size;
        *total_objects += count;
    }

    for child in &node.children {
        walk(
            child,
            stack,
            state,
            count_by_node,
            have_samples,
            out,
            total_bytes,
            total_objects,
        );
    }
    stack.pop();
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
        // Pre-intern the ValueType slot strings.
        me.intern("alloc_objects");
        me.intern("count");
        me.intern("alloc_space");
        me.intern("bytes");
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

    fn intern_location(&mut self, node: &HeapNode) -> u64 {
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

    fn finish(self, mapping_filename: i64) -> proto::Profile {
        let alloc_objs = self.string_index["alloc_objects"];
        let count_str = self.string_index["count"];
        let alloc_space = self.string_index["alloc_space"];
        let bytes_str = self.string_index["bytes"];
        proto::Profile {
            sample_type: vec![
                proto::ValueType {
                    r#type: alloc_objs,
                    unit: count_str,
                },
                proto::ValueType {
                    r#type: alloc_space,
                    unit: bytes_str,
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
            time_nanos: 0,
            duration_nanos: 0,
            // V8's sampling heap profiler uses ~512 KiB intervals by
            // default; surface that as the pprof Period so downstream
            // tools (`go tool pprof -alloc_space`) interpret the values
            // consistently. We can't recover the actual interval from
            // the JSON, so report 1 to mean "values are exact".
            period_type: Some(proto::ValueType {
                r#type: alloc_space,
                unit: bytes_str,
            }),
            period: 1,
            comment: vec![],
            // Default to alloc_space so the pprof UI lands on the
            // bytes view (more useful than object count for most
            // investigations).
            default_sample_type: alloc_space,
            doc_url: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_profile() -> HeapProfile {
        // root -> ackermann (4 KB) -> fib (8 KB)
        HeapProfile {
            head: HeapNode {
                call_frame: CallFrame {
                    function_name: "(root)".into(),
                    url: String::new(),
                    script_id: serde_json::Value::String("0".into()),
                    line_number: -1,
                    column_number: -1,
                },
                self_size: 0,
                id: 1,
                children: vec![HeapNode {
                    call_frame: CallFrame {
                        function_name: "_M0FP26mizchi5bench9ackermann".into(),
                        url: "main.js".into(),
                        script_id: serde_json::Value::String("42".into()),
                        line_number: 9,
                        column_number: 0,
                    },
                    self_size: 4096,
                    id: 2,
                    children: vec![HeapNode {
                        call_frame: CallFrame {
                            function_name: "_M0FP26mizchi5bench3fib".into(),
                            url: "main.js".into(),
                            script_id: serde_json::Value::String("42".into()),
                            line_number: 19,
                            column_number: 0,
                        },
                        self_size: 8192,
                        id: 3,
                        children: vec![],
                    }],
                }],
            },
            samples: vec![
                HeapSample {
                    size: 4096,
                    node_id: 2,
                    ordinal: 0,
                },
                HeapSample {
                    size: 8192,
                    node_id: 3,
                    ordinal: 1,
                },
                HeapSample {
                    size: 8192,
                    node_id: 3,
                    ordinal: 2,
                },
            ],
        }
    }

    #[test]
    fn builds_and_demangles() {
        let out = Builder::new(synth_profile()).encode().unwrap();
        // 2 nodes have selfSize > 0 (ackermann + fib).
        assert_eq!(out.stats.samples, 2);
        // 3 unique locations / functions (root + ackermann + fib).
        assert_eq!(out.stats.locations, 3);
        assert_eq!(out.stats.functions, 3);
        // 4 KB + 8 KB attributed.
        assert_eq!(out.stats.total_bytes, 4096 + 8192);
        // ackermann has 1 sample, fib has 2.
        assert_eq!(out.stats.total_objects, 3);
        assert!(!out.encoded.is_empty());
    }

    #[test]
    fn falls_back_to_one_object_per_node_without_samples() {
        let mut p = synth_profile();
        p.samples.clear();
        let out = Builder::new(p).encode().unwrap();
        // Without per-sample data we credit each node with selfSize > 0
        // a single allocation; total_objects therefore matches sample
        // count.
        assert_eq!(out.stats.total_objects, 2);
        assert_eq!(out.stats.total_bytes, 4096 + 8192);
    }

    #[test]
    fn identity_demangler_passes_raw_names() {
        let out = Builder::new(synth_profile())
            .demangle_with(|s| s.to_string())
            .encode()
            .unwrap();
        assert!(out.encoded.len() > 50);
    }

    #[test]
    fn parses_v8_inspector_snake_case_field_names() {
        // V8 inspector uses camelCase keys (selfSize, callFrame, nodeId).
        let json = r#"{
            "head": {
                "callFrame": {"functionName":"(root)","url":"","scriptId":"0","lineNumber":-1,"columnNumber":-1},
                "selfSize": 0,
                "id": 1,
                "children": [{
                    "callFrame": {"functionName":"alloc","url":"a.js","scriptId":"1","lineNumber":0,"columnNumber":0},
                    "selfSize": 128,
                    "id": 2,
                    "children": []
                }]
            },
            "samples": [{"size":128,"nodeId":2,"ordinal":0}]
        }"#;
        let p: HeapProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.head.children.len(), 1);
        assert_eq!(p.head.children[0].self_size, 128);
        assert_eq!(p.samples.len(), 1);
        assert_eq!(p.samples[0].node_id, 2);
    }
}
