# moonbit-wasm-host

Wasmtime host imports that satisfy a [MoonBit][mb]-compiled wasm
guest's basic runtime surface. Drop this crate in alongside
[`wasmtime-guest-pprof`][wgp] when profiling MoonBit wasm; skip it when
profiling generic wasm.

[mb]: https://www.moonbitlang.com/
[wgp]: https://crates.io/crates/wasmtime-guest-pprof

## What it provides

MoonBit's `wasm` target emits one of two import flavors for `println`:

1. **Legacy moonrun style** — calls `spectest.print_char(i32)` once per
   UTF-16 code unit, flushing the buffered line on `\n`.
2. **Modern WASI style** — calls `wasi_snapshot_preview1.fd_write` with
   iovs of UTF-8 bytes.

`moonbit_wasm_host::register(&mut linker)` wires those, plus the common
exception, time, and string-reader imports used by MoonBit test and
benchmark artifacts. `exception.tag` is a store-owned Wasmtime object, so
call `moonbit_wasm_host::register_store_imports(&mut linker, &mut store)`
before instantiation when running modules that import it.

## Usage

```rust,no_run
use wasmtime::{Engine, Linker, Store};
use moonbit_wasm_host::{MoonbitStdio, MoonbitStdioState};

struct MyState {
    stdio: MoonbitStdioState,
    // ...whatever else you need (profile session, etc.)
}
impl MoonbitStdio for MyState {
    fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState { &mut self.stdio }
}

# fn doctest() -> anyhow::Result<()> {
let engine = Engine::default();
let mut store = Store::new(&engine, MyState { stdio: MoonbitStdioState::default() });
let mut linker: Linker<MyState> = Linker::new(&engine);
moonbit_wasm_host::register(&mut linker)?;
moonbit_wasm_host::register_store_imports(&mut linker, &mut store)?;
# Ok(()) }
```

For profiling, combine `MoonbitStdio` and
[`wasmtime_guest_pprof::ProfilerHost`][ph] on the same state struct.
See the `moon-pprof profile` subcommand in this repo for a worked
example.

[ph]: https://docs.rs/wasmtime-guest-pprof/

## License

Apache-2.0
