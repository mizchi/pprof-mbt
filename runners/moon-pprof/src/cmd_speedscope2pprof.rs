//! `moon-pprof speedscope2pprof <in.json> <out.pb.gz>` — convert a
//! Speedscope sampled profile into gzip'd pprof.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use clap::Parser;
use pprof_stack_formats::{speedscope_to_pprof, SpeedscopeImportOptions};

#[derive(Parser, Debug)]
#[command(about = "Convert Speedscope sampled JSON into gzip'd pprof.")]
pub struct Args {
    /// Path to input Speedscope JSON. `.gz` is decompressed automatically.
    pub input: PathBuf,
    /// Path to output gzip'd pprof.
    pub output: PathBuf,
    /// Which Speedscope profile to import.
    #[arg(long, default_value_t = 0)]
    pub profile_index: usize,
    /// Override the pprof Mapping filename.
    #[arg(long, default_value = "speedscope")]
    pub mapping_filename: String,
}

pub fn run(args: Args) -> Result<()> {
    let bytes =
        read_maybe_gz(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let json = String::from_utf8(bytes)
        .with_context(|| format!("input is not UTF-8 JSON: {}", args.input.display()))?;
    let out = speedscope_to_pprof(
        &json,
        &SpeedscopeImportOptions {
            profile_index: args.profile_index,
            mapping_filename: args.mapping_filename,
        },
    )
    .with_context(|| format!("converting Speedscope JSON at {}", args.input.display()))?;
    fs::write(&args.output, out.encoded)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!(
        "[speedscope2pprof] {} samples, {} locs, total={} → {}",
        out.stats.samples,
        out.stats.locations,
        out.stats.total,
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
