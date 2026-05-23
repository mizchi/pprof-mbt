//! `moon-pprof bench` — drive moonbit benches across (baseline, patched)
//! toolchains × (wasm, wasm-gc, js, native) and emit a markdown delta
//! table suitable for pasting into a PR description.
//!
//! Supports two orthogonal baseline/patched axes:
//!   - core toolchain swap (--baseline-moon / --patched-moon, sets
//!     MOON_TOOLCHAIN_ROOT) — for moonbitlang/core patches.
//!   - mooncakes swap (--mooncakes-baseline / --mooncakes-patched, swaps
//!     <bench-dir>/.mooncakes) — for registry-dep patches like
//!     moonbitlang/x or moonbitlang/async.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use regex::Regex;

#[derive(Parser, Debug)]
#[command(about = "Drive moonbit benches across (baseline, patched) × backends and print a markdown delta table.")]
pub struct Args {
    /// Path to baseline moonbit toolchain root
    #[arg(long, default_value_t = default_home(".moon"))]
    pub baseline_moon: String,

    /// Path to patched moonbit toolchain root (falls back to baseline if missing)
    #[arg(long, default_value = "/tmp/moonbit-patched")]
    pub patched_moon: String,

    /// Path to a .mooncakes snapshot to use as baseline (registry-dep swap mode)
    #[arg(long, default_value = "")]
    pub mooncakes_baseline: String,

    /// Path to a .mooncakes snapshot to use as patched (registry-dep swap mode)
    #[arg(long, default_value = "")]
    pub mooncakes_patched: String,

    /// Path to bench workspace (containing cmd/<workload>)
    #[arg(long, default_value = "./bench")]
    pub bench_dir: String,

    /// Path to runners (run-wasm-gc.mjs etc)
    #[arg(long, default_value = "./runners")]
    pub runner_dir: String,

    /// Comma-separated workload names; defaults to every dir under bench/cmd
    #[arg(long, default_value = "")]
    pub workloads: String,

    /// Comma-separated backends
    #[arg(long, default_value = "wasm,wasm-gc,js,native")]
    pub backends: String,

    /// Number of runs per (workload, backend, toolchain) cell
    #[arg(long, default_value_t = 3)]
    pub runs: usize,

    /// Build benches before running (set --build=false to reuse _build)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub build: bool,

    /// Run wasm-gc benches through Node V8 (`runners/run-wasm-gc.mjs`)
    /// instead of the default wasmtime + GuestProfiler path. Useful for
    /// reproducing V8-side numbers; the wasmtime path gives denser
    /// sampling and is the default.
    #[arg(long)]
    pub wasm_gc_via_v8: bool,
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

pub fn run(args: Args) -> Result<()> {
    let workloads: Vec<String> = if args.workloads.is_empty() {
        list_workloads(&PathBuf::from(&args.bench_dir).join("cmd"))?
    } else {
        args.workloads.split(',').map(|s| s.trim().to_string()).collect()
    };
    let backends: Vec<String> = args.backends.split(',').map(|s| s.trim().to_string()).collect();

    let mut results: BTreeMap<String, BTreeMap<String, Cell>> = BTreeMap::new();

    let mut patched_moon = args.patched_moon.clone();
    if !is_dir(&patched_moon) {
        eprintln!(
            "==> patched toolchain {} missing; using baseline for both phases",
            patched_moon
        );
        patched_moon = args.baseline_moon.clone();
    }

    let mooncakes_swap = !args.mooncakes_baseline.is_empty() || !args.mooncakes_patched.is_empty();
    if mooncakes_swap {
        if args.mooncakes_baseline.is_empty() || args.mooncakes_patched.is_empty() {
            bail!("--mooncakes-baseline and --mooncakes-patched must be set together");
        }
        if !is_dir(&args.mooncakes_baseline) {
            bail!("--mooncakes-baseline {} does not exist", args.mooncakes_baseline);
        }
        if !is_dir(&args.mooncakes_patched) {
            bail!("--mooncakes-patched {} does not exist", args.mooncakes_patched);
        }
    }

    for kind in ["baseline", "patched"] {
        let moon_root = if kind == "baseline" { &args.baseline_moon } else { &patched_moon };
        eprintln!("==> {} toolchain: {}", kind, moon_root);

        if mooncakes_swap {
            let src = if kind == "baseline" {
                &args.mooncakes_baseline
            } else {
                &args.mooncakes_patched
            };
            swap_mooncakes(&args.bench_dir, src).with_context(|| format!("mooncakes swap ({})", kind))?;
            eprintln!("==> {} mooncakes: {} -> {}/.mooncakes", kind, src, args.bench_dir);
        }

        if args.build {
            build_all(&args, moon_root, &workloads, &backends)
                .with_context(|| format!("build ({})", kind))?;
        }

        for w in &workloads {
            let entry = results.entry(w.clone()).or_default();
            for b in &backends {
                let mut times: Vec<f64> = Vec::with_capacity(args.runs);
                for r in 0..args.runs {
                    match run_once(&args, w, b) {
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

fn build_all(args: &Args, moon_root: &str, workloads: &[String], backends: &[String]) -> Result<()> {
    let moon_bin = PathBuf::from(moon_root).join("bin").join("moon");
    let bin_path = format!("{}/bin", moon_root);
    let new_path = match env::var("PATH") {
        Ok(p) => format!("{}:{}", bin_path, p),
        Err(_) => bin_path,
    };
    let build_dir = PathBuf::from(&args.bench_dir).join("_build");
    let _ = std::fs::remove_dir_all(&build_dir);

    for w in workloads {
        for b in backends {
            let mut cmd_args = vec!["build".to_string(), "--release".to_string()];
            if b == "wasm" || b == "wasm-gc" {
                cmd_args.push("--no-strip".to_string());
            }
            cmd_args.push(format!("--target={}", b));
            cmd_args.push(format!("cmd/{}", w));
            let out = Command::new(&moon_bin)
                .args(&cmd_args)
                .current_dir(&args.bench_dir)
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

fn run_once(args: &Args, workload: &str, backend: &str) -> Result<f64> {
    match backend {
        "wasm" => run_wasm(args, workload),
        "wasm-gc" => {
            if args.wasm_gc_via_v8 {
                run_wasm_gc_v8(args, workload)
            } else {
                run_wasm_gc(args, workload)
            }
        }
        "js" => run_js(args, workload),
        "native" => run_native(args, workload),
        other => Err(anyhow!("unknown backend {:?}", other)),
    }
}

fn parse_ms(s: &str) -> Result<f64> {
    let re = Regex::new(r"([0-9]+\.[0-9]+|[0-9]+)\s*ms").unwrap();
    let m = re.captures(s).ok_or_else(|| anyhow!("no ms value in: {}", s.trim()))?;
    Ok(m[1].parse()?)
}

/// Path to the moon-pprof binary currently running, used to spawn the
/// `profile` subcommand for wasm benches without needing a separate
/// wasmtime-runner.
fn self_path() -> Result<PathBuf> {
    env::current_exe().context("locating moon-pprof binary path")
}

fn run_wasm(args: &Args, w: &str) -> Result<f64> {
    let path = PathBuf::from(&args.bench_dir)
        .join("_build")
        .join("wasm")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.wasm", w));
    let bin = self_path()?;
    let out = Command::new(&bin)
        .args(["profile", "--no-profile"])
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        bail!(
            "moon-pprof profile {:?}: {}\n{}{}",
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

fn wasm_gc_path(args: &Args, w: &str) -> PathBuf {
    PathBuf::from(&args.bench_dir)
        .join("_build")
        .join("wasm-gc")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.wasm", w))
}

/// Default wasm-gc path: spawn `moon-pprof profile --no-profile --wasm-gc`
/// (wasmtime + Cranelift). No Node dependency.
fn run_wasm_gc(args: &Args, w: &str) -> Result<f64> {
    let path = wasm_gc_path(args, w);
    let bin = self_path()?;
    let out = Command::new(&bin)
        .args(["profile", "--no-profile", "--wasm-gc"])
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        bail!(
            "moon-pprof profile --wasm-gc {:?}: {}\n{}{}",
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

/// Legacy wasm-gc path: spawn Node `runners/run-wasm-gc.mjs --no-profile`
/// (V8). Kept behind --wasm-gc-via-v8 for reproducing V8-side numbers.
fn run_wasm_gc_v8(args: &Args, w: &str) -> Result<f64> {
    let path = wasm_gc_path(args, w);
    let script = PathBuf::from(&args.runner_dir).join("run-wasm-gc.mjs");
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

fn run_js(args: &Args, w: &str) -> Result<f64> {
    let path = PathBuf::from(&args.bench_dir)
        .join("_build")
        .join("js")
        .join("release")
        .join("build")
        .join("cmd")
        .join(w)
        .join(format!("{}.js", w))
        .canonicalize()
        .with_context(|| format!("canonicalize js path for {}", w))?;
    let script = PathBuf::from(&args.runner_dir).join("run-js.mjs");
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

fn run_native(args: &Args, w: &str) -> Result<f64> {
    let bin = PathBuf::from(&args.bench_dir)
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
