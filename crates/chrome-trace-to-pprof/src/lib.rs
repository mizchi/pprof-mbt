//! Convert Chrome trace-event JSON containing V8 CPU profiler
//! `Profile` / `ProfileChunk` events into gzip'd pprof.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};

use anyhow::{bail, Context as _, Result};
use cpuprofile_to_pprof::{Builder, CallFrame, CpuNode, CpuProfile};
use serde::Deserialize;
use serde_json::Value;

/// Conversion options for [`convert`].
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Which extracted CPU profile to convert when the trace contains
    /// more than one `(pid, tid, id)` profile stream.
    pub profile_index: usize,
    /// Disable MoonBit symbol demangling.
    pub no_demangle: bool,
    /// Override the pprof Mapping's filename.
    pub mapping_filename: Option<String>,
    /// Fallback interval in microseconds for traces that omit
    /// `timeDeltas`.
    pub default_sample_delta_us: i64,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            profile_index: 0,
            no_demangle: false,
            mapping_filename: None,
            default_sample_delta_us: 1000,
        }
    }
}

/// Metadata for a V8 CPU profile extracted from the trace stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedProfileInfo {
    /// Stable key derived from trace `pid`, `tid`, and `id` / `id2`.
    pub key: String,
    /// Number of V8 profile nodes.
    pub nodes: usize,
    /// Number of raw CPU samples before pprof aggregation.
    pub samples: usize,
}

/// gzip'd pprof bytes plus extraction/conversion stats.
pub struct EncodedProfile {
    /// gzip-compressed pprof protobuf, ready to write to disk.
    pub encoded: Vec<u8>,
    /// All CPU profiles found in the Chrome trace.
    pub profiles: Vec<ExtractedProfileInfo>,
    /// Stats returned by the underlying V8 cpuprofile converter.
    pub stats: cpuprofile_to_pprof::Stats,
}

/// Parse all V8 CPU profiles embedded in a Chrome trace-event JSON file.
pub fn extract_profiles(input: &str) -> Result<Vec<ExtractedProfileInfo>> {
    Ok(extract_cpu_profiles(input, 1000)?
        .into_iter()
        .map(|p| p.info)
        .collect())
}

/// Convert one embedded V8 CPU profile from Chrome trace JSON to pprof.
pub fn convert(input: &str, opts: &ConvertOptions) -> Result<EncodedProfile> {
    let profiles = extract_cpu_profiles(input, opts.default_sample_delta_us)?;
    if profiles.is_empty() {
        bail!("no V8 CPU Profile/ProfileChunk events found in Chrome trace");
    }
    if opts.profile_index >= profiles.len() {
        bail!(
            "profile index {} out of range; trace contains {} profile(s)",
            opts.profile_index,
            profiles.len()
        );
    }

    let infos: Vec<ExtractedProfileInfo> = profiles.iter().map(|p| p.info.clone()).collect();
    let selected = profiles
        .into_iter()
        .nth(opts.profile_index)
        .expect("profile_index range checked");

    let mut builder = Builder::new(selected.profile);
    if opts.no_demangle {
        builder = builder.demangle_with(|s| s.to_string());
    }
    if let Some(name) = opts.mapping_filename.clone() {
        builder = builder.mapping_filename(name);
    }
    let out = builder.encode()?;

    Ok(EncodedProfile {
        encoded: out.encoded,
        profiles: infos,
        stats: out.stats,
    })
}

#[derive(Debug)]
struct ExtractedCpuProfile {
    info: ExtractedProfileInfo,
    profile: CpuProfile,
}

fn extract_cpu_profiles(
    input: &str,
    default_sample_delta_us: i64,
) -> Result<Vec<ExtractedCpuProfile>> {
    let events = parse_trace_events(input)?;
    let mut streams: BTreeMap<String, PartialProfile> = BTreeMap::new();

    for event in events {
        let Some(name) = event.name.as_deref() else {
            continue;
        };
        if name != "Profile" && name != "ProfileChunk" {
            continue;
        }
        let key = event.profile_key();
        let stream = match streams.entry(key.clone()) {
            Entry::Vacant(v) => v.insert(PartialProfile {
                key,
                ..PartialProfile::default()
            }),
            Entry::Occupied(o) => o.into_mut(),
        };
        ingest_profile_event(stream, &event)
            .with_context(|| format!("reading Chrome trace event `{name}`"))?;
    }

    let mut out = Vec::new();
    for (_key, stream) in streams {
        if let Some(profile) = stream.into_profile(default_sample_delta_us)? {
            out.push(profile);
        }
    }
    Ok(out)
}

fn ingest_profile_event(stream: &mut PartialProfile, event: &TraceEvent) -> Result<()> {
    let Some(data) = event.data() else {
        return Ok(());
    };

    stream.start_time = stream.start_time.or_else(|| i64_field(data, "startTime"));
    stream.end_time = stream.end_time.or_else(|| i64_field(data, "endTime"));

    let Some(cpu_profile_value) = data.get("cpuProfile").or_else(|| {
        if data.get("nodes").is_some() {
            Some(data)
        } else {
            None
        }
    }) else {
        return Ok(());
    };

    stream.start_time = stream
        .start_time
        .or_else(|| i64_field(cpu_profile_value, "startTime"));
    stream.end_time = stream
        .end_time
        .or_else(|| i64_field(cpu_profile_value, "endTime"));

    let raw: RawCpuProfile = serde_json::from_value(cpu_profile_value.clone())
        .context("parsing embedded V8 cpuProfile")?;
    let external_samples = i64_vec_field(data, "samples")?;
    let external_time_deltas = i64_vec_field(data, "timeDeltas")?;

    stream.chunks.push(ProfileChunk {
        ts: event.ts(),
        nodes: raw.nodes,
        samples: if external_samples.is_empty() {
            raw.samples
        } else {
            external_samples
        },
        time_deltas: if external_time_deltas.is_empty() {
            raw.time_deltas
        } else {
            external_time_deltas
        },
    });
    Ok(())
}

#[derive(Default)]
struct PartialProfile {
    key: String,
    start_time: Option<i64>,
    end_time: Option<i64>,
    chunks: Vec<ProfileChunk>,
}

impl PartialProfile {
    fn into_profile(mut self, default_sample_delta_us: i64) -> Result<Option<ExtractedCpuProfile>> {
        self.chunks.sort_by_key(|c| c.ts);
        if self.chunks.is_empty() {
            return Ok(None);
        }

        let mut node_order = Vec::new();
        let mut nodes_by_id: HashMap<i64, RawNode> = HashMap::new();
        let mut parent_links = Vec::new();
        let mut samples = Vec::new();
        let mut time_deltas = Vec::new();

        for chunk in self.chunks {
            for node in chunk.nodes {
                if let Some(parent) = node.parent {
                    parent_links.push((parent, node.id));
                }
                match nodes_by_id.entry(node.id) {
                    std::collections::hash_map::Entry::Vacant(v) => {
                        node_order.push(node.id);
                        v.insert(node);
                    }
                    std::collections::hash_map::Entry::Occupied(mut o) => {
                        let existing = o.get_mut();
                        for child in node.children {
                            push_unique(&mut existing.children, child);
                        }
                        if existing.parent.is_none() {
                            existing.parent = node.parent;
                        }
                    }
                }
            }
            samples.extend(chunk.samples);
            time_deltas.extend(chunk.time_deltas);
        }

        for (parent, child) in parent_links {
            if let Some(parent_node) = nodes_by_id.get_mut(&parent) {
                push_unique(&mut parent_node.children, child);
            }
        }

        if time_deltas.len() < samples.len() {
            time_deltas.resize(samples.len(), default_sample_delta_us.max(1));
        } else if time_deltas.len() > samples.len() {
            time_deltas.truncate(samples.len());
        }

        let duration_us: i64 = time_deltas.iter().copied().sum();
        let start_time = self.start_time.unwrap_or(0);
        let end_time = self
            .end_time
            .unwrap_or(start_time.saturating_add(duration_us));

        let nodes = node_order
            .into_iter()
            .filter_map(|id| nodes_by_id.remove(&id))
            .map(|node| CpuNode {
                id: node.id,
                call_frame: node.call_frame,
                children: node.children,
            })
            .collect::<Vec<_>>();

        let info = ExtractedProfileInfo {
            key: self.key,
            nodes: nodes.len(),
            samples: samples.len(),
        };
        Ok(Some(ExtractedCpuProfile {
            info,
            profile: CpuProfile {
                nodes,
                samples,
                time_deltas,
                start_time,
                end_time,
            },
        }))
    }
}

fn push_unique(values: &mut Vec<i64>, value: i64) {
    if !values.contains(&value) {
        values.push(value);
    }
}

#[derive(Debug)]
struct ProfileChunk {
    ts: i64,
    nodes: Vec<RawNode>,
    samples: Vec<i64>,
    time_deltas: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCpuProfile {
    #[serde(default)]
    nodes: Vec<RawNode>,
    #[serde(default)]
    samples: Vec<i64>,
    #[serde(default)]
    time_deltas: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNode {
    id: i64,
    call_frame: CallFrame,
    #[serde(default)]
    children: Vec<i64>,
    #[serde(default)]
    parent: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TraceJson {
    Object(TraceObject),
    Events(Vec<TraceEvent>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraceObject {
    #[serde(default)]
    trace_events: Vec<TraceEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraceEvent {
    name: Option<String>,
    pid: Option<Value>,
    tid: Option<Value>,
    ts: Option<Value>,
    id: Option<Value>,
    id2: Option<Value>,
    #[serde(default)]
    args: Value,
}

impl TraceEvent {
    fn profile_key(&self) -> String {
        format!(
            "pid={}/tid={}/id={}",
            value_key(self.pid.as_ref()),
            value_key(self.tid.as_ref()),
            self.id
                .as_ref()
                .or(self.id2.as_ref())
                .map(|v| value_key(Some(v)))
                .unwrap_or_else(|| "none".to_string())
        )
    }

    fn ts(&self) -> i64 {
        self.ts.as_ref().and_then(value_to_i64).unwrap_or(0)
    }

    fn data(&self) -> Option<&Value> {
        match &self.args {
            Value::Object(map) => map.get("data").or(Some(&self.args)),
            _ => None,
        }
    }
}

fn parse_trace_events(input: &str) -> Result<Vec<TraceEvent>> {
    let trace: TraceJson = serde_json::from_str(input).context("parsing Chrome trace JSON")?;
    Ok(match trace {
        TraceJson::Object(o) => o.trace_events,
        TraceJson::Events(events) => events,
    })
}

fn i64_field(value: &Value, field: &str) -> Option<i64> {
    value.get(field).and_then(value_to_i64)
}

fn i64_vec_field(value: &Value, field: &str) -> Result<Vec<i64>> {
    match value.get(field) {
        Some(v) => serde_json::from_value(v.clone())
            .with_context(|| format!("parsing `{field}` as integer array")),
        None => Ok(Vec::new()),
    }
}

fn value_to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        Some(v)
    } else if let Some(v) = value.as_u64() {
        i64::try_from(v).ok()
    } else {
        value.as_f64().map(|v| v.round() as i64)
    }
}

fn value_key(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use prost::Message as _;
    use std::io::Read as _;

    const TRACE_WITH_PARENT_LINKS: &str = r#"{
      "traceEvents": [
        {
          "pid": 7,
          "tid": 11,
          "ts": 1000,
          "ph": "P",
          "cat": "disabled-by-default-v8.cpu_profiler",
          "name": "Profile",
          "id": "0x1",
          "args": { "data": { "startTime": 1000 } }
        },
        {
          "pid": 7,
          "tid": 11,
          "ts": 1000,
          "ph": "P",
          "cat": "disabled-by-default-v8.cpu_profiler",
          "name": "ProfileChunk",
          "id": "0x1",
          "args": {
            "data": {
              "cpuProfile": {
                "nodes": [
                  {
                    "id": 1,
                    "callFrame": {
                      "functionName": "(root)",
                      "scriptId": "0",
                      "url": "",
                      "lineNumber": -1,
                      "columnNumber": -1
                    }
                  },
                  {
                    "id": 2,
                    "parent": 1,
                    "callFrame": {
                      "functionName": "hot",
                      "scriptId": "1",
                      "url": "file:///main.js",
                      "lineNumber": 4,
                      "columnNumber": 0
                    }
                  },
                  {
                    "id": 3,
                    "parent": 2,
                    "callFrame": {
                      "functionName": "leaf",
                      "scriptId": "1",
                      "url": "file:///main.js",
                      "lineNumber": 8,
                      "columnNumber": 0
                    }
                  }
                ]
              },
              "samples": [3, 3, 2],
              "timeDeltas": [1000, 1500, 500]
            }
          }
        }
      ]
    }"#;

    const TRACE_ARRAY: &str = r#"[
      {
        "pid": 1,
        "tid": 2,
        "ts": 10,
        "name": "ProfileChunk",
        "id": "inline",
        "args": {
          "data": {
            "cpuProfile": {
              "nodes": [
                {
                  "id": 1,
                  "callFrame": {
                    "functionName": "(root)",
                    "scriptId": "0",
                    "url": "",
                    "lineNumber": -1,
                    "columnNumber": -1
                  },
                  "children": [2]
                },
                {
                  "id": 2,
                  "callFrame": {
                    "functionName": "work",
                    "scriptId": "1",
                    "url": "file:///work.js",
                    "lineNumber": 0,
                    "columnNumber": 0
                  }
                }
              ],
              "samples": [2],
              "timeDeltas": [250]
            }
          }
        }
      }
    ]"#;

    #[test]
    fn extracts_profile_streams_from_trace_object() {
        let profiles = extract_profiles(TRACE_WITH_PARENT_LINKS).unwrap();
        assert_eq!(
            profiles,
            vec![ExtractedProfileInfo {
                key: "pid=7/tid=11/id=0x1".to_string(),
                nodes: 3,
                samples: 3,
            }]
        );
    }

    #[test]
    fn converts_profile_chunks_to_pprof() {
        let out = convert(TRACE_WITH_PARENT_LINKS, &ConvertOptions::default()).unwrap();
        assert_eq!(out.profiles.len(), 1);
        assert_eq!(out.stats.samples, 2);
        assert_eq!(out.stats.locations, 3);

        let profile = decode_pprof(&out.encoded);
        assert_eq!(profile.sample.len(), 2);
        let total_count: i64 = profile.sample.iter().map(|s| s.value[0]).sum();
        assert_eq!(total_count, 3);
        let total_ns: i64 = profile.sample.iter().map(|s| s.value[1]).sum();
        assert_eq!(total_ns, 3_000_000);

        let names: Vec<&str> = profile
            .function
            .iter()
            .map(|f| profile.string_table[f.name as usize].as_str())
            .collect();
        assert!(names.contains(&"hot"));
        assert!(names.contains(&"leaf"));
    }

    #[test]
    fn accepts_top_level_trace_event_arrays_and_nested_samples() {
        let profiles = extract_profiles(TRACE_ARRAY).unwrap();
        assert_eq!(profiles[0].nodes, 2);
        assert_eq!(profiles[0].samples, 1);

        let out = convert(TRACE_ARRAY, &ConvertOptions::default()).unwrap();
        let profile = decode_pprof(&out.encoded);
        assert_eq!(profile.sample.len(), 1);
        assert_eq!(profile.sample[0].value, vec![1, 250_000]);
    }

    fn decode_pprof(bytes: &[u8]) -> firefox_to_pprof::proto::Profile {
        let mut decoder = GzDecoder::new(bytes);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        firefox_to_pprof::proto::Profile::decode(raw.as_slice()).unwrap()
    }
}
