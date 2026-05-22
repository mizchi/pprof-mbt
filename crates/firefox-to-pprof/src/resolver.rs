//! Caller-supplied hooks: how to resolve a frame to a symbol, and how to
//! weigh a sample. Producers diverge here — samply does address lookups
//! against a sidecar, wasmtime reads its own funcTable directly.

use crate::{FirefoxProfile, Thread};

/// One resolved frame in a Location's inline chain.
///
/// pprof Locations may carry multiple `Line` entries to represent inlined
/// frames; resolvers return them leaf first. Many producers don't expose
/// inline info at all — return a single-element vector in that case.
#[derive(Debug, Clone, Default)]
pub struct ResolvedFrame {
    /// Raw (potentially mangled) function name.
    pub name: String,
    /// Source file path, if known.
    pub file: String,
    /// Source line number, if known.
    pub line: i64,
    /// Index into `FirefoxProfile.libs` for the owning mapping. The
    /// builder maps this onto pprof's `Mapping.id` (1-based).
    pub mapping_index: usize,
    /// Address / RVA within the owning lib. Stored in `Location.address`.
    pub address: u64,
}

/// One resolved sample.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedSample {
    /// Leaf stack id (None = skip this sample).
    pub stack: Option<i64>,
    /// Sample count (multiplied by any caller-supplied weighting).
    pub count: i64,
    /// CPU time in nanoseconds attributed to this sample.
    pub ns: i64,
}

/// Resolve a frame index to a function call chain. See [`ResolvedFrame`]
/// for the contract.
pub trait FrameResolver {
    /// Resolve `thread.frameTable[frame_idx]` to one or more frames
    /// (leaf-first for inline chains).
    fn resolve(
        &self,
        profile: &FirefoxProfile,
        thread: &Thread,
        frame_idx: i64,
    ) -> Vec<ResolvedFrame>;
}

/// Resolve one sample.
pub trait SampleResolver {
    /// Inspect `thread.samples[i]` and produce the pprof-side sample.
    fn resolve(&self, profile: &FirefoxProfile, thread: &Thread, i: usize) -> ResolvedSample;
}

// Make trait objects ergonomic.
impl<T: FrameResolver + ?Sized> FrameResolver for Box<T> {
    fn resolve(&self, p: &FirefoxProfile, t: &Thread, frame_idx: i64) -> Vec<ResolvedFrame> {
        (**self).resolve(p, t, frame_idx)
    }
}
impl<T: SampleResolver + ?Sized> SampleResolver for Box<T> {
    fn resolve(&self, p: &FirefoxProfile, t: &Thread, i: usize) -> ResolvedSample {
        (**self).resolve(p, t, i)
    }
}

/// Convenience [`FrameResolver`] for producers (like wasmtime's GuestProfiler)
/// that pre-resolve symbols into `funcTable`. Each frame is treated as a
/// single-line Location.
pub struct FuncTableResolver;

impl FrameResolver for FuncTableResolver {
    fn resolve(
        &self,
        _profile: &FirefoxProfile,
        thread: &Thread,
        frame_idx: i64,
    ) -> Vec<ResolvedFrame> {
        let fi = frame_idx as usize;
        let func_idx = thread.frame_table.func[fi] as usize;
        let name_idx = thread.func_table.name[func_idx];
        let name = thread
            .string_array
            .get(name_idx as usize)
            .cloned()
            .unwrap_or_else(|| "(anonymous)".into());
        let file = thread
            .func_table
            .file_name
            .get(func_idx)
            .and_then(|s| *s)
            .and_then(|idx| thread.string_array.get(idx as usize).cloned())
            .unwrap_or_default();
        let line = thread
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
        vec![ResolvedFrame {
            name,
            file,
            line,
            mapping_index: 0,
            address,
        }]
    }
}

/// Sample weighting strategy.
#[derive(Debug, Clone, Copy)]
pub enum SampleWeighting {
    /// Fixed sampling rate. `count = weight (default 1)`, `ns = count × interval_ns`.
    FixedInterval {
        /// CPU time, in ns, attributed to each sample.
        interval_ns: i64,
    },
    /// Variable sampling rate from `samples.timeDeltas`. Falls back to
    /// `default_interval_ns` when `timeDeltas` is missing or zero.
    PerSampleTimeDeltas {
        /// CPU time, in ns, used when no per-sample `timeDeltas` entry is
        /// available (e.g. the first sample of a thread).
        default_interval_ns: i64,
    },
}

impl SampleResolver for SampleWeighting {
    fn resolve(&self, _profile: &FirefoxProfile, thread: &Thread, i: usize) -> ResolvedSample {
        let stack = thread.samples.stack.get(i).copied().flatten();
        let count = thread
            .samples
            .weight
            .get(i)
            .copied()
            .map(|w| w.round() as i64)
            .unwrap_or(1)
            .max(1);
        let ns = match *self {
            SampleWeighting::FixedInterval { interval_ns } => count * interval_ns,
            SampleWeighting::PerSampleTimeDeltas { default_interval_ns } => {
                let dt_ms = thread.samples.time_deltas.get(i).copied().unwrap_or(0.0);
                if dt_ms > 0.0 {
                    (dt_ms * 1_000_000.0).round() as i64
                } else {
                    default_interval_ns
                }
            }
        };
        ResolvedSample { stack, count, ns }
    }
}
