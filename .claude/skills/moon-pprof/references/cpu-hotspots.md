# CPU hot-spot identification

`memprofile-native` doubles as a rough CPU profile on native because
alloc count tracks work roughly linearly for allocation-bound
MoonBit code (most of `core` / `json` / `hashmap` lives here).
For pure-compute paths or when you need wall-time precision, use
the per-backend CPU profilers.

## wasm / wasm-gc (wasmtime + GuestProfiler)

```sh
moon build --target wasm-gc --release
moon-pprof profile bench/_build/wasm-gc/release/build/cmd/<name>/main.wasm \
  --out wasm-cpu.pb.gz
moon-pprof summary wasm-cpu.pb.gz
```

Inflates wall time by ~10–15 % vs `--no-profile`. The trade-off is
worth it — the alternative is `--no-profile` for clean wall-time
numbers and a separate run with profiling for attribution.

## js (Node V8 inspector)

```sh
npm run build:js
node runners/v8/run-js.mjs <jsPath> /tmp/js.cpuprofile 1
moon-pprof cpuprofile2pprof /tmp/js.cpuprofile --out js-cpu.pb.gz
```

The runner falls back to `vm.Script` when the moonbit-emitted code
calls `require()` (e.g. `@path`, `@fs`). That's ~5–15× slower wall
time but symbol resolution stays the same.

## native (macOS)

```sh
samply record --save-only -o samply.json -- ./main.exe …args
moon-pprof firefox2pprof --source samply --syms <samply-syms-sidecar> \
  samply.json --out native-cpu.pb.gz
```

samply provides RVA + inline expansion via the syms sidecar.

## native (Linux, no samply)

samply doesn't always work inside containers / restricted hosts.
Use `perf` instead:

```sh
perf record -F 999 -g --weight -e cpu-clock -o perf.data -- ./main.exe
perf script -i perf.data -F comm,pid,tid,time,event,period,ip,sym,dso > script.out
moon-pprof perf2pprof script.out --out native-cpu.pb.gz
```

Required flags:

- `--weight` on `perf record` — without it `perf script` doesn't
  emit a numeric period and the resulting pprof has period=1 per
  sample (no wall-time scale). `moon-pprof perf2pprof` warns on
  this case.
- `period` field on `perf script -F` — same reason.
- `-g` on `perf record` for call graphs; without it you get only
  the top-of-stack and the pprof is mostly useless.

If frames come back as `[unknown]` even though `nm` shows symbols
in the binary, the typical causes are stripping, PIE base
mismatches, or container filesystem path quirks (perf records the
mmap path but later can't open it). `moon-pprof perf2pprof` warns
when > 50 % of frames are unresolved with the three usual fixes
(`cc -g`, same fs view as record, `perf script --symfs=<root>`).

## When to use `--no-profile`

For "is this faster?" wall-time comparisons, run the same workload
N times with `--no-profile` and compare medians. The GuestProfiler
overhead is consistent enough that a profiled-only comparison still
gives the right direction, but the magnitudes will be wrong.
