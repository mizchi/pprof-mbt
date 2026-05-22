# moonbit-pprof

Convert profiling output from various producers into [pprof][pprof], with
moonbit-aware symbol demangling. Subpath exports keep imports tree-shakable
and explicit.

[pprof]: https://github.com/google/pprof

```js
import { demangle } from "moonbit-pprof/demangle";
import { convert as cpuprofileToPprof } from "moonbit-pprof/cpuprofile-to-pprof";
import { writePprofFromFirefox } from "moonbit-pprof/firefox-to-pprof";

demangle("_M0FP26mizchi5bench9ackermann");
// -> "mizchi::bench::ackermann"
```

## Exports

### `moonbit-pprof/demangle`

```js
demangle(name: string): string
```

Heuristic decoder for moonbit's symbol mangling. Returns `name` verbatim if
it doesn't look mangled. See [`crates/moonbit-demangle`][rust] and
[`go/demangle`][go] for matching implementations.

[rust]: https://crates.io/crates/moonbit-demangle
[go]: https://pkg.go.dev/github.com/mizchi/pprof-mbt/go/demangle

### `moonbit-pprof/cpuprofile-to-pprof`

```js
convert(cpuprofile, { demangle?, mappingFilename? }): { encoded, stats }
```

Convert a Node V8 `.cpuprofile` (from `Profiler.start/stop` or
`node --cpu-prof`) into gzip'd pprof bytes. The default demangler is
moonbit's; pass `{ demangle: s => s }` for non-moonbit code.

### `moonbit-pprof/firefox-to-pprof`

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

## License

Apache-2.0
