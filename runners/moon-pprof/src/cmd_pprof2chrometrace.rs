//! `moon-pprof pprof2chrometrace <in.pb.gz> <out.json>` — convert pprof
//! into synthetic Chrome trace-event JSON containing V8 CPU profiler
//! `Profile` / `ProfileChunk` events.

use std::fs;
use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use firefox_to_pprof::proto::Profile;
use pprof_to_chrome_trace::{convert_profile, ConvertOptions};
use prost::Message as _;

#[derive(Parser, Debug)]
#[command(about = "Convert pprof into synthetic Chrome trace-event JSON.")]
pub struct Args {
    /// Path to input pprof protobuf. gzip-compressed `.pb.gz` and raw
    /// `.pb` are both accepted.
    pub input: PathBuf,
    /// Path to output Chrome trace-event JSON.
    pub output: PathBuf,
    /// pprof sample value index to use as elapsed time. Defaults to the
    /// first CPU/wall time sample type.
    #[arg(long)]
    pub value_index: Option<usize>,
    /// pprof sample value index to use as sample count when
    /// `--expand-samples` is set. Defaults to `samples/count` if present.
    #[arg(long)]
    pub count_index: Option<usize>,
    /// Expand `samples/count` into repeated V8 samples. This preserves
    /// sample counts if the trace is converted back to pprof, but can
    /// produce much larger JSON.
    #[arg(long)]
    pub expand_samples: bool,
    /// Synthetic process id in the trace.
    #[arg(long, default_value_t = 1)]
    pub pid: i64,
    /// Synthetic thread id in the trace.
    #[arg(long, default_value_t = 1)]
    pub tid: i64,
    /// Synthetic V8 profile id.
    #[arg(long, default_value = "0x1")]
    pub profile_id: String,
}

pub fn run(args: Args) -> Result<()> {
    let profile = load_profile(&args.input)?;
    let out = convert_profile(
        &profile,
        &ConvertOptions {
            value_index: args.value_index,
            count_index: args.count_index,
            expand_samples: args.expand_samples,
            pid: args.pid,
            tid: args.tid,
            profile_id: args.profile_id,
        },
    )
    .with_context(|| format!("converting pprof at {}", args.input.display()))?;

    fs::write(&args.output, out.json)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!(
        "[pprof2chrometrace] {} nodes, {} samples, {:.2} ms → {}",
        out.stats.nodes,
        out.stats.samples,
        out.stats.total_delta_us as f64 / 1000.0,
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
