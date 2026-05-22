// wasmtime-runner: loads a moonbit wasm (non-gc target), provides the
// `spectest.print_char` import moonrun-style modules expect, and samples
// CPU usage with wasmtime's built-in GuestProfiler.
//
// The output is Firefox's "processed profile" JSON. Convert with
// runners/wasmtime-to-pprof.mjs (or load directly into
// https://profiler.firefox.com/).

use std::cell::RefCell;
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use clap::Parser;
use wasmtime::{
    Caller, Config, Engine, GuestProfiler, Linker, Module, Store, StoreContextMut,
    UpdateDeadline,
};

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

struct HostState {
    // Buffer for UTF-16 code units coming through spectest.print_char.
    line: Vec<u16>,
    profiler: Rc<RefCell<GuestProfiler>>,
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
    // ackermann(3,10) recurses ~16k deep — moonbit emits >32 bytes/frame so
    // the default 512 KiB wasm stack overflows. Bump both wasm + host caps.
    config.async_stack_size(16 * 1024 * 1024);
    config.max_wasm_stack(8 * 1024 * 1024);

    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, &wasm_bytes)?;

    let interval = Duration::from_micros(args.interval_us);
    let profiler = Rc::new(RefCell::new(GuestProfiler::new(
        &engine,
        "moonbit-guest",
        interval,
        vec![("main.wasm".to_string(), module.clone())],
    )?));

    let mut store = Store::new(
        &engine,
        HostState {
            line: Vec::new(),
            profiler: profiler.clone(),
        },
    );
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(move |mut ctx: StoreContextMut<'_, HostState>| {
        let prof = ctx.data().profiler.clone();
        prof.borrow_mut().sample(&ctx, Duration::ZERO);
        Ok(UpdateDeadline::Continue(1))
    });

    // Bump the engine's epoch counter on a background thread so the
    // deadline callback fires (and samples) every `interval`.
    let engine_for_ticker = engine.clone();
    let stop_ticker = Arc::new(AtomicBool::new(false));
    let stop_flag = stop_ticker.clone();
    let ticker = std::thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(interval);
            engine_for_ticker.increment_epoch();
        }
    });

    let mut linker: Linker<HostState> = Linker::new(&engine);
    linker.func_wrap(
        "spectest",
        "print_char",
        |mut caller: Caller<'_, HostState>, code: i32| {
            let state = caller.data_mut();
            if code == 10 {
                let s: String = String::from_utf16_lossy(&state.line);
                println!("{s}");
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

    stop_ticker.store(true, Ordering::Relaxed);
    let _ = ticker.join();

    // store holds the epoch_deadline_callback closure, which owns a clone of
    // the profiler Rc. Drop it before unwrapping.
    drop(start);
    drop(instance);
    drop(store);

    let out = File::create(&args.out)
        .with_context(|| format!("creating {}", args.out.display()))?;
    Rc::try_unwrap(profiler)
        .map_err(|_| anyhow::anyhow!("profiler still has outstanding refs"))?
        .into_inner()
        .finish(out)?;

    eprintln!(
        "[wasmtime-runner] {} iter in {:.2?} → {}",
        args.iterations,
        elapsed,
        args.out.display()
    );
    Ok(())
}
