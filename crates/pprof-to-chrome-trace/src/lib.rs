//! Convert pprof CPU profiles into synthetic Chrome trace-event JSON
//! with V8 `Profile` / `ProfileChunk` events.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;

use anyhow::{bail, Context as _, Result};
use firefox_to_pprof::proto;
use serde::Serialize;

/// Conversion options for [`convert_profile`].
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// pprof sample value index to use as elapsed time. If omitted,
    /// the first CPU/wall time axis is selected.
    pub value_index: Option<usize>,
    /// pprof sample value index to use as sample count when
    /// [`expand_samples`](Self::expand_samples) is true.
    pub count_index: Option<usize>,
    /// Expand `samples/count` into repeated V8 samples. This makes
    /// `pprof -> trace -> pprof` preserve sample counts, at the cost of
    /// larger JSON.
    pub expand_samples: bool,
    /// Synthetic process id in the Chrome trace.
    pub pid: i64,
    /// Synthetic thread id in the Chrome trace.
    pub tid: i64,
    /// Synthetic V8 profile id.
    pub profile_id: String,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            value_index: None,
            count_index: None,
            expand_samples: false,
            pid: 1,
            tid: 1,
            profile_id: "0x1".to_string(),
        }
    }
}

/// Conversion stats returned with the trace JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Number of V8 profile nodes emitted.
    pub nodes: usize,
    /// Number of V8 samples emitted.
    pub samples: usize,
    /// Sum of emitted `timeDeltas`, in microseconds.
    pub total_delta_us: i64,
}

/// Output of [`convert_profile`].
pub struct EncodedTrace {
    /// Chrome trace-event JSON.
    pub json: String,
    /// Conversion stats.
    pub stats: Stats,
}

/// Convert a decoded pprof [`proto::Profile`] into Chrome trace JSON.
pub fn convert_profile(profile: &proto::Profile, opts: &ConvertOptions) -> Result<EncodedTrace> {
    let value_axis = select_value_axis(profile, opts.value_index)?;
    let count_index = opts.count_index.or_else(|| find_count_axis(profile));
    let locs = location_map(profile);
    let funcs = function_map(profile);

    let start_us = if profile.time_nanos > 0 {
        profile.time_nanos / 1000
    } else {
        0
    };

    let mut builder = TreeBuilder::new();
    let mut sample_node_ids = Vec::new();
    let mut time_deltas = Vec::new();

    for sample in &profile.sample {
        let raw = sample.value.get(value_axis.idx).copied().unwrap_or(0);
        if raw <= 0 {
            continue;
        }
        let delta_us = value_to_us(raw, value_axis.unit);
        let mut frames = Vec::new();
        for &loc_id in sample.location_id.iter().rev() {
            frames.push(frame_for_location(profile, &locs, &funcs, loc_id));
        }
        if frames.is_empty() {
            continue;
        }
        let leaf = builder.intern_stack(&frames);
        if opts.expand_samples {
            let count = count_index
                .and_then(|i| sample.value.get(i).copied())
                .unwrap_or(1)
                .max(1);
            push_expanded_samples(
                leaf,
                delta_us,
                count,
                &mut sample_node_ids,
                &mut time_deltas,
            )?;
        } else {
            sample_node_ids.push(leaf);
            time_deltas.push(delta_us);
        }
    }

    if sample_node_ids.is_empty() {
        bail!("pprof profile had no positive samples on selected value axis");
    }

    let total_delta_us = time_deltas.iter().sum::<i64>();
    let end_us = if profile.duration_nanos > 0 {
        start_us.saturating_add(profile.duration_nanos / 1000)
    } else {
        start_us.saturating_add(total_delta_us)
    };
    let nodes = builder.finish();
    let stats = Stats {
        nodes: nodes.len(),
        samples: sample_node_ids.len(),
        total_delta_us,
    };

    let trace = Trace {
        trace_events: vec![
            TraceEvent::metadata(opts.pid, opts.tid, start_us, "process_name", "pprof"),
            TraceEvent::metadata(opts.pid, opts.tid, start_us, "thread_name", "pprof"),
            TraceEvent::profile(opts, start_us),
            TraceEvent::profile_chunk(
                opts,
                start_us,
                CpuProfile {
                    nodes,
                    samples: sample_node_ids,
                    time_deltas,
                    start_time: start_us,
                    end_time: end_us,
                },
            ),
        ],
    };
    Ok(EncodedTrace {
        json: serde_json::to_string_pretty(&trace).context("encoding Chrome trace JSON")?,
        stats,
    })
}

struct ValueAxis<'a> {
    idx: usize,
    unit: &'a str,
}

fn select_value_axis(profile: &proto::Profile, explicit: Option<usize>) -> Result<ValueAxis<'_>> {
    if let Some(idx) = explicit {
        let Some(st) = profile.sample_type.get(idx) else {
            bail!(
                "value index {} out of range; profile has {} sample type(s)",
                idx,
                profile.sample_type.len()
            );
        };
        return Ok(ValueAxis {
            idx,
            unit: string_at(profile, st.unit),
        });
    }

    for (i, st) in profile.sample_type.iter().enumerate() {
        let ty = string_at(profile, st.r#type);
        let unit = string_at(profile, st.unit);
        if (ty == "cpu" || ty == "wall") && is_time_unit(unit) {
            return Ok(ValueAxis { idx: i, unit });
        }
    }
    bail!("no CPU/wall time sample_type found; pass --value-index to choose one")
}

fn find_count_axis(profile: &proto::Profile) -> Option<usize> {
    profile.sample_type.iter().enumerate().find_map(|(i, st)| {
        let ty = string_at(profile, st.r#type);
        let unit = string_at(profile, st.unit);
        if ty == "samples" && unit == "count" {
            Some(i)
        } else {
            None
        }
    })
}

fn is_time_unit(unit: &str) -> bool {
    matches!(
        unit,
        "nanoseconds" | "microseconds" | "milliseconds" | "seconds"
    )
}

fn value_to_us(raw: i64, unit: &str) -> i64 {
    let us = match unit {
        "nanoseconds" => (raw + 500) / 1000,
        "microseconds" => raw,
        "milliseconds" => raw.saturating_mul(1000),
        "seconds" => raw.saturating_mul(1_000_000),
        _ => raw,
    };
    if raw > 0 {
        us.max(1)
    } else {
        0
    }
}

fn push_expanded_samples(
    leaf: i64,
    delta_us: i64,
    count: i64,
    sample_node_ids: &mut Vec<i64>,
    time_deltas: &mut Vec<i64>,
) -> Result<()> {
    let count_usize: usize = count
        .try_into()
        .context("sample count cannot fit in memory on this platform")?;
    if count_usize > 10_000_000 {
        bail!("--expand-samples would emit more than 10,000,000 samples; omit it for compact JSON");
    }
    let base = delta_us / count;
    let remainder = delta_us % count;
    for i in 0..count {
        sample_node_ids.push(leaf);
        time_deltas.push(base + if i < remainder { 1 } else { 0 });
    }
    Ok(())
}

fn string_at(profile: &proto::Profile, idx: i64) -> &str {
    if idx < 0 {
        return "";
    }
    profile
        .string_table
        .get(idx as usize)
        .map(String::as_str)
        .unwrap_or("")
}

fn location_map(profile: &proto::Profile) -> HashMap<u64, &proto::Location> {
    profile.location.iter().map(|l| (l.id, l)).collect()
}

fn function_map(profile: &proto::Profile) -> HashMap<u64, &proto::Function> {
    profile.function.iter().map(|f| (f.id, f)).collect()
}

fn frame_for_location(
    profile: &proto::Profile,
    locs: &HashMap<u64, &proto::Location>,
    funcs: &HashMap<u64, &proto::Function>,
    loc_id: u64,
) -> CallFrame {
    let Some(loc) = locs.get(&loc_id).copied() else {
        return CallFrame::unknown(format!("location:{loc_id}"));
    };
    let Some(line) = loc.line.first() else {
        return CallFrame::unknown(format!("0x{:x}", loc.address));
    };
    let Some(func) = funcs.get(&line.function_id).copied() else {
        return CallFrame::unknown(format!("function:{}", line.function_id));
    };
    let name = string_at(profile, func.name);
    let filename = string_at(profile, func.filename);
    let line_number = if line.line > 0 {
        line.line - 1
    } else if func.start_line > 0 {
        func.start_line - 1
    } else {
        -1
    };
    CallFrame {
        function_name: if name.is_empty() { "(unknown)" } else { name }.to_string(),
        script_id: "0".to_string(),
        url: filename.to_string(),
        line_number,
        column_number: line.column.max(0),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
struct CallFrame {
    function_name: String,
    script_id: String,
    url: String,
    line_number: i64,
    column_number: i64,
}

impl CallFrame {
    fn unknown(name: String) -> Self {
        Self {
            function_name: name,
            script_id: "0".to_string(),
            url: String::new(),
            line_number: -1,
            column_number: -1,
        }
    }
}

struct TreeBuilder {
    nodes: Vec<Node>,
    child_by_key: HashMap<(i64, CallFrame), i64>,
}

impl TreeBuilder {
    fn new() -> Self {
        Self {
            nodes: vec![Node {
                id: 1,
                call_frame: CallFrame {
                    function_name: "(root)".to_string(),
                    script_id: "0".to_string(),
                    url: String::new(),
                    line_number: -1,
                    column_number: -1,
                },
                children: Vec::new(),
            }],
            child_by_key: HashMap::new(),
        }
    }

    fn intern_stack(&mut self, frames: &[CallFrame]) -> i64 {
        let mut parent = 1;
        for frame in frames {
            let key = (parent, frame.clone());
            let id = if let Some(&id) = self.child_by_key.get(&key) {
                id
            } else {
                let id = self.nodes.len() as i64 + 1;
                self.nodes.push(Node {
                    id,
                    call_frame: frame.clone(),
                    children: Vec::new(),
                });
                self.nodes[parent as usize - 1].children.push(id);
                self.child_by_key.insert(key, id);
                id
            };
            parent = id;
        }
        parent
    }

    fn finish(self) -> Vec<Node> {
        self.nodes
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Trace {
    trace_events: Vec<TraceEvent>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceEvent {
    pid: i64,
    tid: i64,
    ts: i64,
    ph: &'static str,
    cat: &'static str,
    name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    args: serde_json::Value,
}

impl TraceEvent {
    fn metadata(pid: i64, tid: i64, ts: i64, name: &'static str, value: &'static str) -> Self {
        Self {
            pid,
            tid,
            ts,
            ph: "M",
            cat: "__metadata",
            name,
            id: None,
            args: serde_json::json!({ "name": value }),
        }
    }

    fn profile(opts: &ConvertOptions, ts: i64) -> Self {
        Self {
            pid: opts.pid,
            tid: opts.tid,
            ts,
            ph: "P",
            cat: "disabled-by-default-v8.cpu_profiler",
            name: "Profile",
            id: Some(opts.profile_id.clone()),
            args: serde_json::json!({ "data": { "startTime": ts } }),
        }
    }

    fn profile_chunk(opts: &ConvertOptions, ts: i64, profile: CpuProfile) -> Self {
        Self {
            pid: opts.pid,
            tid: opts.tid,
            ts,
            ph: "P",
            cat: "disabled-by-default-v8.cpu_profiler",
            name: "ProfileChunk",
            id: Some(opts.profile_id.clone()),
            args: serde_json::json!({ "data": {
                "cpuProfile": {
                    "nodes": profile.nodes,
                    "startTime": profile.start_time,
                    "endTime": profile.end_time,
                },
                "samples": profile.samples,
                "timeDeltas": profile.time_deltas,
            }}),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CpuProfile {
    nodes: Vec<Node>,
    samples: Vec<i64>,
    time_deltas: Vec<i64>,
    start_time: i64,
    end_time: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    id: i64,
    call_frame: CallFrame,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use prost::Message as _;
    use std::io::Read as _;

    #[test]
    fn emits_chrome_profile_chunk_json() {
        let out = convert_profile(&synthetic_pprof(), &ConvertOptions::default()).unwrap();
        assert_eq!(
            out.stats,
            Stats {
                nodes: 4,
                samples: 2,
                total_delta_us: 3_500,
            }
        );

        let v: serde_json::Value = serde_json::from_str(&out.json).unwrap();
        let events = v["traceEvents"].as_array().unwrap();
        assert!(events.iter().any(|e| e["name"] == "Profile"));
        let chunk = events.iter().find(|e| e["name"] == "ProfileChunk").unwrap();
        assert_eq!(
            chunk["args"]["data"]["samples"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            chunk["args"]["data"]["timeDeltas"],
            serde_json::json!([3000, 500])
        );

        let nodes = chunk["args"]["data"]["cpuProfile"]["nodes"]
            .as_array()
            .unwrap();
        let names: Vec<&str> = nodes
            .iter()
            .map(|n| n["callFrame"]["functionName"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"(root)"));
        assert!(names.contains(&"root"));
        assert!(names.contains(&"hot"));
        assert!(names.contains(&"leaf"));
    }

    #[test]
    fn expand_samples_preserves_count_on_roundtrip() {
        let out = convert_profile(
            &synthetic_pprof(),
            &ConvertOptions {
                expand_samples: true,
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        assert_eq!(out.stats.samples, 4);
        assert_eq!(out.stats.total_delta_us, 3_500);

        let roundtrip = chrome_trace_to_pprof::convert(
            &out.json,
            &chrome_trace_to_pprof::ConvertOptions::default(),
        )
        .unwrap();
        let profile = decode_pprof(&roundtrip.encoded);
        let total_count: i64 = profile.sample.iter().map(|s| s.value[0]).sum();
        let total_ns: i64 = profile.sample.iter().map(|s| s.value[1]).sum();
        assert_eq!(total_count, 4);
        assert_eq!(total_ns, 3_500_000);
    }

    fn synthetic_pprof() -> proto::Profile {
        let strings = vec![
            "",
            "samples",
            "count",
            "cpu",
            "nanoseconds",
            "root",
            "hot",
            "leaf",
            "file:///main.js",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        proto::Profile {
            sample_type: vec![
                proto::ValueType { r#type: 1, unit: 2 },
                proto::ValueType { r#type: 3, unit: 4 },
            ],
            sample: vec![
                proto::Sample {
                    location_id: vec![3, 2, 1],
                    value: vec![3, 3_000_000],
                    label: vec![],
                },
                proto::Sample {
                    location_id: vec![2, 1],
                    value: vec![1, 500_000],
                    label: vec![],
                },
            ],
            mapping: vec![],
            location: vec![
                proto::Location {
                    id: 1,
                    mapping_id: 0,
                    address: 0,
                    line: vec![proto::Line {
                        function_id: 1,
                        line: 1,
                        column: 0,
                    }],
                    is_folded: false,
                },
                proto::Location {
                    id: 2,
                    mapping_id: 0,
                    address: 0,
                    line: vec![proto::Line {
                        function_id: 2,
                        line: 5,
                        column: 0,
                    }],
                    is_folded: false,
                },
                proto::Location {
                    id: 3,
                    mapping_id: 0,
                    address: 0,
                    line: vec![proto::Line {
                        function_id: 3,
                        line: 9,
                        column: 0,
                    }],
                    is_folded: false,
                },
            ],
            function: vec![
                proto::Function {
                    id: 1,
                    name: 5,
                    system_name: 5,
                    filename: 8,
                    start_line: 1,
                },
                proto::Function {
                    id: 2,
                    name: 6,
                    system_name: 6,
                    filename: 8,
                    start_line: 5,
                },
                proto::Function {
                    id: 3,
                    name: 7,
                    system_name: 7,
                    filename: 8,
                    start_line: 9,
                },
            ],
            string_table: strings,
            drop_frames: 0,
            keep_frames: 0,
            time_nanos: 1_000_000_000,
            duration_nanos: 3_500_000,
            period_type: None,
            period: 0,
            comment: vec![],
            default_sample_type: 0,
            doc_url: 0,
        }
    }

    fn decode_pprof(bytes: &[u8]) -> proto::Profile {
        let mut decoder = GzDecoder::new(bytes);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        proto::Profile::decode(raw.as_slice()).unwrap()
    }
}
