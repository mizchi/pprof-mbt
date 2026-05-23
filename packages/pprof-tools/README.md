# @mizchi/pprof-tools

Host imports for running a [MoonBit](https://www.moonbitlang.com/)-compiled
wasm-gc / wasm guest under Node. Mirrors the moonrun host:
`spectest.print_char` flushes a line on `\n`, and
`__moonbit_sys_unstable.is_windows` returns the platform flag.

```js
import {
  moonbitWasmImports,
  autoStubMissing,
} from "@mizchi/pprof-tools/moonbit/wasm-host-imports";

const mod = await WebAssembly.compile(wasmBytes);
const imports = moonbitWasmImports();
const stubbed = autoStubMissing(imports, mod);
const instance = await WebAssembly.instantiate(mod, imports);
```

`autoStubMissing` is generic: for every import the module declares that
doesn't already have a binding, it injects a noop function returning
`0`. Useful to keep a bench linking while the toolchain adds new FFI
shims.

## Moved to Rust

This package used to also ship V8 cpuprofile / Firefox Profiler /
MoonBit demangle utilities written in TypeScript. They've been
consolidated into Rust crates exposed by the `moon-pprof` CLI:

| Old export | New home |
|---|---|
| `@mizchi/pprof-tools/cpuprofile-to-pprof` | `cpuprofile-to-pprof` crate / `moon-pprof cpuprofile2pprof` |
| `@mizchi/pprof-tools/firefox-to-pprof`   | `firefox-to-pprof` crate / `moon-pprof firefox2pprof` |
| `@mizchi/pprof-tools/moonbit/demangle`   | `moonbit-demangle` crate |

The Rust implementations and previous TS implementations were verified
against the same inputs in the parent repo before the TS ports were
removed.

## License

Apache-2.0
