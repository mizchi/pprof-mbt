//! `moon-pprof folded2pprof <in.folded> <out.pb.gz>` — convert folded
//! stack text into gzip'd pprof.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use clap::Parser;
use pprof_stack_formats::{folded_to_pprof, FoldedImportOptions};

#[derive(Parser, Debug)]
#[command(about = "Convert folded stack text into gzip'd pprof.")]
pub struct Args {
    /// Path to input folded stack text. `.gz` is decompressed automatically.
    pub input: PathBuf,
    /// Path to output gzip'd pprof.
    pub output: PathBuf,
    /// pprof sample type for the folded value column.
    #[arg(long, default_value = "delay")]
    pub sample_type: String,
    /// pprof unit for the folded value column.
    #[arg(long, default_value = "microseconds")]
    pub unit: String,
    /// Override the pprof Mapping filename.
    #[arg(long, default_value = "folded")]
    pub mapping_filename: String,
}

pub fn run(args: Args) -> Result<()> {
    let bytes =
        read_maybe_gz(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let text = String::from_utf8(bytes)
        .with_context(|| format!("input is not UTF-8 folded text: {}", args.input.display()))?;
    let out = folded_to_pprof(
        &text,
        &FoldedImportOptions {
            sample_type: args.sample_type,
            unit: args.unit,
            mapping_filename: args.mapping_filename,
        },
    )
    .with_context(|| format!("converting folded stacks at {}", args.input.display()))?;
    fs::write(&args.output, out.encoded)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!(
        "[folded2pprof] {} samples, {} locs, total={} -> {}",
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
