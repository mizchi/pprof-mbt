//! `moon-pprof pprof2speedscope <in.pb.gz> <out.json>` — convert pprof
//! into Speedscope JSON.

use std::fs;
use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use firefox_to_pprof::proto::Profile;
use pprof_stack_formats::{pprof_to_speedscope, SpeedscopeExportOptions};
use prost::Message as _;

#[derive(Parser, Debug)]
#[command(about = "Convert pprof into Speedscope JSON.")]
pub struct Args {
    /// Path to input pprof protobuf. gzip-compressed `.pb.gz` and raw
    /// `.pb` are both accepted.
    pub input: PathBuf,
    /// Path to output Speedscope JSON.
    pub output: PathBuf,
    /// pprof sample value index to export. Defaults to the first CPU/wall
    /// time sample type.
    #[arg(long)]
    pub value_index: Option<usize>,
    /// File-level Speedscope name.
    #[arg(long, default_value = "pprof")]
    pub name: String,
    /// Profile-level Speedscope name.
    #[arg(long, default_value = "pprof")]
    pub profile_name: String,
}

pub fn run(args: Args) -> Result<()> {
    let profile = load_profile(&args.input)?;
    let (json, stats) = pprof_to_speedscope(
        &profile,
        &SpeedscopeExportOptions {
            value_index: args.value_index,
            name: args.name,
            profile_name: args.profile_name,
        },
    )
    .with_context(|| format!("converting pprof at {}", args.input.display()))?;
    fs::write(&args.output, json).with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!(
        "[pprof2speedscope] {} stacks, total={} → {}",
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
