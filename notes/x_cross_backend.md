# `moonbitlang/x` patches across backends

Re-ran the 6 PR patches we collected on `moonbitlang/x` against
`native`, `wasm-gc` (wasmtime via the run-wasm-gc.mjs host shim), and
`js` (node 22 with `--no-profile`).

bench-x setup is identical for all three backends (same `moon.mod`,
same bench source). Each row is the 3-run median wall time in seconds.
For wasm-gc / js the timing comes from the runner's own
`performance.now()` around the workload (excluding node startup); for
native it's `bash time real`.

## Results

| bench               | backend | baseline | patched | delta   |
|---------------------|---------|---------:|--------:|--------:|
| uuid_parse          | native  |   0.543  |  0.200  | **-63.2%** |
|                     | wasm-gc |   0.561  |  0.377  | **-32.8%** |
|                     | js      |   6.611  |  4.032  | **-39.0%** |
| encoding_utf8       | native  |   0.527  |  0.401  | **-23.9%** |
|                     | wasm-gc |   0.418  |  0.252  | **-39.7%** |
|                     | js      |   0.589  |  0.358  | **-39.2%** |
| path_normalize      | native  |   0.285  |  0.206  | **-27.7%** |
|                     | wasm-gc |   —      |  —      | (now runs after host-shim fix; see below) |
|                     | js      |   —      |  —      | (now runs via vm.Script fallback; see below) |
| plain_datetime_parse| native  |   0.180  |  0.133  | **-26.1%** |
|                     | wasm-gc |   0.232  |  0.212  | **-8.6%**  |
|                     | js      |   0.395  |  0.324  | **-18.0%** |
| base64_encode       | native  |   0.711  |  0.561  | **-21.1%** |
|                     | wasm-gc |   0.561  |  0.505  | **-10.0%** |
|                     | js      |   1.130  |  1.046  | **-7.4%**  |
| base64_decode       | native  |   0.690  |  0.425  | **-38.4%** |
|                     | wasm-gc |   0.613  |  0.369  | **-39.8%** |
|                     | js      |   0.844  |  0.712  | **-15.6%** |
| json5_parse         | native  |   0.236  |  0.236  | ~0%        |
|                     | wasm-gc |   0.334  |  0.345  | +3%        |
|                     | js      |   0.469  |  0.468  | ~0%        |

## What this says about each patch

- **PR-04 (uuid in-place reinterpret)**: wins on every backend. The
  `Bytes::from_array(Array::from_fixed_array(rv))` chain is two real
  copies regardless of backend, so removing them helps everywhere. The
  native win is largest because the bench is dominated by the alloc /
  copy work; wasm-gc / js have higher constant overhead per iteration
  that dilutes the relative gain. JS keeps a -39% wall time gain
  despite running the inner loop ~10x slower.

- **PR-03 (encoding utf-8 code-unit walk)**: bigger win on wasm-gc / js
  (-40%) than on native (-24%). The `for char in src` iterator path
  uses `Iter::next` + per-char `Option<Char>` allocation + surrogate
  decode; these allocations are relatively more expensive on the GC'd
  backends.

- **PR-02 (base64 index loop)**: similar pattern — base64_decode wins
  -38% on native, -40% on wasm-gc, -16% on js. The js gain is smaller
  because V8 already inlines simple iterator chains well. base64_encode
  wins -21% / -10% / -7% — smaller on managed backends but still real.

- **PR-05 (path Show write_view)**: native-only because the wasm-gc /
  js benches couldn't run (the `path` module depends on
  `@path.internal.ffi.is_windows`, which needs an `os` shim we don't
  have in our runners). The patch itself is backend-independent so it
  should help wasm-gc / js once the bench can run.

- **PR-06 (time split substring)**: wins everywhere — native -26%,
  wasm-gc -9%, js -18%. The wasm-gc gain is smaller because the
  per-char StringBuilder loop was already cheap on the wasm-gc path
  (no per-iteration allocation overhead for primitives).

- **PR-01 (json5 lazy to_string)**: noise on every backend. The bench
  builds a small number-heavy JSON5 array; the saved allocation count
  is real but proportionally small. Worth keeping as a correctness /
  hygiene fix more than a perf fix.

## Backend characteristics observed

- **native** is fastest in absolute terms for these workloads (no
  GC, no host crossings).
- **wasm-gc** is competitive — usually within 1.5–2x of native for
  numeric / loop workloads, and sometimes faster than native for
  alloc-heavy paths where mimalloc's free path is expensive (e.g.
  encoding_utf8 baseline: wasm-gc 0.418 vs native 0.527).
- **js** is the slowest, with the gap being largest for tight
  allocation-heavy loops (uuid_parse: js 6.6s vs native 0.5s, 12x
  slower). For workloads dominated by string parsing the gap shrinks
  to ~2x.

## What broke (now fixed in roadmap v2 task D)

- ~~`path_normalize` on wasm-gc / js failed~~. The wasm-gc host shim
  (`runners/run-wasm-gc.mjs`) now provides an
  `__moonbit_sys_unstable.is_windows` stub and a catch-all noop stub
  for any other moonbit FFI imports it doesn't know about. The JS
  runner (`runners/run-js.mjs`) now detects `require()` calls in the
  emitted source and switches to a `vm.Script` fallback with
  `createRequire()` injected. With those two fixes:
  - wasm-gc: path_normalize 304 ms baseline (full speed)
  - js: 7700 ms baseline (vm.Script is ~5-15x slower than dynamic
    import, which is the trade-off for not being able to dynamic-import
    a CommonJS-style file).

- A few other modules (decimal, fs, sys) aren't on bench-x because
  they either need a JS-only runtime (decimal pulls `@bigint`, which
  works but we never added a wasm-gc bench), or they're I/O-bound.

## Snapshot

- Baseline data: `/tmp/bench-x-baseline.tsv`
- Patched data: `/tmp/bench-x-patched-all.tsv`
- Patched `.mooncakes/` snapshot for reuse: `/tmp/mooncakes-patched/`
  (contains all 6 PRs applied on top of `moonbitlang/x` 0.4.43)
- Runner shim: `runners/run-wasm-gc.mjs` (--no-profile flag),
  `runners/run-js.mjs` (--no-profile flag)
- Driver: `/tmp/bench-x-cross.sh`
