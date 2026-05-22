# wasmtime-guest-pprof

A drop-in helper around [wasmtime's `GuestProfiler`][gp] that handles:

- the [`take()` / restore][borrow] borrow-checker dance the sampling
  callback needs,
- the background thread that ticks `engine.increment_epoch()` to drive
  sampling,
- the [Firefox Profiler JSON][ff] → [pprof][pprof] conversion (via
  [`firefox-to-pprof`](https://crates.io/crates/firefox-to-pprof)).

[gp]: https://docs.rs/wasmtime/latest/wasmtime/struct.GuestProfiler.html
[borrow]: https://github.com/bytecodealliance/wasmtime/blob/main/crates/wasmtime/src/runtime/profiling.rs
[ff]: https://github.com/firefox-devtools/profiler/blob/main/docs-developer/processed-profile-format.md
[pprof]: https://github.com/google/pprof

You still own:

- the `wasmtime::Engine`/`Module`/`Linker` (so you can declare host imports)
- your `Store<T>` data, which must implement [`ProfilerHost`] +
  [`TakeProfileSession`]

## Sketch

```rust,no_run
use std::time::Duration;
use wasmtime::{Config, Engine, Module, Store};
use wasmtime_guest_pprof::{ProfileSession, ProfilerHost, ProfilerHostExt, TakeProfileSession};

struct App {
    profiler: ProfileSession,
    // ...your host state
}
impl ProfilerHost for App {
    fn profiler(&mut self) -> &mut ProfileSession { &mut self.profiler }
}
impl TakeProfileSession for App {
    fn take_session(store: Store<Self>) -> ProfileSession {
        store.into_data().profiler
    }
}

# fn main() -> anyhow::Result<()> {
let mut config = Config::new();
config.epoch_interruption(true);
config.generate_address_map(true);
let engine = Engine::new(&config)?;
let module = Module::from_file(&engine, "guest.wasm")?;

let session = ProfileSession::new(
    &engine,
    "my-guest",
    Duration::from_millis(1),
    vec![("guest.wasm".into(), module.clone())],
)?;
let mut store = Store::new(&engine, App { profiler: session });
App::install(&mut store);
let _ticker = App::start_ticker(&engine, Duration::from_millis(1));

// ...invoke guest exports against `store`...

drop(_ticker);
let bytes = App::finish_pprof(store)?;
std::fs::write("guest.pb.gz", bytes)?;
# Ok(())
# }
```

## License

Apache-2.0
