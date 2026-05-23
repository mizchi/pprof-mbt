//! `moon-pprof cpuprofile2pprof <in> <out>` — convert a Node V8
//! `.cpuprofile` into gzip'd pprof via the `cpuprofile-to-pprof` crate.
//!
//! Replaces the standalone `runners/v8/cpuprofile-to-pprof.mjs`. The JS
//! wrapper still exists to write the .cpuprofile under Node's inspector;
//! pprof construction now lives in Rust.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use cpuprofile_to_pprof::{Builder, CpuProfile};

#[derive(Parser, Debug)]
#[command(about = "Convert a Node V8 .cpuprofile into gzip'd pprof.")]
pub struct Args {
    /// Path to the input `.cpuprofile` (V8 Inspector `Profiler.Profile` JSON).
    pub input: PathBuf,
    /// Path to the output `.pb.gz`.
    pub output: PathBuf,
    /// Disable MoonBit symbol demangling (pass raw V8 function names through).
    #[arg(long)]
    pub no_demangle: bool,
    /// Override the pprof Mapping's filename. Defaults to empty.
    #[arg(long)]
    pub mapping_filename: Option<String>,
}

pub fn run(args: Args) -> Result<()> {
    let bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let profile: CpuProfile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing cpuprofile JSON at {}", args.input.display()))?;

    let mut builder = Builder::new(profile);
    if args.no_demangle {
        builder = builder.demangle_with(|s: &str| s.to_string());
    }
    if let Some(name) = args.mapping_filename {
        builder = builder.mapping_filename(name);
    }
    let out = builder.encode()?;
    fs::write(&args.output, &out.encoded)
        .with_context(|| format!("writing {}", args.output.display()))?;

    eprintln!(
        "[cpuprofile2pprof] {} samples, {} funcs, {} locs → {}",
        out.stats.samples,
        out.stats.functions,
        out.stats.locations,
        args.output.display(),
    );
    Ok(())
}
