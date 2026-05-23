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

struct ValueAxis {
    idx: usize,
    unit: &'static str,
    div: f64,
}

fn value_axis(p: &Profile) -> ValueAxis {
    for (i, st) in p.sample_type.iter().enumerate() {
        let ty = string_at(p, st.r#type);
        let unit = string_at(p, st.unit);
        if ty == "cpu" || ty == "wall" {
            let (label, div) = match unit {
                "nanoseconds" => ("ms", 1e6),
                "microseconds" => ("ms", 1e3),
                "milliseconds" => ("ms", 1.0),
                "count" => ("samples", 1.0),
                _ => continue,
            };
            return ValueAxis { idx: i, unit: label, div };
        }
    }
    ValueAxis { idx: 0, unit: "ns", div: 1.0 }
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
}

struct Summary {
    total: i64,
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
    let mut stats: HashMap<String, FuncStats> = HashMap::new();
    let mut total: i64 = 0;
    let mut mem_mgmt_total: i64 = 0;

    for sample in &p.sample {
        let v = sample.value.get(axis.idx).copied().unwrap_or(0);
        total += v;

        let leaf = sample
            .location_id
            .first()
            .and_then(|id| loc_to_name.get(id))
            .map(|s| s.as_str())
            .unwrap_or("(unknown)");

        let leaf_is_mem = mem_mgmt.is_match(leaf);
        stats.entry(leaf.to_string()).or_default().self_v += v;
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
    for (name, v) in rows.iter().take(n) {
        if *v == 0 {
            break;
        }
        println!(
            "  {:>7.2} {}  {:>5.1}%  {}",
            *v as f64 / axis.div,
            axis.unit,
            pct(*v, total),
            name
        );
    }
    println!();
}

fn run_single(path: &PathBuf, mem_mgmt: &Regex) -> Result<()> {
    let p = load_profile(path)?;
    let s = compute_summary(&p, mem_mgmt);

    println!("Profile: {}", path.display());
    println!(
        "Total {}: {:.2} ({} samples)",
        s.axis.unit,
        s.total as f64 / s.axis.div,
        s.num_samples
    );
    println!(
        "Memory-management self time: {:.2} {} ({:.1}%)",
        s.mem_mgmt_total as f64 / s.axis.div,
        s.axis.unit,
        pct(s.mem_mgmt_total, s.total)
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

    if bs.axis.unit != ps.axis.unit || bs.axis.div != ps.axis.div {
        return Err(anyhow!(
            "base / patched use different time units ({} vs {})",
            bs.axis.unit, ps.axis.unit
        ));
    }
    let axis = &bs.axis;

    println!("Profile diff:");
    println!("  base    = {}", base_path.display());
    println!("  patched = {}", patched_path.display());
    let total_delta = ps.total - bs.total;
    println!();
    println!(
        "Total {}: {:.2} ({} samples) -> {:.2} ({} samples) | Δ {:+.2} {} ({:+.1}%)",
        axis.unit,
        bs.total as f64 / axis.div,
        bs.num_samples,
        ps.total as f64 / axis.div,
        ps.num_samples,
        total_delta as f64 / axis.div,
        axis.unit,
        pct(total_delta, bs.total)
    );
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
    for (name, base, patched, dx) in rows.iter().take(n) {
        let pct_change = if *base > 0 {
            *dx as f64 / *base as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "  {:>+9.2} {}  {:>+6.1}%  {:<50} ({:.2} -> {:.2})",
            *dx as f64 / axis.div,
            axis.unit,
            pct_change,
            name,
            *base as f64 / axis.div,
            *patched as f64 / axis.div,
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
    for (name, base, _, _) in rows.iter().take(n) {
        println!(
            "  {:>9.2} {}  was {:>5.1}% of base   {}",
            *base as f64 / axis.div,
            axis.unit,
            pct(*base, base_total),
            name
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
    for (name, _, patched, _) in rows.iter().take(n) {
        println!(
            "  {:>9.2} {}                       {}",
            *patched as f64 / axis.div,
            axis.unit,
            name
        );
    }
    println!();
}
