//! `moon-pprof perf2pprof <perf-script.txt> --out cpu.pb.gz` — wrap
//! `perf-to-pprof::convert` so users have a one-shot CLI alongside the
//! other converters.

use std::fs;
use std::io::{self, Read as _};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    about = "Convert Linux `perf script` textual output into gzip'd pprof."
)]
pub struct Args {
    /// Input file containing `perf script` output. Use `-` for stdin.
    pub input: PathBuf,
    /// Output path for the gzip'd pprof.
    #[arg(long, default_value = "perf.pb.gz")]
    pub out: PathBuf,
    /// Pprof `sample_type[1]` label for the perf event. Defaults to `cpu`
    /// which matches the recommended `perf record -F 999 -e cpu-clock`
    /// recipe.
    #[arg(long, default_value = "cpu")]
    pub event_type: String,
    /// Unit label for that sample type. Defaults to `nanoseconds`.
    #[arg(long, default_value = "nanoseconds")]
    pub event_unit: String,
    /// Pass raw symbols through instead of running them through
    /// `moonbit_demangle::demangle`.
    #[arg(long)]
    pub no_demangle: bool,
}

pub fn run(args: Args) -> Result<()> {
    let input = if args.input == PathBuf::from("-") {
        let mut s = String::new();
        io::stdin()
            .read_to_string(&mut s)
            .context("reading perf script from stdin")?;
        s
    } else {
        fs::read_to_string(&args.input)
            .with_context(|| format!("reading {}", args.input.display()))?
    };

    let opts = perf_to_pprof::ConvertOptions {
        event_type: args.event_type,
        event_unit: args.event_unit,
        no_demangle: args.no_demangle,
    };

    let samples = perf_to_pprof::parse(&input)?;
    let stats = perf_to_pprof::Stats::from_samples(&samples);
    emit_warnings(&stats);

    let pprof = perf_to_pprof::convert_from_samples(samples, &opts)?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&args.out, &pprof)
        .with_context(|| format!("writing pprof to {}", args.out.display()))?;
    eprintln!(
        "[moon-pprof perf2pprof] {} → {} ({} samples, {} frames)",
        args.input.display(),
        args.out.display(),
        stats.sample_count,
        stats.frame_count,
    );
    Ok(())
}

/// Warn the user about two common capture mistakes that produce a
/// pprof you can still load but whose numbers are misleading.
fn emit_warnings(stats: &perf_to_pprof::Stats) {
    if stats.sample_count == 0 {
        eprintln!(
            "[moon-pprof perf2pprof] warning: parsed 0 samples — \
             input may not be `perf script` text, or every sample was empty"
        );
        return;
    }
    if stats.period_likely_missing() {
        eprintln!(
            "[moon-pprof perf2pprof] warning: every sample has period=1 — \
             re-capture with `perf record --weight` and `perf script \
             -F comm,pid,tid,time,event,period,ip,sym,dso` so the pprof \
             carries real wall-time units (otherwise `Total` will read as nanoseconds)"
        );
    }
    let ratio = stats.unknown_ratio();
    if ratio > 0.5 {
        eprintln!(
            "[moon-pprof perf2pprof] warning: {:.0}% of frames ({}/{}) came back as `[unknown]` — \
             ensure the recorded binary's debug info is reachable to `perf script` \
             (build with `cc -g`, run `perf script` in the same fs view that recorded, \
             or pass `--symfs=<root>` so perf can find the DSO)",
            ratio * 100.0,
            stats.unknown_frame_count,
            stats.frame_count,
        );
    }
}
