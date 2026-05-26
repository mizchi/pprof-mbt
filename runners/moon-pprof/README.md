# moon-pprof

Unified CLI for profiling [MoonBit](https://www.moonbitlang.com/) code
across all four backends (`native` / `wasm-gc` / `wasm` / `js`) and
normalising every run into the
[pprof](https://github.com/google/pprof) format.

## Install

```sh
cargo install moon-pprof --locked
```

Requires `protoc` on PATH at build time (the wasm-host helpers depend
on prost-build).

## Subcommands

| Subcommand | Purpose |
|---|---|
| `profile <wasm>` | Run a wasm binary under wasmtime + GuestProfiler and emit gzip'd pprof. |
| `summary <file>` | Top-N self-time + mem-mgmt rollup for a pprof file. |
| `summary --diff <a> <b>` | Diff two profiles at function granularity. |
| `bench` | Drive a set of MoonBit benches across backends and emit a markdown delta table. |
| `cpuprofile2pprof <in> <out>` | V8 `.cpuprofile` → pprof gzip. |
| `heapprofile2pprof <in> <out>` | V8 `.heapprofile` → pprof gzip. |
| `firefox2pprof <in> <out>` | Firefox Profiler JSON → pprof (samply / wasmtime guest). |
| `perf2pprof <perf-script.txt>` | Linux `perf script` text → pprof gzip. |
| `memprofile <wasm>` | Per-alloc profile via wasm instrumentation. |
| `memprofile-native <exe>` | Per-alloc profile of a `--target native` binary (patch-and-relink, macOS + Linux). |

See the [main repo](https://github.com/mizchi/pprof-mbt) for the
broader workflow (Quickstart, baseline ↔ patched bench comparisons,
host shims).

## License

Apache-2.0.
