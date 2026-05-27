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
| `chrometrace2pprof <in> <out>` | Chrome trace-event JSON with V8 CPU profile chunks → pprof gzip. |
| `pprof2chrometrace <in.pb.gz> <out.json>` | pprof CPU profile → synthetic Chrome trace-event JSON. |
| `pprof2folded <in.pb.gz> <out.folded>` | pprof CPU profile → folded stacks. |
| `folded2pprof <in.folded> <out.pb.gz>` | Folded stacks → pprof gzip (`delay/microseconds` by default). |
| `pprof2speedscope <in.pb.gz> <out.json>` | pprof CPU profile → Speedscope JSON. |
| `speedscope2pprof <in.json> <out.pb.gz>` | Speedscope sampled JSON → pprof gzip. |
| `heapprofile2pprof <in> <out>` | V8 `.heapprofile` → pprof gzip. |
| `firefox2pprof <in> <out>` | Firefox Profiler JSON → pprof (samply / wasmtime guest). |
| `perf2pprof <perf-script.txt>` | Linux `perf script` text → pprof gzip. |
| `memprofile <wasm>` | Per-alloc profile via wasm instrumentation. Add `--trace-out` for a Chrome trace allocation timeline. |
| `memprofile-native <exe>` | Per-alloc profile of a `--target native` binary (patch-and-relink, macOS + Linux). Add `--retained` for retained heap. |

See the [main repo](https://github.com/mizchi/moon-pprof) for the
broader workflow (Quickstart, baseline ↔ patched bench comparisons,
host shims).

## License

Apache-2.0.
