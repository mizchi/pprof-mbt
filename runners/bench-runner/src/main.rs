//! bench-runner: drive a set of moonbit benches across (baseline, patched)
//! toolchains × (wasm, wasm-gc, js, native) and emit a markdown delta
//! table suitable for pasting into a PR description.
//!
//! Also supports a "mooncakes swap" mode where the baseline/patched
//! axis is per-project `.mooncakes/` snapshots rather than the global
//! `~/.moon` toolchain. Used for `moonbitlang/x` / `moonbitlang/async`
//! style registry-dep patches.
//!
//! Rust rewrite of runners/wzprof-runner/cmd/bench-runner.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use regex::Regex;

#[derive(Parser, Debug)]
#[command(
    name = "bench-runner",
    about = "Drive moonbit benches across (baseline, patched) × backends and print a markdown delta table.",
    disable_help_flag = false,
    // Accept both `-foo` and `--foo` for parity with the Go flag package
    // version that this binary replaces.
    allow_external_subcommands = false
)]
struct Cli {
    /// Path to baseline moonbit toolchain root
    #[arg(long, default_value_t = default_home(".moon"))]
    baseline_moon: String,

    /// Path to patched moonbit toolchain root (falls back to baseline if missing)
    #[arg(long, default_value = "/tmp/moonbit-patched")]
    patched_moon: String,

    /// Path to a .mooncakes snapshot to use as baseline (registry-dep swap mode)
    #[arg(long, default_value = "")]
    mooncakes_baseline: String,

    /// Path to a .mooncakes snapshot to use as patched (registry-dep swap mode)
    #[arg(long, default_value = "")]
    mooncakes_patched: String,

    /// Path to bench workspace (containing cmd/<workload>)
    #[arg(long, default_value = "./bench")]
    bench_dir: String,

    /// Path to runners (run-wasm-gc.mjs etc)
    #[arg(long, default_value = "./runners")]
    runner_dir: String,

    /// Path to .bin (wasmtime-runner)
    #[arg(long, default_value = "./.bin")]
    bin_dir: String,

    /// Comma-separated workload names; defaults to every dir under bench/cmd
    #[arg(long, default_value = "")]
    workloads: String,

    /// Comma-separated backends
    #[arg(long, default_value = "wasm,wasm-gc,js,native")]
    backends: String,

    /// Number of runs per (workload, backend, toolchain) cell
    #[arg(long, default_value_t = 3)]
    runs: usize,

    /// Build benches before running (set --build=false to reuse _build)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    build: bool,
}

fn default_home(suffix: &str) -> String {
    let h = env::var("HOME").unwrap_or_else(|_| "/root".into());
    format!("{}/{}", h, suffix)
}

#[derive(Default, Clone, Copy)]
struct Cell {
    base_ms: f64,
    patched_ms: f64,
}

fn main() -> ExitCode {
    // Convert single-dash long options to double-dash so clap accepts them
    // (`-runs 3` → `--runs 3`). This preserves the Go-era invocation style.
    let raw: Vec<String> = env::args().collect();
    let argv = normalize_argv(raw);
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            e.print().ok();
            return ExitCode::from(2);
        }
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bench-runner: {:#}", e);
            ExitCode::FAILURE
        }
    }
}

/// Translate a single-leading-dash long option to double-dash. `-foo bar`
/// and `-foo=bar` both become `--foo bar` / `--foo=bar`. Short flags (-h)
/// stay if they're a single char, but bench-runner doesn't use any.
fn normalize_argv(argv: Vec<String>) -> Vec<String> {
    argv.into_iter()
        .map(|a| {
            if a.starts_with("--") || !a.starts_with('-') {
                return a;
            }
            // -x (1-char) — leave for clap to parse as short flag
            let inner = &a[1..];
            if inner.len() <= 1 {
                return a;
            }
            // Treat first '=' as the value separator
            format!("--{}", inner)
        })
        .collect()
}

fn run(c: Cli) -> Result<()> {
    let workloads: Vec<String> = if c.workloads.is_empty() {
        list_workloads(&PathBuf::from(&c.bench_dir).join("cmd"))?
    } else {
        c.workloads.split(',').map(|s| s.trim().to_string()).collect()
    };
    let backends: Vec<String> = c.backends.split(',').map(|s| s.trim().to_string()).collect();

    // (workload, backend) -> cell
    let mut results: BTreeMap<String, BTreeMap<String, Cell>> = BTreeMap::new();

    let mut patched_moon = c.patched_moon.clone();
    if !is_dir(&patched_moon) {
        eprintln!(
            "==> patched toolchain {} missing; using baseline for both phases",
            patched_moon
        );
        patched_moon = c.baseline_moon.clone();
    }

    let mooncakes_swap = !c.mooncakes_baseline.is_empty() || !c.mooncakes_patched.is_empty();
    if mooncakes_swap {
        if c.mooncakes_baseline.is_empty() || c.mooncakes_patched.is_empty() {
            bail!("--mooncakes-baseline and --mooncakes-patched must be set together");
        }
        if !is_dir(&c.mooncakes_baseline) {
            bail!("--mooncakes-baseline {} does not exist", c.mooncakes_baseline);
        }
        if !is_dir(&c.mooncakes_patched) {
            bail!("--mooncakes-patched {} does not exist", c.mooncakes_patched);
        }
    }

    for kind in ["baseline", "patched"] {
        let moon_root = if kind == "baseline" { &c.baseline_moon } else { &patched_moon };
        eprintln!("==> {} toolchain: {}", kind, moon_root);

        if mooncakes_swap {
            let src = if kind == "baseline" {
                &c.mooncakes_baseline
            } else {
                &c.mooncakes_patched
            };
            swap_mooncakes(&c.bench_dir, src).with_context(|| format!("mooncakes swap ({})", kind))?;
            eprintln!("==> {} mooncakes: {} -> {}/.mooncakes", kind, src, c.bench_dir);
        }

        if c.build {
            build_all(&c, moon_root, &workloads, &backends)
                .with_context(|| format!("build ({})", kind))?;
        }

        for w in &workloads {
            let entry = results.entry(w.clone()).or_default();
            for b in &backends {
                let mut times: Vec<f64> = Vec::with_capacity(c.runs);
                for r in 0..c.runs {
                    match run_once(&c, w, b) {
                        Ok(t) => times.push(t),
                        Err(e) => {
                            eprintln!("  {}/{} run {}: {:#}", w, b, r + 1, e);
                            break;
                        }
                    }
                }
                if times.is_empty() {
                    continue;
                }
                let med = median(&mut times);
                let ce = entry.entry(b.clone()).or_default();
                if kind == "baseline" {
                    ce.base_ms = med;
                } else {
                    ce.patched_ms = med;
                }
                eprintln!(
                    "  {:<22} {:<7} {} = {:.1} ms (median of {})",
                    w,
                    b,
                    kind,
                    med,
                    times.len()
                );
            }
        }
    }

    print_markdown(&backends, &results);
    Ok(())
}

fn list_workloads(cmd_dir: &Path) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for e in std::fs::read_dir(cmd_dir).with_context(|| format!("read {:?}", cmd_dir))? {
        let e = e?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        // Skip "main" since it's just the generic startup workload.
        if name == "main" {
            continue;
        }
        out.push(name);
    }
    out.sort();
    Ok(out)
}

fn is_dir(p: &str) -> bool {
    if p.is_empty() {
        return false;
    }
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn swap_mooncakes(bench_dir: &str, src: &str) -> Result<()> {
    let dst = PathBuf::from(bench_dir).join(".mooncakes");
    if dst.exists() {
        std::fs::remove_dir_all(&dst).with_context(|| format!("rm {:?}", dst))?;
    }
    let status = Command::new("cp").args(["-r", src, dst.to_str().unwrap()]).status()?;
    if !status.success() {
        bail!("cp -r {} {:?} failed: {}", src, dst, status);
    }
    let status = Command::new("chmod")
        .args(["-R", "u+w", dst.to_str().unwrap()])
        .status()?;
    if !status.success() {
        bail!("chmod -R u+w {:?} failed: {}", dst, status);
    }
    Ok(())
}

fn build_all(c: &Cli, moon_root: &str, workloads: &[String], backends: &[String]) -> Result<()> {
    let moon_bin = PathBuf::from(moon_root).join("bin").join("moon");
    let bin_path = format!("{}/bin", moon_root);
    let new_path = match env::var("PATH") {
        Ok(p) => format!("{}:{}", bin_path, p),
        Err(_) => bin_path,
    };
    let build_dir = PathBuf::from(&c.bench_dir).join("_build");
    let _ = std::fs::remove_dir_all(&build_dir);

    for w in workloads {
        for b in backends {
            let mut args = vec!["build".to_string(), "--release".to_string()];
            if b == "wasm" || b == "wasm-gc" {
                args.push("--no-strip".to_string());
            }
            args.push(format!("--target={}", b));
            args.push(format!("cmd/{}", w));
            let out = Command::new(&moon_bin)
                .args(&args)
                .current_dir(&c.bench_dir)
                .env("PATH", &new_path)
                .env("MOON_TOOLCHAIN_ROOT", moon_root)
                .output()
                .with_context(|| format!("moon build {}/{}", w, b))?;
            if !out.status.success() {
                bail!(
                    "moon build {}/{}: {}\n{}",
                    w,
                    b,
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
    Ok(())
}

fn run_once(c: &Cli, workload: &str, backend: &str) -> Result<f64> {
    match backend {
        "wasm" => run_wasm(c, workload),
        "wasm-gc" => run_wasm_gc(c, workload),
        "js" => run_js(c, workload),
        "native" => run_native(c, workload),
        other => Err(anyhow!("unknown backend {:?}", other)),
    }
}

fn parse_ms(s: &str) -> Result<f64> {
    let re = Regex::new(r"([0-9]+\.[0-9]+|[0-9]+)\s*ms").unwrap();
    let m = re
        .captures(s)
        .ok_or_else(|| anyhow!("no ms value in: {}", s.trim()))?;
    Ok(m[1].parse()?)
}

fn run_wasm(c: &Cli, w: &str) -> Result<f64> {
    let bin = PathBuf::from(&c.bin_dir).join("wasmtime-runner");
    let path = PathBuf::from(&c.bench_dir)
        .join("_build")
        .join("wasm")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.wasm", w));
    let out = Command::new(&bin)
        .args(["--no-profile"])
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        bail!(
            "wasmtime-runner {:?}: {}\n{}{}",
            path,
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_ms(&combined)
}

fn run_wasm_gc(c: &Cli, w: &str) -> Result<f64> {
    let path = PathBuf::from(&c.bench_dir)
        .join("_build")
        .join("wasm-gc")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.wasm", w));
    let script = PathBuf::from(&c.runner_dir).join("run-wasm-gc.mjs");
    let out = Command::new("node")
        .arg(&script)
        .arg("--no-profile")
        .arg(&path)
        .output()?;
    if !out.status.success() {
        bail!(
            "run-wasm-gc {:?}: {}\n{}{}",
            path,
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_ms(&combined)
}

fn run_js(c: &Cli, w: &str) -> Result<f64> {
    let path = PathBuf::from(&c.bench_dir)
        .join("_build")
        .join("js")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.js", w))
        .canonicalize()
        .with_context(|| format!("canonicalize js path for {}", w))?;
    let script = PathBuf::from(&c.runner_dir).join("run-js.mjs");
    let out = Command::new("node")
        .arg(&script)
        .arg("--no-profile")
        .arg(&path)
        .output()?;
    if !out.status.success() {
        bail!(
            "run-js {:?}: {}\n{}{}",
            path,
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_ms(&combined)
}

fn run_native(c: &Cli, w: &str) -> Result<f64> {
    let bin = PathBuf::from(&c.bench_dir)
        .join("_build")
        .join("native")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.exe", w));
    let start = Instant::now();
    let status = Command::new(&bin).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    if !status.success() {
        bail!("native {:?}: {}", bin, status);
    }
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs[xs.len() / 2]
}

fn print_markdown(backends: &[String], results: &BTreeMap<String, BTreeMap<String, Cell>>) {
    println!();
    println!("## Results");
    println!();
    print!("| workload |");
    for b in backends {
        print!(" {} base | {} patched | Δ |", b, b);
    }
    println!();
    print!("|---|");
    for _ in backends {
        print!("--:|--:|--:|");
    }
    println!();
    for (w, by_backend) in results {
        print!("| {} |", w);
        for b in backends {
            let ce = by_backend.get(b).copied().unwrap_or_default();
            if ce.base_ms == 0.0 && ce.patched_ms == 0.0 {
                print!(" - | - | - |");
                continue;
            }
            let delta = if ce.base_ms > 0.0 {
                let d = (ce.patched_ms - ce.base_ms) / ce.base_ms * 100.0;
                format!("{:+.1}%", d)
            } else {
                "".to_string()
            };
            print!(" {:.1} | {:.1} | {} |", ce.base_ms, ce.patched_ms, delta);
        }
        println!();
    }
}
