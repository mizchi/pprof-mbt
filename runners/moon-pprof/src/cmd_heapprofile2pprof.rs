//! `moon-pprof heapprofile2pprof <in> <out>` — convert a Node V8
//! `.heapprofile` (sampling allocation profile, written by
//! `node --heap-prof` or `runners/v8/run-js-heap.mjs`) into gzip'd
//! pprof via the `heapprofile-to-pprof` crate.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use heapprofile_to_pprof::{Builder, HeapProfile};

#[derive(Parser, Debug)]
#[command(about = "Convert a Node V8 .heapprofile into gzip'd pprof.")]
pub struct Args {
    /// Path to the input `.heapprofile` (V8 Inspector
    /// `HeapProfiler.SamplingHeapProfile` JSON).
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
    let profile: HeapProfile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing heapprofile JSON at {}", args.input.display()))?;

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
        "[heapprofile2pprof] {} samples, {} funcs, {} locs, {} objects, {} bytes → {}",
        out.stats.samples,
        out.stats.functions,
        out.stats.locations,
        out.stats.total_objects,
        out.stats.total_bytes,
        args.output.display(),
    );
    Ok(())
}
