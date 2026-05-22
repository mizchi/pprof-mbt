//! Serde model for the Firefox Profiler "processed profile" JSON.
//!
//! Only the fields we care about are present; everything else is silently
//! ignored. Field names mirror the JS schema so external code reading
//! `runners/lib/firefox-to-pprof.mjs` translates 1:1.

use serde::Deserialize;

/// A parsed Firefox profile.
#[derive(Debug, Deserialize)]
pub struct FirefoxProfile {
    /// Loaded shared libraries / wasm modules.
    #[serde(default)]
    pub libs: Vec<Lib>,
    /// One profile per sampled thread.
    pub threads: Vec<Thread>,
    /// Profile-wide metadata (sampling interval, start time, …).
    #[serde(default)]
    pub meta: Meta,
}

/// Profile-wide metadata.
#[derive(Debug, Deserialize)]
pub struct Meta {
    /// Nominal sampling interval in milliseconds.
    #[serde(default = "default_interval")]
    pub interval: f64,
    /// Start time in milliseconds since the Unix epoch.
    #[serde(rename = "startTime")]
    #[serde(default)]
    pub start_time: f64,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            interval: 1.0,
            start_time: 0.0,
        }
    }
}

fn default_interval() -> f64 {
    1.0
}

/// One loaded module (shared library, wasm module, …).
#[derive(Debug, Deserialize)]
pub struct Lib {
    /// Display name (e.g. `"main.wasm"`).
    #[serde(default)]
    pub name: String,
    /// samply-style debug identifier; absent in wasmtime output.
    #[serde(rename = "debugName")]
    #[serde(default)]
    pub debug_name: String,
}

/// One sampled thread.
#[derive(Debug, Deserialize)]
pub struct Thread {
    /// Frame table — struct-of-arrays.
    #[serde(rename = "frameTable")]
    pub frame_table: FrameTable,
    /// Function table — struct-of-arrays.
    #[serde(rename = "funcTable")]
    pub func_table: FuncTable,
    /// Stack table — linked list of frames via `prefix`.
    #[serde(rename = "stackTable")]
    pub stack_table: StackTable,
    /// Samples — `stack[i]` is the leaf stack id, `timeDeltas[i]` is the ms
    /// elapsed since the previous sample.
    pub samples: Samples,
    /// Shared string pool; indices live in other tables.
    #[serde(rename = "stringArray")]
    pub string_array: Vec<String>,
    /// Resource table (libs etc). Optional — wasmtime omits it.
    #[serde(rename = "resourceTable")]
    #[serde(default)]
    pub resource_table: Option<ResourceTable>,
}

/// Struct-of-arrays frame table.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct FrameTable {
    /// Number of frames.
    pub length: usize,
    /// `func[i]` — funcTable index for frame `i`.
    pub func: Vec<i64>,
    /// `address[i]` — RVA inside the owning lib.
    #[serde(default)]
    pub address: Vec<i64>,
    /// `line[i]` — source line, if known.
    #[serde(default)]
    pub line: Vec<Option<i64>>,
}

/// Struct-of-arrays function table.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct FuncTable {
    /// Number of functions.
    pub length: usize,
    /// `name[i]` — stringArray index for the function's name.
    pub name: Vec<i64>,
    /// `fileName[i]` — stringArray index for the source filename.
    #[serde(rename = "fileName")]
    #[serde(default)]
    pub file_name: Vec<Option<i64>>,
    /// `resource[i]` — resourceTable index (samply uses this for lib lookup).
    #[serde(default)]
    pub resource: Vec<i64>,
}

/// Struct-of-arrays stack table (linked list).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct StackTable {
    /// Number of stacks.
    pub length: usize,
    /// `prefix[i]` — parent stack id, or null for root.
    pub prefix: Vec<Option<i64>>,
    /// `frame[i]` — frameTable index at this stack node.
    pub frame: Vec<i64>,
}

/// Struct-of-arrays sample table.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Samples {
    /// Number of samples.
    pub length: usize,
    /// `stack[i]` — leaf stack id, or null if no stack was captured.
    pub stack: Vec<Option<i64>>,
    /// `timeDeltas[i]` — ms since the previous sample (wasmtime only).
    #[serde(rename = "timeDeltas")]
    #[serde(default)]
    pub time_deltas: Vec<f64>,
    /// `weight[i]` — multiplier for this sample's count.
    #[serde(default)]
    pub weight: Vec<f64>,
}

/// Struct-of-arrays resource table (libs / categories).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ResourceTable {
    /// Number of resources.
    pub length: usize,
    /// `lib[i]` — libs index for this resource.
    #[serde(default)]
    pub lib: Vec<i64>,
}
