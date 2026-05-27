//! `moon-pprof pprof2folded <in.pb.gz> <out.folded>` — convert pprof
//! into folded stack text (`root;child;leaf value`).

use std::fs;
use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use firefox_to_pprof::proto::Profile;
use pprof_stack_formats::{pprof_to_folded, PprofExportOptions};
use prost::Message as _;

#[derive(Parser, Debug)]
#[command(about = "Convert pprof into folded stack text.")]
pub struct Args {
    /// Path to input pprof protobuf. gzip-compressed `.pb.gz` and raw
    /// `.pb` are both accepted.
    pub input: PathBuf,
    /// Path to output folded stack text.
    pub output: PathBuf,
    /// pprof sample value index to export. Defaults to the first CPU/wall
    /// time sample type.
    #[arg(long)]
    pub value_index: Option<usize>,
}

pub fn run(args: Args) -> Result<()> {
    let profile = load_profile(&args.input)?;
    let (folded, stats) = pprof_to_folded(
        &profile,
        &PprofExportOptions {
            value_index: args.value_index,
        },
    )
    .with_context(|| format!("converting pprof at {}", args.input.display()))?;
    fs::write(&args.output, folded)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!(
        "[pprof2folded] {} stacks, total={} → {}",
        stats.stacks,
        stats.total,
        args.output.display(),
    );
    Ok(())
}

fn load_profile(path: &PathBuf) -> Result<Profile> {
    let raw = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let buf = if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut dec = flate2::read::GzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        dec.read_to_end(&mut out).context("gunzip pprof")?;
        out
    } else {
        raw
    };
    Profile::decode(buf.as_slice()).context("decode pprof protobuf")
}
