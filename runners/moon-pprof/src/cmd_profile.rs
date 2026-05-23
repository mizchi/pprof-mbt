//! `moon-pprof profile <wasm>` — profile a MoonBit wasm binary via
//! wasmtime's GuestProfiler and write the result as gzip'd pprof.
//!
//! Composes:
//!   - wasmtime-guest-pprof: GuestProfiler + pprof emission (generic)
//!   - firefox-to-pprof:     Firefox JSON → pprof (generic)
//!   - moonbit-demangle:     symbol demangling (MoonBit-specific)
//!   - moonbit-wasm-host:    spectest.print_char + wasi fd_write host imports

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use clap::Parser;
use moonbit_wasm_host::{MoonbitStdio, MoonbitStdioState};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_guest_pprof::{
    ProfileSession, ProfilerHost, ProfilerHostExt as _, TakeProfileSession, json_to_pprof,
};

#[derive(Parser, Debug)]
#[command(about = "Profile a MoonBit wasm with wasmtime's guest profiler")]
pub struct Args {
    /// Path to the .wasm file
    pub wasm: PathBuf,
    /// Output path for the gzip'd pprof
    #[arg(long, default_value = "wasmtime-guest.pb.gz")]
    pub out: PathBuf,
    /// If set, also write the Firefox Profiler JSON to this path
    #[arg(long)]
    pub json_out: Option<PathBuf>,
    /// Sampling interval in microseconds
    #[arg(long, default_value_t = 1000)]
    pub interval_us: u64,
    /// How many times to invoke `_start`
    #[arg(long, default_value_t = 1)]
    pub iterations: usize,
    /// Skip the GuestProfiler entirely: no epoch_interruption, no
    /// epoch ticker thread, no pprof output. Use this for clean
    /// wall-time measurements without the ~10-15% sampling overhead.
    #[arg(long)]
    pub no_profile: bool,
    /// Enable wasm-gc proposal (typed refs, struct/array, i31ref) on
    /// the wasmtime engine. Required for profiling MoonBit's
    /// `--target=wasm-gc` output. The legacy `--target=wasm` output
    /// does not need this flag.
    #[arg(long)]
    pub wasm_gc: bool,
}

struct HostState {
    stdio: MoonbitStdioState,
    profiler: ProfileSession,
}

impl MoonbitStdio for HostState {
    fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState {
        &mut self.stdio
    }
}

impl ProfilerHost for HostState {
    fn profiler(&mut self) -> &mut ProfileSession {
        &mut self.profiler
    }
}

impl TakeProfileSession for HostState {
    fn take_session(store: Store<Self>) -> ProfileSession {
        store.into_data().profiler
    }
}

pub fn run(args: Args) -> Result<()> {
    let wasm_bytes = fs::read(&args.wasm)
        .with_context(|| format!("reading wasm at {}", args.wasm.display()))?;

    let mut config = Config::new();
    if !args.no_profile {
        config.epoch_interruption(true);
    }
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);
    config.generate_address_map(true);
    if args.wasm_gc {
        // wasm-gc requires function-references and reference-types as
        // prerequisites (the spec layers them).
        config.wasm_reference_types(true);
        config.wasm_function_references(true);
        config.wasm_gc(true);
    }
    // ackermann(3, 10) recurses ~16k deep — moonbit emits >32 bytes/frame so
    // the default 512 KiB wasm stack overflows. Bump both wasm + host caps.
    config.async_stack_size(16 * 1024 * 1024);
    config.max_wasm_stack(8 * 1024 * 1024);

    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, &wasm_bytes)?;

    let interval = Duration::from_micros(args.interval_us);
    let session = ProfileSession::new(
        &engine,
        "moonbit-guest",
        interval,
        vec![("main.wasm".to_string(), module.clone())],
    )?;

    let mut store = Store::new(
        &engine,
        HostState {
            stdio: MoonbitStdioState::default(),
            profiler: session,
        },
    );
    let _ticker = if args.no_profile {
        None
    } else {
        HostState::install(&mut store);
        Some(HostState::start_ticker(&engine, interval))
    };

    let mut linker: Linker<HostState> = Linker::new(&engine);
    moonbit_wasm_host::register(&mut linker)?;

    let instance = linker.instantiate(&mut store, &module)?;
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;

    let t0 = Instant::now();
    for _ in 0..args.iterations {
        start.call(&mut store, ())?;
    }
    let elapsed = t0.elapsed();
    drop(_ticker);

    if args.no_profile {
        eprintln!(
            "[moon-pprof profile] {} iter in {:.2?} (no profile)",
            args.iterations, elapsed,
        );
        return Ok(());
    }

    let session = HostState::take_session(store);
    let mut json = Vec::new();
    session.into_json(&mut json)?;

    if let Some(json_path) = args.json_out.as_ref() {
        fs::write(json_path, &json)
            .with_context(|| format!("writing {}", json_path.display()))?;
    }
    let pprof_bytes = json_to_pprof(&json)?;
    fs::write(&args.out, &pprof_bytes)
        .with_context(|| format!("writing {}", args.out.display()))?;

    eprintln!(
        "[moon-pprof profile] {} iter in {:.2?} → {}",
        args.iterations,
        elapsed,
        args.out.display(),
    );
    if let Some(p) = args.json_out.as_ref() {
        eprintln!("[moon-pprof profile] firefox json → {}", p.display());
    }
    Ok(())
}
