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
    let pprof = perf_to_pprof::convert(&input, &opts)?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&args.out, &pprof)
        .with_context(|| format!("writing pprof to {}", args.out.display()))?;
    eprintln!(
        "[moon-pprof perf2pprof] {} → {}",
        args.input.display(),
        args.out.display()
    );
    Ok(())
}
