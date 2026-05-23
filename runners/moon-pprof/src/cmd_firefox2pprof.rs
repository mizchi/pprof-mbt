//! `moon-pprof firefox2pprof <in> <out>` — convert a Firefox Profiler
//! "processed profile" JSON (samply or wasmtime GuestProfiler) into
//! gzip'd pprof.
//!
//! Replaces `runners/samply-to-pprof.mjs` and `runners/wasmtime-to-pprof.mjs`.
//! The wasmtime mode reuses [`firefox_to_pprof::FuncTableResolver`];
//! samply mode pairs the profile with its `.syms.json` sidecar via
//! [`firefox_to_pprof::samply::SamplySyms`].

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, ValueEnum};
use firefox_to_pprof::{
    Builder, FirefoxProfile, FuncTableResolver, SampleWeighting,
    samply::SamplySyms,
};

#[derive(Parser, Debug)]
#[command(about = "Convert a Firefox Profiler 'processed profile' JSON into gzip'd pprof.")]
pub struct Args {
    /// Path to the input Firefox-format JSON. `.gz` is decompressed
    /// automatically.
    pub input: PathBuf,
    /// Path to the output `.pb.gz`.
    pub output: PathBuf,
    /// Producer the input came from. `samply` reads raw RVAs from
    /// `frameTable.address` and pairs them with a `.syms.json` sidecar;
    /// `wasmtime-guest` reads the pre-resolved funcTable directly.
    #[arg(long, value_enum, default_value_t = Source::WasmtimeGuest)]
    pub source: Source,
    /// `.syms.json` sidecar path (required for `--source samply`).
    /// Defaults to `<input>.syms.json` (with any `.gz` extension stripped).
    #[arg(long)]
    pub syms: Option<PathBuf>,
    /// Disable MoonBit symbol demangling.
    #[arg(long)]
    pub no_demangle: bool,
    /// Override the pprof Mapping's filename.
    #[arg(long)]
    pub mapping_filename: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Source {
    /// samply native sampler (RVAs + `.syms.json` sidecar).
    Samply,
    /// wasmtime `GuestProfiler` (funcTable pre-resolved).
    WasmtimeGuest,
}

pub fn run(args: Args) -> Result<()> {
    let bytes = read_maybe_gz(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let profile: FirefoxProfile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing Firefox JSON at {}", args.input.display()))?;

    let interval_ns = (profile.meta.interval.max(1.0) * 1_000_000.0).round() as i64;

    let bytes_out = match args.source {
        Source::WasmtimeGuest => {
            let mut b = Builder::new(
                &profile,
                FuncTableResolver,
                SampleWeighting::PerSampleTimeDeltas {
                    default_interval_ns: interval_ns,
                },
            );
            if args.no_demangle {
                b = b.demangle_with(|s| s.to_string());
            }
            if let Some(name) = args.mapping_filename {
                b = b.mapping_filename(name);
            }
            b.encode()?
        }
        Source::Samply => {
            let syms_path = match args.syms {
                Some(p) => p,
                None => default_syms_path(&args.input),
            };
            let syms_bytes = fs::read(&syms_path)
                .with_context(|| format!("reading samply .syms.json at {}", syms_path.display()))?;
            let resolver = SamplySyms::load(&syms_bytes)?.into_resolver();
            let mut b = Builder::new(
                &profile,
                resolver,
                SampleWeighting::FixedInterval { interval_ns },
            );
            if args.no_demangle {
                b = b.demangle_with(|s| s.to_string());
            }
            if let Some(name) = args.mapping_filename {
                b = b.mapping_filename(name);
            }
            b.encode()?
        }
    };

    fs::write(&args.output, &bytes_out)
        .with_context(|| format!("writing {}", args.output.display()))?;
    eprintln!(
        "[firefox2pprof:{:?}] {} bytes → {}",
        args.source,
        bytes_out.len(),
        args.output.display(),
    );
    Ok(())
}

/// Default companion sidecar path: strip any trailing `.gz` from the
/// input filename and append `.syms.json`. e.g. `native-samply.json.gz`
/// → `native-samply.json.syms.json`.
fn default_syms_path(input: &Path) -> PathBuf {
    let s = input.to_string_lossy();
    let trimmed = s.strip_suffix(".gz").unwrap_or(&s);
    PathBuf::from(format!("{trimmed}.syms.json"))
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
