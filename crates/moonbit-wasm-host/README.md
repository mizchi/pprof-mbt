# moonbit-wasm-host

Wasmtime host imports that satisfy a [MoonBit][mb]-compiled wasm
guest's `println` surface. Drop this crate in alongside
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

`moonbit_wasm_host::register(&mut linker)` wires both in one call.
Buffers live on caller-provided state via the [`MoonbitStdio`] trait so
you keep full control of the `Store<T>` shape.

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
let mut linker: Linker<MyState> = Linker::new(&engine);
moonbit_wasm_host::register(&mut linker)?;
# Ok(()) }
```

For profiling, combine `MoonbitStdio` and
[`wasmtime_guest_pprof::ProfilerHost`][ph] on the same state struct.
See the `moon-pprof profile` subcommand in this repo for a worked
example.

[ph]: https://docs.rs/wasmtime-guest-pprof/

## License

Apache-2.0
