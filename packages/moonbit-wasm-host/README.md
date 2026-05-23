# @mizchi/moonbit-wasm-host

Host imports for running a [MoonBit](https://www.moonbitlang.com/)-compiled
wasm-gc / wasm guest under Node (V8). Mirrors the moonrun host so a
MoonBit wasm runs out of the box:

* `spectest.print_char(i32)` — UTF-16 code units, flushes a line on `\n`.
* `__moonbit_sys_unstable.is_windows() -> i32` — platform flag.
* `wasi_snapshot_preview1.fd_write(...)` — minimal WASI stub that walks
  the iovs, buffers per-line, and writes the byte count back.

```js
import { moonbitWasmImports, autoStubMissing } from "@mizchi/moonbit-wasm-host";

const mod = await WebAssembly.compile(wasmBytes);
const imports = moonbitWasmImports();
const stubbed = autoStubMissing(imports, mod);
const instance = await WebAssembly.instantiate(mod, imports);
```

`autoStubMissing` is generic: for every import the module declares that
doesn't already have a binding, it injects a noop function returning
`0`. Useful to keep a bench linking while the toolchain adds new FFI
shims.

## Related

The `wasmtime` (Rust) equivalent lives in the
[`moonbit-wasm-host`](https://crates.io/crates/moonbit-wasm-host) crate.

This package used to ship pprof / firefox-profile / cpuprofile / MoonBit
demangle utilities in TypeScript. They have been consolidated into Rust
crates exposed by the `moon-pprof` CLI (`cpuprofile2pprof`,
`firefox2pprof`).

## License

Apache-2.0
