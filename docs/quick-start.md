# Quick start

Install the CLI, profile a small sample wasm, read the summary — no
MoonBit toolchain required.

## 1. Install the CLI

Requires `rustc` 1.80+ and `protoc` on `PATH`.

```sh
cargo install --git https://github.com/mizchi/pprof-mbt moon-pprof --locked
moon-pprof --help
```

Nix users can skip the toolchain dance entirely:

```sh
nix run github:mizchi/pprof-mbt -- --help
# or persistently:
nix profile install github:mizchi/pprof-mbt
```

## 2. Grab a sample wasm

A 4.7 KB MoonBit `wasm-gc` binary that exercises ackermann / fib /
mandelbrot lives in this repo at `docs/samples/main.wasm`:

```sh
curl -fSLO https://raw.githubusercontent.com/mizchi/pprof-mbt/main/docs/samples/main.wasm
```

## 3. Profile and summarize

```sh
moon-pprof profile --wasm-gc main.wasm --out main.pb.gz
moon-pprof summary main.pb.gz
```

Expected output (timings vary by machine):

```
Profile: main.pb.gz
Total ms: 213.31 (162 samples)
Memory-management self time: 0.00 ms (0.0%)

Top user functions by self time (mem-mgmt frames hidden)
--------------------------------------------------------
   186.93 ms   87.6%  mizchi::bench::ackermann
    14.99 ms    7.0%  mizchi::bench::fib
    11.39 ms    5.3%  mizchi::bench::mandel__point
```

## 4. View it in a browser (optional)

`moon-pprof` emits standard pprof, so `go tool pprof` renders it
directly — flamegraph, call graph, top, source, all included.

```sh
# requires `go` on PATH (and `graphviz` for the SVG call graph)
go tool pprof -http :8000 main.pb.gz
```

Open <http://localhost:8000> and switch views from the **VIEW** menu
(Top / Graph / Flame Graph / Source).

No `go` installed? `nix develop github:mizchi/pprof-mbt` drops you into a
shell with `go` + `graphviz` ready:

```sh
nix develop github:mizchi/pprof-mbt -c go tool pprof -http :8000 main.pb.gz
```

## Next steps

- Convert a Chrome / Node V8 `.cpuprofile`:
  `moon-pprof cpuprofile2pprof in.cpuprofile out.pb.gz`
- Convert a Node V8 `.heapprofile` (sampling allocations) into a pprof
  with `alloc_objects` / `alloc_space` sample types:
  `moon-pprof heapprofile2pprof in.heapprofile out.pb.gz`
- Profile allocations of any MoonBit wasm or wasm-gc via walrus
  instrumentation + wasmtime backtraces:
  `moon-pprof memprofile path/to/main.wasm --out wasm-mem.pb.gz`
  (add `--sample-rate 100` on large workloads — within 0.1 % of the
  exact top sites and ~22 × faster on a 13 M-alloc JSON parse).
- Profile allocations of a MoonBit `--target native` binary by
  patching its generated C and relinking with a hook (macOS only):
  `moon-pprof memprofile-native path/to/cmd.exe --out native-mem.pb.gz --sample-rate 100`.
- Convert a Firefox Profiler / samply JSON:
  `moon-pprof firefox2pprof in.json out.pb.gz`
- Build and profile your own MoonBit project across all four backends —
  see the [README](../README.md) Quickstart section. Memory profiling
  is supported on the js backend via
  [`npm run profile:js:heap`](../README.md#memory-profiling-js).
