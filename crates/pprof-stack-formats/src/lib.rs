//! Convert pprof CPU profiles to folded stacks and Speedscope JSON, and
//! convert Speedscope sampled profiles back to gzip'd pprof.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;

use anyhow::{bail, Context as _, Result};
use firefox_to_pprof::proto;
use flate2::write::GzEncoder;
use flate2::Compression;
use prost::Message as _;
use serde::{Deserialize, Serialize};

/// Options shared by pprof exporters.
#[derive(Debug, Clone, Default)]
pub struct PprofExportOptions {
    /// pprof sample value index to export. If omitted, the first
    /// CPU/wall time axis is selected.
    pub value_index: Option<usize>,
}

/// Options for Speedscope export.
#[derive(Debug, Clone)]
pub struct SpeedscopeExportOptions {
    /// pprof sample value index to export.
    pub value_index: Option<usize>,
    /// File-level name.
    pub name: String,
    /// Profile-level name.
    pub profile_name: String,
}

impl Default for SpeedscopeExportOptions {
    fn default() -> Self {
        Self {
            value_index: None,
            name: "pprof".to_string(),
            profile_name: "pprof".to_string(),
        }
    }
}

/// Options for Speedscope import.
#[derive(Debug, Clone)]
pub struct SpeedscopeImportOptions {
    /// Which profile to import.
    pub profile_index: usize,
    /// Override the pprof Mapping filename.
    pub mapping_filename: String,
}

impl Default for SpeedscopeImportOptions {
    fn default() -> Self {
        Self {
            profile_index: 0,
            mapping_filename: "speedscope".to_string(),
        }
    }
}

/// Options for folded stack import.
#[derive(Debug, Clone)]
pub struct FoldedImportOptions {
    /// pprof sample type for the folded value column.
    pub sample_type: String,
    /// pprof unit for the folded value column.
    pub unit: String,
    /// Override the pprof Mapping filename.
    pub mapping_filename: String,
}

impl Default for FoldedImportOptions {
    fn default() -> Self {
        Self {
            sample_type: "delay".to_string(),
            unit: "microseconds".to_string(),
            mapping_filename: "folded".to_string(),
        }
    }
}

/// Export stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportStats {
    /// Number of stacks emitted before any textual aggregation.
    pub stacks: usize,
    /// Sum of exported values.
    pub total: i64,
}

/// Import stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    /// Number of pprof samples emitted.
    pub samples: usize,
    /// Number of pprof locations emitted.
    pub locations: usize,
    /// Sum of imported sample weights.
    pub total: i64,
}

/// Output from pprof importers.
pub struct EncodedPprof {
    /// gzip-compressed pprof protobuf.
    pub encoded: Vec<u8>,
    /// Import stats.
    pub stats: ImportStats,
}

/// Convert pprof to folded stack lines: `root;child;leaf value`.
pub fn pprof_to_folded(
    profile: &proto::Profile,
    opts: &PprofExportOptions,
) -> Result<(String, ExportStats)> {
    let axis = select_pprof_axis(profile, opts.value_index)?;
    let resolver = PprofResolver::new(profile);
    let mut folded: BTreeMap<String, i64> = BTreeMap::new();
    let mut stats = ExportStats {
        stacks: 0,
        total: 0,
    };

    for sample in &profile.sample {
        let value = sample.value.get(axis.idx).copied().unwrap_or(0);
        if value <= 0 {
            continue;
        }
        let stack = resolver
            .stack(sample)
            .into_iter()
            .map(|f| sanitize_folded_frame(&f.name))
            .collect::<Vec<_>>();
        if stack.is_empty() {
            continue;
        }
        *folded.entry(stack.join(";")).or_default() += value;
        stats.stacks += 1;
        stats.total += value;
    }

    let mut out = String::new();
    for (stack, value) in folded {
        out.push_str(&stack);
        out.push(' ');
        out.push_str(&value.to_string());
        out.push('\n');
    }
    Ok((out, stats))
}

/// Convert pprof to Speedscope JSON.
pub fn pprof_to_speedscope(
    profile: &proto::Profile,
    opts: &SpeedscopeExportOptions,
) -> Result<(String, ExportStats)> {
    let axis = select_pprof_axis(profile, opts.value_index)?;
    let resolver = PprofResolver::new(profile);
    let mut frame_ids: HashMap<Frame, usize> = HashMap::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut samples: Vec<Vec<usize>> = Vec::new();
    let mut weights: Vec<f64> = Vec::new();
    let mut stats = ExportStats {
        stacks: 0,
        total: 0,
    };

    for sample in &profile.sample {
        let value = sample.value.get(axis.idx).copied().unwrap_or(0);
        if value <= 0 {
            continue;
        }
        let stack = resolver.stack(sample);
        if stack.is_empty() {
            continue;
        }
        let mut stack_ids = Vec::with_capacity(stack.len());
        for frame in stack {
            let id = if let Some(&id) = frame_ids.get(&frame) {
                id
            } else {
                let id = frames.len();
                frame_ids.insert(frame.clone(), id);
                frames.push(frame);
                id
            };
            stack_ids.push(id);
        }
        samples.push(stack_ids);
        weights.push(value as f64);
        stats.stacks += 1;
        stats.total += value;
    }

    let speedscope = SpeedscopeFile {
        schema: "https://www.speedscope.app/file-format-schema.json".to_string(),
        name: opts.name.clone(),
        active_profile_index: 0,
        exporter: "moon-pprof".to_string(),
        shared: Shared { frames },
        profiles: vec![SampledProfile {
            profile_type: "sampled".to_string(),
            name: opts.profile_name.clone(),
            unit: axis.unit.to_string(),
            start_value: 0.0,
            end_value: stats.total as f64,
            samples,
            weights,
        }],
    };
    Ok((
        serde_json::to_string_pretty(&speedscope).context("encoding Speedscope JSON")?,
        stats,
    ))
}

/// Convert folded stack text (`root;child;leaf value`) to gzip-compressed pprof.
pub fn folded_to_pprof(input: &str, opts: &FoldedImportOptions) -> Result<EncodedPprof> {
    let mut strings = StringPool::new();
    let samples_str = strings.intern("samples");
    let count_str = strings.intern("count");
    let value_type_str = strings.intern(&opts.sample_type);
    let value_unit_str = strings.intern(&opts.unit);
    let empty_str = strings.intern("");
    let mapping_filename = strings.intern(&opts.mapping_filename);

    let mut frame_ids: HashMap<String, u64> = HashMap::new();
    let mut functions = Vec::new();
    let mut locations = Vec::new();
    let mut samples = Vec::new();
    let mut total = 0i64;

    for (line_idx, line) in input.lines().enumerate() {
        let Some((frames, value)) = parse_folded_line(line, line_idx + 1)? else {
            continue;
        };
        let mut root_to_leaf = Vec::with_capacity(frames.len());
        for frame in frames {
            let id = if let Some(&id) = frame_ids.get(&frame) {
                id
            } else {
                let id = (functions.len() + 1) as u64;
                let name = strings.intern(&frame);
                frame_ids.insert(frame, id);
                functions.push(proto::Function {
                    id,
                    name,
                    system_name: name,
                    filename: empty_str,
                    start_line: 0,
                });
                locations.push(proto::Location {
                    id,
                    mapping_id: 1,
                    address: 0,
                    line: vec![proto::Line {
                        function_id: id,
                        line: 0,
                        column: 0,
                    }],
                    is_folded: false,
                });
                id
            };
            root_to_leaf.push(id);
        }
        samples.push(proto::Sample {
            location_id: root_to_leaf.into_iter().rev().collect(),
            value: vec![1, value],
            label: vec![],
        });
        total += value;
    }

    let stats = ImportStats {
        samples: samples.len(),
        locations: locations.len(),
        total,
    };
    let pprof = proto::Profile {
        sample_type: vec![
            proto::ValueType {
                r#type: samples_str,
                unit: count_str,
            },
            proto::ValueType {
                r#type: value_type_str,
                unit: value_unit_str,
            },
        ],
        sample: samples,
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
        location: locations,
        function: functions,
        string_table: strings.strings,
        drop_frames: 0,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: duration_nanos(total, &opts.unit),
        period_type: Some(proto::ValueType {
            r#type: value_type_str,
            unit: value_unit_str,
        }),
        period: 1,
        comment: vec![],
        default_sample_type: value_type_str,
        doc_url: 0,
    };

    encode_profile(pprof, stats)
}

/// Convert a Speedscope sampled profile JSON to gzip-compressed pprof.
pub fn speedscope_to_pprof(json: &str, opts: &SpeedscopeImportOptions) -> Result<EncodedPprof> {
    let file: SpeedscopeFile = serde_json::from_str(json).context("parsing Speedscope JSON")?;
    let Some(profile) = file.profiles.get(opts.profile_index) else {
        bail!(
            "profile index {} out of range; file has {} profile(s)",
            opts.profile_index,
            file.profiles.len()
        );
    };
    if profile.profile_type != "sampled" {
        bail!("only Speedscope sampled profiles are supported");
    }

    let mut strings = StringPool::new();
    let samples_str = strings.intern("samples");
    let count_str = strings.intern("count");
    let (value_type, value_unit) = speedscope_value_type(&profile.unit);
    let value_type_str = strings.intern(value_type);
    let value_unit_str = strings.intern(value_unit);
    let mapping_filename = strings.intern(&opts.mapping_filename);

    let mut functions = Vec::new();
    let mut locations = Vec::new();
    for (i, frame) in file.shared.frames.iter().enumerate() {
        let fid = (i + 1) as u64;
        let name = strings.intern(if frame.name.is_empty() {
            "(unknown)"
        } else {
            &frame.name
        });
        let filename = strings.intern(frame.file.as_deref().unwrap_or(""));
        functions.push(proto::Function {
            id: fid,
            name,
            system_name: name,
            filename,
            start_line: frame.line.unwrap_or(0),
        });
        locations.push(proto::Location {
            id: fid,
            mapping_id: 1,
            address: 0,
            line: vec![proto::Line {
                function_id: fid,
                line: frame.line.unwrap_or(0),
                column: frame.col.unwrap_or(0),
            }],
            is_folded: false,
        });
    }

    let mut samples = Vec::new();
    let mut total = 0i64;
    for (i, stack) in profile.samples.iter().enumerate() {
        let weight = profile
            .weights
            .get(i)
            .copied()
            .unwrap_or(1.0)
            .round()
            .max(0.0) as i64;
        if weight == 0 {
            continue;
        }
        let location_id = stack
            .iter()
            .rev()
            .filter_map(|&idx| locations.get(idx).map(|l| l.id))
            .collect::<Vec<_>>();
        if location_id.is_empty() {
            continue;
        }
        samples.push(proto::Sample {
            location_id,
            value: vec![1, weight],
            label: vec![],
        });
        total += weight;
    }

    let stats = ImportStats {
        samples: samples.len(),
        locations: locations.len(),
        total,
    };
    let pprof = proto::Profile {
        sample_type: vec![
            proto::ValueType {
                r#type: samples_str,
                unit: count_str,
            },
            proto::ValueType {
                r#type: value_type_str,
                unit: value_unit_str,
            },
        ],
        sample: samples,
        mapping: vec![proto::Mapping {
            id: 1,
            memory_start: 0,
            memory_limit: 0,
            file_offset: 0,
            filename: mapping_filename,
            build_id: 0,
            has_functions: true,
            has_filenames: true,
            has_line_numbers: true,
            has_inline_frames: false,
        }],
        location: locations,
        function: functions,
        string_table: strings.strings,
        drop_frames: 0,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: speedscope_duration_nanos(profile),
        period_type: Some(proto::ValueType {
            r#type: value_type_str,
            unit: value_unit_str,
        }),
        period: 1,
        comment: vec![],
        default_sample_type: value_type_str,
        doc_url: 0,
    };

    encode_profile(pprof, stats)
}

fn encode_profile(pprof: proto::Profile, stats: ImportStats) -> Result<EncodedPprof> {
    let mut buf = Vec::new();
    pprof.encode(&mut buf).context("encoding pprof Profile")?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&buf)?;
    Ok(EncodedPprof {
        encoded: gz.finish()?,
        stats,
    })
}

fn parse_folded_line(line: &str, line_no: usize) -> Result<Option<(Vec<String>, i64)>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }
    let Some(split_at) = trimmed.rfind(char::is_whitespace) else {
        bail!("folded line {line_no}: missing value column");
    };
    let (stack_raw, value_raw) = trimmed.split_at(split_at);
    let stack_raw = stack_raw.trim();
    let value_raw = value_raw.trim();
    if stack_raw.is_empty() {
        bail!("folded line {line_no}: empty stack");
    }
    let value = value_raw
        .parse::<f64>()
        .with_context(|| format!("folded line {line_no}: invalid value `{value_raw}`"))?;
    if !value.is_finite() {
        bail!("folded line {line_no}: value must be finite");
    }
    if value < 0.0 {
        bail!("folded line {line_no}: value must be non-negative");
    }
    let value = value.round();
    if value > i64::MAX as f64 {
        bail!("folded line {line_no}: value is too large");
    }
    let value = value as i64;
    if value == 0 {
        return Ok(None);
    }

    let mut frames = Vec::new();
    for raw in stack_raw.split(';') {
        let frame = raw.trim();
        if frame.is_empty() {
            bail!("folded line {line_no}: empty frame in stack");
        }
        frames.push(frame.to_string());
    }
    Ok(Some((frames, value)))
}

fn duration_nanos(value: i64, unit: &str) -> i64 {
    match unit {
        "nanoseconds" => value,
        "microseconds" => value.saturating_mul(1000),
        "milliseconds" => value.saturating_mul(1_000_000),
        "seconds" => value.saturating_mul(1_000_000_000),
        _ => 0,
    }
}

#[derive(Clone, Copy)]
struct Axis<'a> {
    idx: usize,
    unit: &'a str,
}

fn select_pprof_axis(profile: &proto::Profile, explicit: Option<usize>) -> Result<Axis<'_>> {
    if let Some(idx) = explicit {
        let Some(st) = profile.sample_type.get(idx) else {
            bail!(
                "value index {} out of range; profile has {} sample type(s)",
                idx,
                profile.sample_type.len()
            );
        };
        return Ok(Axis {
            idx,
            unit: string_at(profile, st.unit),
        });
    }
    for (i, st) in profile.sample_type.iter().enumerate() {
        let ty = string_at(profile, st.r#type);
        let unit = string_at(profile, st.unit);
        if (ty == "cpu" || ty == "wall") && is_speedscope_unit(unit) {
            return Ok(Axis { idx: i, unit });
        }
    }
    bail!("no CPU/wall Speedscope-compatible sample_type found; pass --value-index")
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

fn is_speedscope_unit(unit: &str) -> bool {
    matches!(
        unit,
        "none" | "nanoseconds" | "microseconds" | "milliseconds" | "seconds" | "bytes"
    )
}

#[derive(Clone)]
struct PprofResolver<'a> {
    profile: &'a proto::Profile,
    locations: HashMap<u64, &'a proto::Location>,
    functions: HashMap<u64, &'a proto::Function>,
}

impl<'a> PprofResolver<'a> {
    fn new(profile: &'a proto::Profile) -> Self {
        Self {
            profile,
            locations: profile.location.iter().map(|l| (l.id, l)).collect(),
            functions: profile.function.iter().map(|f| (f.id, f)).collect(),
        }
    }

    fn stack(&self, sample: &proto::Sample) -> Vec<Frame> {
        sample
            .location_id
            .iter()
            .rev()
            .map(|&id| self.frame_for_location(id))
            .collect()
    }

    fn frame_for_location(&self, loc_id: u64) -> Frame {
        let Some(loc) = self.locations.get(&loc_id).copied() else {
            return Frame {
                name: format!("location:{loc_id}"),
                file: None,
                line: None,
                col: None,
            };
        };
        let Some(line) = loc.line.first() else {
            return Frame {
                name: format!("0x{:x}", loc.address),
                file: None,
                line: None,
                col: None,
            };
        };
        let Some(func) = self.functions.get(&line.function_id).copied() else {
            return Frame {
                name: format!("function:{}", line.function_id),
                file: None,
                line: None,
                col: None,
            };
        };
        let name = string_at(self.profile, func.name);
        let file = string_at(self.profile, func.filename);
        Frame {
            name: if name.is_empty() { "(unknown)" } else { name }.to_string(),
            file: if file.is_empty() {
                None
            } else {
                Some(file.to_string())
            },
            line: if line.line > 0 {
                Some(line.line)
            } else if func.start_line > 0 {
                Some(func.start_line)
            } else {
                None
            },
            col: if line.column > 0 {
                Some(line.column)
            } else {
                None
            },
        }
    }
}

fn sanitize_folded_frame(s: &str) -> String {
    s.replace(';', ":").replace('\n', " ")
}

fn speedscope_value_type(unit: &str) -> (&'static str, &'static str) {
    match unit {
        "nanoseconds" => ("cpu", "nanoseconds"),
        "microseconds" => ("cpu", "microseconds"),
        "milliseconds" => ("cpu", "milliseconds"),
        "seconds" => ("cpu", "seconds"),
        "bytes" => ("space", "bytes"),
        _ => ("samples", "count"),
    }
}

fn speedscope_duration_nanos(profile: &SampledProfile) -> i64 {
    let duration = (profile.end_value - profile.start_value).max(0.0).round() as i64;
    match profile.unit.as_str() {
        "nanoseconds" => duration,
        "microseconds" => duration.saturating_mul(1000),
        "milliseconds" => duration.saturating_mul(1_000_000),
        "seconds" => duration.saturating_mul(1_000_000_000),
        _ => 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Frame {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    col: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Shared {
    frames: Vec<Frame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpeedscopeFile {
    #[serde(rename = "$schema")]
    schema: String,
    name: String,
    #[serde(rename = "activeProfileIndex")]
    active_profile_index: usize,
    exporter: String,
    shared: Shared,
    profiles: Vec<SampledProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SampledProfile {
    #[serde(rename = "type")]
    profile_type: String,
    name: String,
    unit: String,
    #[serde(rename = "startValue")]
    start_value: f64,
    #[serde(rename = "endValue")]
    end_value: f64,
    samples: Vec<Vec<usize>>,
    #[serde(default)]
    weights: Vec<f64>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read as _;

    #[test]
    fn exports_folded_stacks() {
        let (folded, stats) =
            pprof_to_folded(&synthetic_pprof(), &PprofExportOptions::default()).unwrap();
        assert_eq!(
            stats,
            ExportStats {
                stacks: 2,
                total: 3_500_000,
            }
        );
        assert!(folded.contains("root;hot;leaf 3000000"));
        assert!(folded.contains("root;hot 500000"));
    }

    #[test]
    fn exports_speedscope_sampled_profile() {
        let (json, stats) =
            pprof_to_speedscope(&synthetic_pprof(), &SpeedscopeExportOptions::default()).unwrap();
        assert_eq!(
            stats,
            ExportStats {
                stacks: 2,
                total: 3_500_000,
            }
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["$schema"],
            "https://www.speedscope.app/file-format-schema.json"
        );
        assert_eq!(value["profiles"][0]["type"], "sampled");
        assert_eq!(value["profiles"][0]["unit"], "nanoseconds");
        assert_eq!(
            value["profiles"][0]["endValue"].as_f64().unwrap(),
            3_500_000.0
        );
        assert_eq!(value["profiles"][0]["samples"].as_array().unwrap().len(), 2);
        assert_eq!(value["shared"]["frames"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn imports_speedscope_sampled_profile_to_pprof() {
        let (json, _) =
            pprof_to_speedscope(&synthetic_pprof(), &SpeedscopeExportOptions::default()).unwrap();
        let out = speedscope_to_pprof(&json, &SpeedscopeImportOptions::default()).unwrap();
        assert_eq!(
            out.stats,
            ImportStats {
                samples: 2,
                locations: 3,
                total: 3_500_000,
            }
        );

        let profile = decode_pprof(&out.encoded);
        let total_count: i64 = profile.sample.iter().map(|s| s.value[0]).sum();
        let total_ns: i64 = profile.sample.iter().map(|s| s.value[1]).sum();
        assert_eq!(total_count, 2);
        assert_eq!(total_ns, 3_500_000);
    }

    #[test]
    fn imports_folded_stack_text_to_pprof() {
        let input = "root;wait;leaf 120\nroot;wait 30\n";
        let out = folded_to_pprof(input, &FoldedImportOptions::default()).unwrap();
        assert_eq!(
            out.stats,
            ImportStats {
                samples: 2,
                locations: 3,
                total: 150,
            }
        );

        let profile = decode_pprof(&out.encoded);
        assert_eq!(string_at(&profile, profile.sample_type[1].r#type), "delay");
        assert_eq!(
            string_at(&profile, profile.sample_type[1].unit),
            "microseconds"
        );
        assert_eq!(profile.duration_nanos, 150_000);

        let total_count: i64 = profile.sample.iter().map(|s| s.value[0]).sum();
        let total_delay: i64 = profile.sample.iter().map(|s| s.value[1]).sum();
        assert_eq!(total_count, 2);
        assert_eq!(total_delay, 150);
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
            time_nanos: 0,
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
