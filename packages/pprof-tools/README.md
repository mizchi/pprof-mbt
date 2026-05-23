# @mizchi/pprof-tools

Convert profiling output from various producers into [pprof][pprof].
Generic converters live at the package root; MoonBit-specific helpers
live under the `/moonbit` subpath.

[pprof]: https://github.com/google/pprof

```js
import { writePprofFromFirefox } from "@mizchi/pprof-tools/firefox-to-pprof";

// MoonBit-specific helpers (skip these if you're profiling Rust / AS / Zig wasm):
import { demangle } from "@mizchi/pprof-tools/moonbit/demangle";
import {
  moonbitWasmImports,
  autoStubMissing,
} from "@mizchi/pprof-tools/moonbit/wasm-host-imports";
```

> V8 `.cpuprofile → pprof` conversion has moved to the Rust crate
> `cpuprofile-to-pprof` and is invoked from the `moon-pprof` CLI
> (`moon-pprof cpuprofile2pprof <in> <out>`). The TypeScript port that
> used to live here was removed; use the Rust crate or the CLI instead.

## Generic exports

### `@mizchi/pprof-tools/firefox-to-pprof`

```js
writePprofFromFirefox(options, outPath)
buildPprofFromFirefox(options): { encoded, stats }
```

Convert Firefox Profiler "processed profile" JSON (samply, wasmtime
GuestProfiler, …) into gzip'd pprof. You supply two callbacks:

- `resolveFrame(thread, frameIdx)` — returns one or more
  [`ResolvedFrame`](./firefox-to-pprof.mjs) entries (leaf-first for
  inline chains).
- `resolveSample(thread, sampleIdx)` — returns `{ stack, count, ns }`.

See `runners/samply-to-pprof.mjs` and `runners/wasmtime-to-pprof.mjs` in
the source repo for worked examples.

## MoonBit-specific exports (`/moonbit` subpath)

### `@mizchi/pprof-tools/moonbit/demangle`

```js
demangle(name: string): string
```

Heuristic decoder for MoonBit's symbol mangling (e.g.
`_M0FP26mizchi5bench9ackermann` → `mizchi::bench::ackermann`). Returns
`name` verbatim if it doesn't look mangled. See
[`crates/moonbit-demangle`][rust] for the matching Rust implementation.

[rust]: https://crates.io/crates/moonbit-demangle

### `@mizchi/pprof-tools/moonbit/wasm-host-imports`

```js
moonbitWasmImports({ writeLine?, isWindows? }) // returns an imports object
autoStubMissing(imports, mod): string[]        // generic helper
```

Host imports for running a MoonBit-compiled wasm-gc / wasm guest in
Node. Mirrors the moonrun host: `spectest.print_char` flushes a line on
`\n`, and `__moonbit_sys_unstable.is_windows` returns the platform flag.

`autoStubMissing` is generic: for every import in `mod` that doesn't
already have a binding, it injects a noop function returning 0. Useful
to keep the bench linking when the toolchain adds new FFI shims.

## License

Apache-2.0
