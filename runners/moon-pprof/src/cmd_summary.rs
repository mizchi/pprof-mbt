//! `moon-pprof summary <file>` — print self-time / mem-mgmt rollup.
//! `moon-pprof summary --diff <a> <b>` — diff two profiles at function granularity.

use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use firefox_to_pprof::proto::Profile;
use flate2::read::GzDecoder;
use prost::Message;
use regex::Regex;

/// Default regex matching MoonBit's runtime mem-mgmt symbols. Tuned
/// against json_parse / sorted_map_merge / regex_match outputs.
const DEFAULT_MEM_PATTERN: &str = r"^(moonbit\.(incref|decref|gc\.malloc|gc\.free|malloc|free|make_array_header|get_tag|array_length|check_range|drop_object)|tlsf/.+|moonbit_drop_object|libc_(malloc|free)|moonbit_malloc|moonbit_decref|moonbit_incref|_(?:malloc|free)|libsystem_malloc\..*)$";

#[derive(Parser, Debug)]
#[command(about = "Print pprof self-time / mem-mgmt rollup, or diff two profiles")]
pub struct Args {
    /// Profile to summarize (positional). With --diff, this is the baseline
    /// and the second positional is the patched profile.
    pub profile: PathBuf,

    /// Patched profile (only used with --diff).
    pub patched: Option<PathBuf>,

    /// Diff two profiles instead of summarizing one.
    #[arg(long, short)]
    pub diff: bool,

    /// Regex matched against function names to classify memory-management
    /// primitives in the rollup. Default is tuned for MoonBit. Falls back
    /// to $PPROF_SUMMARY_MEM_PATTERN env var.
    #[arg(long)]
    pub mem_pattern: Option<String>,
}

pub fn run(args: Args) -> Result<()> {
    let mem_pattern = args
        .mem_pattern
        .clone()
        .or_else(|| env::var("PPROF_SUMMARY_MEM_PATTERN").ok())
        .unwrap_or_else(|| DEFAULT_MEM_PATTERN.to_string());
    let mem_mgmt = mem_mgmt_re(&mem_pattern)?;

    if args.diff {
        let patched = args
            .patched
            .as_ref()
            .ok_or_else(|| anyhow!("--diff needs two positional args (base, patched)"))?;
        run_diff(&args.profile, patched, &mem_mgmt)
    } else {
        if args.patched.is_some() {
            return Err(anyhow!("second positional only allowed with --diff"));
        }
        run_single(&args.profile, &mem_mgmt)
    }
}

fn mem_mgmt_re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("compile --mem-pattern regex: {}", pattern))
}

/// What "primary axis" the summary uses for ranking and totals. CPU
/// profiles get cpu/wall time; heap profiles get bytes. The Bytes
/// variant is rendered with a 1024-base human formatter (`go tool
/// pprof` convention: 12.34kB / 1.23MB / etc.).
enum AxisKind {
    Time { div: f64, label: &'static str },
    Bytes,
    Count(&'static str),
}

struct ValueAxis {
    idx: usize,
    kind: AxisKind,
    /// Optional secondary axis (idx into sample_type) we still want to
    /// surface — e.g. alloc_objects alongside alloc_space in heap mode.
    /// None for CPU profiles.
    secondary_idx: Option<usize>,
    /// True iff this profile is a heap / allocation profile. Drives
    /// layout choices in `run_single` and `run_diff`.
    is_heap: bool,
}

impl AxisKind {
    fn format(&self, raw: i64) -> String {
        match self {
            AxisKind::Time { div, label } => format!("{:.2} {}", raw as f64 / div, label),
            AxisKind::Bytes => format_bytes(raw),
            AxisKind::Count(label) => format!("{} {}", raw, label),
        }
    }

    fn format_signed(&self, raw: i64) -> String {
        match self {
            AxisKind::Time { div, label } => format!("{:+.2} {}", raw as f64 / div, label),
            AxisKind::Bytes => {
                let sign = if raw < 0 { "-" } else { "+" };
                format!("{sign}{}", format_bytes(raw.abs()))
            }
            AxisKind::Count(label) => format!("{:+} {}", raw, label),
        }
    }

    /// Width hint for right-aligning the formatted value in tables.
    fn width(&self) -> usize {
        match self {
            AxisKind::Time { .. } => 12,
            AxisKind::Bytes => 11,
            AxisKind::Count(_) => 12,
        }
    }
}

fn format_bytes(raw: i64) -> String {
    let v = raw as f64;
    let abs = v.abs();
    if abs < 1024.0 {
        format!("{}B", raw)
    } else if abs < 1024.0 * 1024.0 {
        format!("{:.2}kB", v / 1024.0)
    } else if abs < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2}MB", v / (1024.0 * 1024.0))
    } else {
        format!("{:.2}GB", v / (1024.0 * 1024.0 * 1024.0))
    }
}

fn value_axis(p: &Profile) -> ValueAxis {
    // Detect heap profile: any sample_type named alloc_space /
    // alloc_objects / inuse_space / inuse_objects flips us into heap
    // mode. Bytes-flavored axes win over count-flavored ones (more
    // signal in most investigations).
    let mut alloc_space_idx: Option<usize> = None;
    let mut alloc_objects_idx: Option<usize> = None;
    for (i, st) in p.sample_type.iter().enumerate() {
        let ty = string_at(p, st.r#type);
        match ty {
            "alloc_space" | "inuse_space" | "space" => {
                if alloc_space_idx.is_none() {
                    alloc_space_idx = Some(i);
                }
            }
            "alloc_objects" | "inuse_objects" | "objects" => {
                if alloc_objects_idx.is_none() {
                    alloc_objects_idx = Some(i);
                }
            }
            _ => {}
        }
    }
    if let Some(idx) = alloc_space_idx {
        return ValueAxis {
            idx,
            kind: AxisKind::Bytes,
            secondary_idx: alloc_objects_idx,
            is_heap: true,
        };
    }
    if let Some(idx) = alloc_objects_idx {
        return ValueAxis {
            idx,
            kind: AxisKind::Count("allocs"),
            secondary_idx: None,
            is_heap: true,
        };
    }

    // CPU / wall / wait-like time profile. Go block/mutex profiles use
    // `delay`, and folded off-CPU imports default to the same axis.
    for (i, st) in p.sample_type.iter().enumerate() {
        let ty = string_at(p, st.r#type);
        let unit = string_at(p, st.unit);
        if is_time_sample_type(ty) {
            let (label, div) = match unit {
                "nanoseconds" => ("ms", 1e6),
                "microseconds" => ("ms", 1e3),
                "milliseconds" => ("ms", 1.0),
                "count" => {
                    return ValueAxis {
                        idx: i,
                        kind: AxisKind::Count("samples"),
                        secondary_idx: None,
                        is_heap: false,
                    };
                }
                _ => continue,
            };
            return ValueAxis {
                idx: i,
                kind: AxisKind::Time { div, label },
                secondary_idx: None,
                is_heap: false,
            };
        }
    }
    ValueAxis {
        idx: 0,
        kind: AxisKind::Count("units"),
        secondary_idx: None,
        is_heap: false,
    }
}

fn is_time_sample_type(ty: &str) -> bool {
    matches!(
        ty,
        "cpu" | "wall" | "delay" | "latency" | "block" | "blocked" | "off_cpu" | "wait"
    )
}

fn string_at(p: &Profile, idx: i64) -> &str {
    if idx < 0 {
        return "";
    }
    p.string_table.get(idx as usize).map(|s| s.as_str()).unwrap_or("")
}

fn resolve_top_lines(p: &Profile) -> HashMap<u64, String> {
    let mut function_name: HashMap<u64, String> = HashMap::new();
    for f in &p.function {
        function_name.insert(f.id, p.string_table.get(f.name as usize).cloned().unwrap_or_default());
    }
    let mut out: HashMap<u64, String> = HashMap::new();
    for loc in &p.location {
        let name = if let Some(line) = loc.line.first() {
            function_name.get(&line.function_id).cloned().unwrap_or_else(|| "(unknown)".into())
        } else {
            "(unknown)".into()
        };
        out.insert(loc.id, name);
    }
    out
}

#[derive(Default)]
struct FuncStats {
    self_v: i64,
    #[allow(dead_code)]
    cum: i64,
    mem_cum: i64,
    /// Secondary value (e.g. alloc_objects when primary is alloc_space).
    /// 0 when no secondary axis is present.
    self_v2: i64,
}

struct Summary {
    total: i64,
    total2: i64,
    mem_mgmt_total: i64,
    num_samples: usize,
    stats: HashMap<String, FuncStats>,
    axis: ValueAxis,
}

fn load_profile(path: &PathBuf) -> Result<Profile> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut raw = Vec::new();
    f.read_to_end(&mut raw)?;
    let buf = if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut dec = GzDecoder::new(&raw[..]);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).context("gunzip pprof")?;
        out
    } else {
        raw
    };
    Profile::decode(&*buf).context("decode pprof protobuf")
}

fn compute_summary(p: &Profile, mem_mgmt: &Regex) -> Summary {
    let axis = value_axis(p);
    let loc_to_name = resolve_top_lines(p);

    // Honor pprof's drop_frames field if set — pprof tools (go tool
    // pprof, speedscope) skip leaf frames matching this regex so the
    // visible leaf is the caller, not e.g. moonbit.malloc.
    let drop_re: Option<Regex> = if p.drop_frames > 0 {
        let pat = string_at(p, p.drop_frames);
        if pat.is_empty() {
            None
        } else {
            Regex::new(pat).ok()
        }
    } else {
        None
    };
    let pick_leaf = |sample: &firefox_to_pprof::proto::Sample| -> &str {
        for id in &sample.location_id {
            if let Some(name) = loc_to_name.get(id) {
                if let Some(re) = drop_re.as_ref() {
                    if re.is_match(name.as_str()) {
                        continue;
                    }
                }
                return name.as_str();
            }
        }
        "(unknown)"
    };

    let mut stats: HashMap<String, FuncStats> = HashMap::new();
    let mut total: i64 = 0;
    let mut total2: i64 = 0;
    let mut mem_mgmt_total: i64 = 0;

    for sample in &p.sample {
        let v = sample.value.get(axis.idx).copied().unwrap_or(0);
        let v2 = axis
            .secondary_idx
            .and_then(|i| sample.value.get(i).copied())
            .unwrap_or(0);
        total += v;
        total2 += v2;

        let leaf = pick_leaf(sample);

        let leaf_is_mem = !axis.is_heap && mem_mgmt.is_match(leaf);
        {
            let entry = stats.entry(leaf.to_string()).or_default();
            entry.self_v += v;
            entry.self_v2 += v2;
        }
        if leaf_is_mem {
            mem_mgmt_total += v;
        }

        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for id in &sample.location_id {
            if let Some(name) = loc_to_name.get(id) {
                if seen.insert(name.as_str()) {
                    stats.entry(name.clone()).or_default().cum += v;
                }
            }
        }

        if leaf_is_mem {
            for id in &sample.location_id {
                if let Some(name) = loc_to_name.get(id) {
                    if mem_mgmt.is_match(name) {
                        continue;
                    }
                    stats.entry(name.clone()).or_default().mem_cum += v;
                }
            }
        }
    }

    Summary {
        total,
        total2,
        mem_mgmt_total,
        num_samples: p.sample.len(),
        stats,
        axis,
    }
}

fn pct(num: i64, den: i64) -> f64 {
    if den == 0 {
        return 0.0;
    }
    100.0 * num as f64 / den as f64
}

fn print_top(
    title: &str,
    mut rows: Vec<(&String, i64)>,
    axis: &ValueAxis,
    total: i64,
    n: usize,
) {
    println!("{}", title);
    println!("{}", "-".repeat(title.len()));
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    let w = axis.kind.width();
    for (name, v) in rows.iter().take(n) {
        if *v == 0 {
            break;
        }
        println!(
            "  {:>w$}  {:>5.1}%  {}",
            axis.kind.format(*v),
            pct(*v, total),
            name,
            w = w,
        );
    }
    println!();
}

/// Like print_top but also shows a secondary column (e.g. alloc_objects
/// when primary is alloc_space). `total2` is the grand total of the
/// secondary axis for percentage display.
fn print_top_two_col(
    title: &str,
    secondary_label: &str,
    mut rows: Vec<(&String, i64, i64)>,
    axis: &ValueAxis,
    total: i64,
    n: usize,
) {
    println!("{}", title);
    println!("{}", "-".repeat(title.len()));
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    let w = axis.kind.width();
    for (name, v, v2) in rows.iter().take(n) {
        if *v == 0 {
            break;
        }
        println!(
            "  {:>w$}  {:>5.1}%  {:>6} {}  {}",
            axis.kind.format(*v),
            pct(*v, total),
            v2,
            secondary_label,
            name,
            w = w,
        );
    }
    println!();
}

fn run_single(path: &PathBuf, mem_mgmt: &Regex) -> Result<()> {
    let p = load_profile(path)?;
    let s = compute_summary(&p, mem_mgmt);

    println!("Profile: {}", path.display());

    if s.axis.is_heap {
        // Heap profile: every sample is an allocation, mem-mgmt regex
        // doesn't apply. Show total bytes + count, then per-site
        // bytes-and-count rankings.
        println!(
            "Total: {} across {} sites ({} alloc records)",
            s.axis.kind.format(s.total),
            s.stats.len(),
            s.num_samples,
        );
        if s.axis.secondary_idx.is_some() {
            println!("Total allocations: {}", s.total2);
        }
        println!();

        let rows: Vec<(&String, i64, i64)> = s
            .stats
            .iter()
            .map(|(n, st)| (n, st.self_v, st.self_v2))
            .collect();
        if s.axis.secondary_idx.is_some() {
            print_top_two_col(
                "Top allocation sites by bytes",
                "allocs",
                rows,
                &s.axis,
                s.total,
                15,
            );
        } else {
            let simple: Vec<(&String, i64)> = rows.into_iter().map(|(n, v, _)| (n, v)).collect();
            print_top("Top allocation sites", simple, &s.axis, s.total, 15);
        }
        return Ok(());
    }

    // CPU / wall profile: original layout (mem-mgmt regex rollup).
    println!(
        "Total: {} ({} samples)",
        s.axis.kind.format(s.total),
        s.num_samples,
    );
    println!(
        "Memory-management self time: {} ({:.1}%)",
        s.axis.kind.format(s.mem_mgmt_total),
        pct(s.mem_mgmt_total, s.total),
    );
    println!();

    let users: Vec<(&String, i64)> = s
        .stats
        .iter()
        .filter(|(name, _)| !mem_mgmt.is_match(name))
        .map(|(n, st)| (n, st.self_v))
        .collect();
    print_top(
        "Top user functions by self time (mem-mgmt frames hidden)",
        users,
        &s.axis,
        s.total,
        12,
    );

    let users_mc: Vec<(&String, i64)> = s
        .stats
        .iter()
        .filter(|(name, _)| !mem_mgmt.is_match(name))
        .map(|(n, st)| (n, st.mem_cum))
        .collect();
    print_top(
        "Top user functions by mem-mgmt-attributed time (callers of allocs)",
        users_mc,
        &s.axis,
        s.total,
        12,
    );

    let prims: Vec<(&String, i64)> = s
        .stats
        .iter()
        .filter(|(name, _)| mem_mgmt.is_match(name))
        .map(|(n, st)| (n, st.self_v))
        .collect();
    print_top(
        "Top mem-mgmt primitives by self time",
        prims,
        &s.axis,
        s.total,
        10,
    );
    Ok(())
}

fn run_diff(base_path: &PathBuf, patched_path: &PathBuf, mem_mgmt: &Regex) -> Result<()> {
    let base = load_profile(base_path)?;
    let patched = load_profile(patched_path)?;
    let bs = compute_summary(&base, mem_mgmt);
    let ps = compute_summary(&patched, mem_mgmt);

    if bs.axis.is_heap != ps.axis.is_heap {
        return Err(anyhow!(
            "base / patched are different kinds of profile (cpu vs heap) — can't diff"
        ));
    }
    let axis = &bs.axis;

    println!("Profile diff:");
    println!("  base    = {}", base_path.display());
    println!("  patched = {}", patched_path.display());
    let total_delta = ps.total - bs.total;
    println!();
    println!(
        "Total: {} ({} samples) -> {} ({} samples) | Δ {} ({:+.1}%)",
        axis.kind.format(bs.total),
        bs.num_samples,
        axis.kind.format(ps.total),
        ps.num_samples,
        axis.kind.format_signed(total_delta),
        pct(total_delta, bs.total),
    );
    if axis.secondary_idx.is_some() {
        let d2 = ps.total2 - bs.total2;
        println!(
            "Total allocations: {} -> {} | Δ {:+} ({:+.1}%)",
            bs.total2,
            ps.total2,
            d2,
            pct(d2, bs.total2),
        );
    }
    println!();

    let mut keys: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for k in bs.stats.keys() {
        keys.insert(k.as_str());
    }
    for k in ps.stats.keys() {
        keys.insert(k.as_str());
    }
    let rows: Vec<(String, i64, i64, i64)> = keys
        .into_iter()
        .map(|k| {
            let b = bs.stats.get(k).map(|s| s.self_v).unwrap_or(0);
            let p = ps.stats.get(k).map(|s| s.self_v).unwrap_or(0);
            (k.to_string(), b, p, p - b)
        })
        .collect();

    let mut improvements: Vec<_> = rows
        .iter()
        .filter(|(_, b, p, dx)| *dx < 0 && *b > 0 && *p > 0)
        .cloned()
        .collect();
    improvements.sort_by_key(|r| r.3);
    print_diff_rows("Top improvements (Δself, largest decrease first)", &improvements, axis, 15);

    let mut regressions: Vec<_> = rows
        .iter()
        .filter(|(_, b, p, dx)| *dx > 0 && *b > 0 && *p > 0)
        .cloned()
        .collect();
    regressions.sort_by(|a, b| b.3.cmp(&a.3));
    print_diff_rows("Top regressions (Δself, largest increase first)", &regressions, axis, 10);

    let mut gone: Vec<_> = rows.iter().filter(|(_, b, p, _)| *b > 0 && *p == 0).cloned().collect();
    gone.sort_by(|a, b| b.1.cmp(&a.1));
    print_disappeared_rows("Disappeared in patched (function only in base)", &gone, axis, bs.total, 10);

    let mut novel: Vec<_> = rows.iter().filter(|(_, b, p, _)| *b == 0 && *p > 0).cloned().collect();
    novel.sort_by(|a, b| b.2.cmp(&a.2));
    print_appeared_rows("New in patched (function only in patched)", &novel, axis, 10);

    Ok(())
}

fn print_diff_rows(title: &str, rows: &[(String, i64, i64, i64)], axis: &ValueAxis, n: usize) {
    println!("{}", title);
    println!("{}", "-".repeat(title.len()));
    if rows.is_empty() {
        println!("  (none)");
        println!();
        return;
    }
    let w = axis.kind.width();
    for (name, base, patched, dx) in rows.iter().take(n) {
        let pct_change = if *base > 0 {
            *dx as f64 / *base as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "  {:>w$}  {:>+6.1}%  {:<50} ({} -> {})",
            axis.kind.format_signed(*dx),
            pct_change,
            name,
            axis.kind.format(*base),
            axis.kind.format(*patched),
            w = w,
        );
    }
    println!();
}

fn print_disappeared_rows(
    title: &str,
    rows: &[(String, i64, i64, i64)],
    axis: &ValueAxis,
    base_total: i64,
    n: usize,
) {
    println!("{}", title);
    println!("{}", "-".repeat(title.len()));
    if rows.is_empty() {
        println!("  (none)");
        println!();
        return;
    }
    let w = axis.kind.width();
    for (name, base, _, _) in rows.iter().take(n) {
        println!(
            "  {:>w$}  was {:>5.1}% of base   {}",
            axis.kind.format(*base),
            pct(*base, base_total),
            name,
            w = w,
        );
    }
    println!();
}

fn print_appeared_rows(
    title: &str,
    rows: &[(String, i64, i64, i64)],
    axis: &ValueAxis,
    n: usize,
) {
    println!("{}", title);
    println!("{}", "-".repeat(title.len()));
    if rows.is_empty() {
        println!("  (none)");
        println!();
        return;
    }
    let w = axis.kind.width();
    for (name, _, patched, _) in rows.iter().take(n) {
        println!(
            "  {:>w$}                       {}",
            axis.kind.format(*patched),
            name,
            w = w,
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_prefers_delay_time_axis_for_blocking_profiles() {
        let profile = Profile {
            sample_type: vec![
                firefox_to_pprof::proto::ValueType { r#type: 1, unit: 2 },
                firefox_to_pprof::proto::ValueType { r#type: 3, unit: 4 },
            ],
            sample: vec![],
            mapping: vec![],
            location: vec![],
            function: vec![],
            string_table: ["", "samples", "count", "delay", "microseconds"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            drop_frames: 0,
            keep_frames: 0,
            time_nanos: 0,
            duration_nanos: 0,
            period_type: None,
            period: 0,
            comment: vec![],
            default_sample_type: 3,
            doc_url: 0,
        };

        let axis = value_axis(&profile);
        assert_eq!(axis.idx, 1);
        match axis.kind {
            AxisKind::Time { div, label } => {
                assert_eq!(div, 1e3);
                assert_eq!(label, "ms");
            }
            AxisKind::Bytes | AxisKind::Count(_) => panic!("expected time axis"),
        }
    }
}
