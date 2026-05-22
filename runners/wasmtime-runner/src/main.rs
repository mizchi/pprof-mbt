// wasmtime-runner: profile a moonbit wasm (non-gc target) with wasmtime's
// GuestProfiler and write the result as gzip'd pprof.
//
// Most of the work lives in the workspace crates:
//   - wasmtime-guest-pprof: GuestProfiler driving + pprof emission
//   - firefox-to-pprof:     the Firefox JSON → pprof conversion
//   - moonbit-demangle:     symbol demangling
//
// This file is just the CLI + the moonbit-specific spectest host import.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use clap::Parser;
use wasmtime::{Caller, Config, Engine, Linker, Module, Store};
use wasmtime_guest_pprof::{
    json_to_pprof, ProfileSession, ProfilerHost, ProfilerHostExt as _, TakeProfileSession,
};

#[derive(Parser, Debug)]
#[command(about = "Profile a moonbit wasm with wasmtime's guest profiler")]
struct Args {
    /// Path to the .wasm file
    wasm: PathBuf,
    /// Output path for the gzip'd pprof
    #[arg(long, default_value = "wasmtime-guest.pb.gz")]
    out: PathBuf,
    /// If set, also write the Firefox Profiler JSON to this path
    #[arg(long)]
    json_out: Option<PathBuf>,
    /// Sampling interval in microseconds
    #[arg(long, default_value_t = 1000)]
    interval_us: u64,
    /// How many times to invoke `_start`
    #[arg(long, default_value_t = 1)]
    iterations: usize,
}

/// Host state carried in the wasmtime `Store`. Owns the line buffer used
/// by the moonrun-style `spectest.print_char` host function and the
/// profile session.
struct HostState {
    line: Vec<u16>,
    profiler: ProfileSession,
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

fn main() -> Result<()> {
    let args = Args::parse();
    let wasm_bytes = fs::read(&args.wasm)
        .with_context(|| format!("reading wasm at {}", args.wasm.display()))?;

    let mut config = Config::new();
    config.epoch_interruption(true);
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);
    // GuestProfiler needs the pc → wasm offset map to resolve symbols.
    config.generate_address_map(true);
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
            line: Vec::new(),
            profiler: session,
        },
    );
    HostState::install(&mut store);
    let _ticker = HostState::start_ticker(&engine, interval);

    let mut linker: Linker<HostState> = Linker::new(&engine);
    linker.func_wrap(
        "spectest",
        "print_char",
        |mut caller: Caller<'_, HostState>, code: i32| {
            // moonbit emits UTF-16 code units one at a time; flush on '\n'.
            let state = caller.data_mut();
            if code == b'\n' as i32 {
                println!("{}", String::from_utf16_lossy(&state.line));
                state.line.clear();
            } else {
                state.line.push(code as u16);
            }
        },
    )?;

    let instance = linker.instantiate(&mut store, &module)?;
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;

    let t0 = Instant::now();
    for _ in 0..args.iterations {
        start.call(&mut store, ())?;
    }
    let elapsed = t0.elapsed();
    drop(_ticker); // stop epoch bumps before consuming the store

    // Extract the session, then derive both outputs from the same JSON.
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
        "[wasmtime-runner] {} iter in {:.2?} → {}",
        args.iterations,
        elapsed,
        args.out.display(),
    );
    if let Some(p) = args.json_out.as_ref() {
        eprintln!("[wasmtime-runner] firefox json → {}", p.display());
    }
    Ok(())
}
