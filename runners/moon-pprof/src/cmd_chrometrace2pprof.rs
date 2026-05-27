//! `moon-pprof chrometrace2pprof <in> <out>` — convert Chrome
//! trace-event JSON containing V8 CPU profiler `Profile` /
//! `ProfileChunk` events into gzip'd pprof.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use chrome_trace_to_pprof::{convert, ConvertOptions};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Convert Chrome trace-event JSON with V8 CPU profile chunks into gzip'd pprof.")]
pub struct Args {
    /// Path to the input Chrome trace-event JSON. `.gz` is decompressed
    /// automatically.
    pub input: PathBuf,
    /// Path to the output `.pb.gz`.
    pub output: PathBuf,
    /// Which embedded V8 CPU profile stream to convert when the trace
    /// contains multiple `(pid, tid, id)` streams.
    #[arg(long, default_value_t = 0)]
    pub profile_index: usize,
    /// Disable MoonBit symbol demangling.
    #[arg(long)]
    pub no_demangle: bool,
    /// Override the pprof Mapping's filename. Defaults to empty.
    #[arg(long)]
    pub mapping_filename: Option<String>,
    /// Fallback interval in microseconds for traces that omit
    /// `timeDeltas`.
    #[arg(long, default_value_t = 1000)]
    pub default_sample_delta_us: i64,
}

pub fn run(args: Args) -> Result<()> {
    let bytes =
        read_maybe_gz(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let json = String::from_utf8(bytes)
        .with_context(|| format!("input is not UTF-8 JSON: {}", args.input.display()))?;
    let opts = ConvertOptions {
        profile_index: args.profile_index,
        no_demangle: args.no_demangle,
        mapping_filename: args.mapping_filename,
        default_sample_delta_us: args.default_sample_delta_us,
    };
    let out = convert(&json, &opts)
        .with_context(|| format!("converting Chrome trace at {}", args.input.display()))?;

    fs::write(&args.output, &out.encoded)
        .with_context(|| format!("writing {}", args.output.display()))?;

    let selected = &out.profiles[args.profile_index];
    eprintln!(
        "[chrometrace2pprof] profile {}/{} {}: {} raw samples, {} pprof samples, {} funcs, {} locs → {}",
        args.profile_index + 1,
        out.profiles.len(),
        selected.key,
        selected.samples,
        out.stats.samples,
        out.stats.functions,
        out.stats.locations,
        args.output.display(),
    );
    Ok(())
}

fn read_maybe_gz(path: &Path) -> Result<Vec<u8>> {
    let raw = fs::read(path)?;
    if path.extension().and_then(|s| s.to_str()) == Some("gz") {
        let mut buf = Vec::new();
        flate2::read::GzDecoder::new(raw.as_slice()).read_to_end(&mut buf)?;
        if buf.is_empty() {
            bail!("decompressed payload was empty");
        }
        Ok(buf)
    } else {
        Ok(raw)
    }
}
