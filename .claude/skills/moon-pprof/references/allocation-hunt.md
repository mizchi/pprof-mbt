# Allocation hunt

When you suspect "this code allocates too much", confirm with a
profile before touching anything.

## wasm / wasm-gc

```sh
moon build --target wasm-gc --release
moon-pprof memprofile bench/_build/wasm-gc/release/build/cmd/<name>/main.wasm \
  --out wasm-mem.pb.gz --sample-rate 100
moon-pprof summary wasm-mem.pb.gz
```

- `--sample-rate 100` is the default working point — 22× faster than
  exact, < 0.1 % deviation on top sites.
- wasm wraps `moonbit.malloc` (covers raw + `moonbit.gc.malloc`).
- wasm-gc rewrites every `struct.new` / `array.new*` opcode so the
  host hook fires with the alloc size. The size is a field-sum
  proxy, not the true GC heap cost — useful for relative ranking.

## native

```sh
moon build --target native --release
moon-pprof memprofile-native ./_build/native/release/build/cmd/<name>/main.exe \
  --out native-mem.pb.gz --sample-rate 100
moon-pprof summary native-mem.pb.gz
```

On macOS + Linux. Works by patching the generated `<cmd>.c` to call
a backtrace hook inside `moonbit_malloc_inlined`, then relinking
via the project's own cc command. The `.memprof.exe` sibling stays
around for re-runs / inspection.

For servers / event loops that never `return`, add `--duration N`
(integer seconds) and the hook will flush + `_exit(0)` cleanly on
SIGTERM. See `long-running.md`.

## js

The js path uses Node's V8 sampling heap profiler:

```sh
npm run build:js
npm run profile:js:heap        # writes js.heapprofile
moon-pprof heapprofile2pprof js.heapprofile --out js-heap.pb.gz
moon-pprof summary js-heap.pb.gz
```

## What "Total: X MB" actually means

`memprofile` and `memprofile-native` report **bytes requested
through MoonBit's allocator surface**, not RSS. A workload that
allocates 200 MB and frees it all the same tick will still show
200 MB. Use `summary`'s top-N to rank sites, then verify with a
narrower reproducer (e.g. wrap the suspicious function in a loop
and re-profile) before drawing conclusions.

## Drill-down

When a function is hot, open `go tool pprof -http :8000 <file>` and
use **Source view** for line-by-line attribution. `moon-pprof
summary --by-line` is on the roadmap but not implemented yet.

## Common false positives

- `Int::to_string` / `StringBuilder::write_*` may not go through
  `moonbit_malloc_inlined` on native — the byte counter will look
  smaller than reality. Cross-check with `time -l` RSS or a wasm
  profile.
- The first sample after a `moon clean` rebuild captures one-time
  init allocations. Throw away the first run if you're measuring
  steady-state.
