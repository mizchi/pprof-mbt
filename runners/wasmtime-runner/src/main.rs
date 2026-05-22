// wasmtime-runner: loads a moonbit wasm (non-gc target), provides the
// `spectest.print_char` import moonrun-style modules expect, and samples
// CPU usage with wasmtime's built-in GuestProfiler.
//
// The output is Firefox's "processed profile" JSON. Convert with
// runners/wasmtime-to-pprof.mjs (or load directly into
// https://profiler.firefox.com/).

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use clap::Parser;
use wasmtime::{Caller, Config, Engine, GuestProfiler, Linker, Module, Store, UpdateDeadline};

#[derive(Parser, Debug)]
#[command(about = "Profile a moonbit wasm with wasmtime's guest profiler")]
struct Args {
    /// Path to the .wasm file
    wasm: PathBuf,
    /// Output profile path (Firefox Profiler JSON)
    #[arg(long, default_value = "wasmtime-guest.json")]
    out: PathBuf,
    /// Sampling interval in microseconds
    #[arg(long, default_value_t = 1000)]
    interval_us: u64,
    /// How many times to invoke `_start`
    #[arg(long, default_value_t = 1)]
    iterations: usize,
}

/// Host state carried in the wasmtime `Store`.
///
/// `profiler` is wrapped in `Option` so the epoch-deadline callback can
/// `take()` it out, call `sample()` with the store as `AsContext`, and put
/// it back — `GuestProfiler::sample` needs `&mut self` *and* the store at
/// the same time, which the borrow checker won't grant if the profiler
/// lives behind a regular `&mut data().profiler`.
struct HostState {
    line: Vec<u16>,
    profiler: Option<GuestProfiler>,
}

/// Background thread that bumps wasmtime's epoch every `interval`, driving
/// the deadline callback that samples the guest. RAII so callers don't
/// have to remember to stop it.
struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: &Engine, interval: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let engine = engine.clone();
        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                thread::sleep(interval);
                engine.increment_epoch();
            }
        });
        Self { stop, handle: Some(handle) }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let wasm_bytes = std::fs::read(&args.wasm)
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
    let profiler = GuestProfiler::new(
        &engine,
        "moonbit-guest",
        interval,
        vec![("main.wasm".to_string(), module.clone())],
    )?;

    let mut store = Store::new(
        &engine,
        HostState {
            line: Vec::new(),
            profiler: Some(profiler),
        },
    );
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(|mut ctx| {
        // Pull the profiler out, sample with the store, then put it back.
        if let Some(mut prof) = ctx.data_mut().profiler.take() {
            prof.sample(&ctx, Duration::ZERO);
            ctx.data_mut().profiler = Some(prof);
        }
        Ok(UpdateDeadline::Continue(1))
    });

    let _ticker = EpochTicker::start(&engine, interval);

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
    drop(_ticker); // stop epoch bumps before consuming store

    let profiler = store
        .into_data()
        .profiler
        .ok_or_else(|| anyhow::anyhow!("profiler was taken but never returned"))?;
    let out = File::create(&args.out)
        .with_context(|| format!("creating {}", args.out.display()))?;
    profiler.finish(out)?;

    eprintln!(
        "[wasmtime-runner] {} iter in {:.2?} → {}",
        args.iterations,
        elapsed,
        args.out.display()
    );
    Ok(())
}
