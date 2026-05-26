# Cross-backend bench

Useful for two questions:

1. "Does this MoonBit code perform consistently across native /
   wasm-gc / wasm / js?"
2. "Which backend's profiler gives the cleanest signal for *this*
   workload?"

## One-shot cross-backend run

```sh
moon-pprof bench \
  --bench-dir bench-x \
  --backends native,wasm-gc,wasm,js \
  --workloads uuid_parse,base64_encode,sha256_hash \
  --runs 3 \
  > cross.md
```

Outputs a markdown table with per-backend wall-time medians (markdown
is the default — redirect to capture). Useful
for slotting straight into `notes/` or a PR description.

## Backend characteristics worth knowing

| Backend | Speed | Profiler quality | Notes |
|---|---|---|---|
| `native` | fastest, baseline for most things | `memprofile-native` (allocs), `samply` / `perf` (CPU). Symbol resolution is the cleanest. | mimalloc statically linked — `LD_PRELOAD` doesn't catch allocs, hence the patch-and-relink approach. |
| `wasm-gc` | 1.5–3× native | `wasmtime` `GuestProfiler` for CPU, walrus instrumentation for allocs. 10–15 % profiling overhead. | Most maintained backend right now. |
| `wasm` | similar to wasm-gc, sometimes slower (refcount cost) | Same wasmtime path. `moonbit.malloc` wrapped for memprofile. | Older path; still useful for `Hash` / `Set` micro-comparisons. |
| `js` | 5–15× native depending on workload | V8 inspector CPU + heap profiles. `cpuprofile2pprof` / `heapprofile2pprof`. | The runner falls back to `vm.Script` for code that uses `require()` (~5–15× slower than dynamic import). |

## When backends diverge

If the same workload shows wildly different relative cost across
backends, that's a signal worth investigating, not a bug:

- A wasm regression that's invisible on native often means the
  patch interacts with refcounting (e.g. tail-call patterns).
- A js-only slowdown usually means V8 deoptimized — check
  `--allow-natives-syntax` traces in `runners/v8/`.
- A native-only regression often means mimalloc fragmentation;
  cross-check with `valgrind callgrind`.

## Host-shim status

The wasm-gc / js runners ship with shims that satisfy the common
MoonBit `__moonbit_sys_unstable.is_windows` import + `autoStubMissing`
for anything else. WASI `snapshot_preview1` is NOT stubbed because
no bench currently imports it — if you add a workload that uses
`@io` / `@fs` directly on wasm-gc, expect the autostub fallback
(returns 0 for everything) and rewrite if it breaks.

## Don't put the bench harness in the hot path

`moon-pprof bench` runs `moon build` + warm-up between measurements.
For micro-benches under ~10 ms, the harness overhead can dominate.
For those, use `hyperfine` directly on the built binary and skip
the cross-backend table.
