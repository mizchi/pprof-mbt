# Long-running / server profiling

Servers and event-loop programs never `return main`, so the hook's
destructor never fires and you'd get an empty pprof. Two tools
handle this:

## Allocations: `memprofile-native --duration N`

```sh
moon-pprof memprofile-native ./server.exe \
  --duration 10 \
  --out server-mem.pb.gz \
  --sample-rate 100
```

The patched hook installs SIGTERM / SIGINT handlers in its
constructor (only when `MOON_PPROF_RAW_OUTPUT` is set, so plain
`moon build` binaries are unaffected). The driver sends SIGTERM
after N seconds; the handler flushes the raw stream and
`_exit(0)`s before any other atexit runs, so the file is complete
and well-formed when `wait()` returns.

You do **not** need to add anything to the MoonBit server code —
no `@async.with_timeout_opt`, no signal trap. The hook does it.

Verified at 14M allocations over 3 seconds with the loop fixture
(`notes/linux-memprofile/workload-loop/`).

For retained heap in a long-running process, combine the duration
timer with retained mode:

```sh
moon-pprof memprofile-native ./server.exe \
  --duration 10 \
  --retained \
  --sample-rate 1 \
  --out server-retained.pb.gz
go tool pprof -top -inuse_space server-retained.pb.gz
```

This reports allocations still live at the forced exit point. With
`--sample-rate >1`, retained bytes/counts are estimates because only
sampled allocation pointers are tracked.

## CPU: `perf record` with a timeout

`perf record` itself catches SIGTERM cleanly, so:

```sh
timeout --preserve-status -s TERM 10 \
  perf record -F 999 -g --weight -e cpu-clock -o perf.data -- ./server.exe
perf script -i perf.data -F comm,pid,tid,time,event,period,ip,sym,dso > script.out
moon-pprof perf2pprof script.out --out server-cpu.pb.gz
```

The `--preserve-status` flag is important — without it `timeout`
returns 124 which trips a non-zero-exit check upstream.

## Driving load

For a 10-second profile window to mean anything, the server has to
actually do work for that whole window. Three common patterns:

1. **`wrk` driver** (the one from `notes/async-server-alloc-report.md`):
   ```sh
   wrk -t 8 -c 128 -d 8s http://localhost:30001/
   ```
   Start the server first, then start `wrk`, then let the duration
   timer wind down on the server.
2. **`hey`** if wrk isn't on PATH (similar args).
3. **In-process loop** — for non-network workloads, write the
   server to drive itself: `loop { do_work() }` with a sleep
   between iterations.

## Reading server profiles

`summary` on a server profile groups by function across the whole
window, which is what you want — per-request attribution is doable
but rarely useful. If you do need per-req, drop a profile-friendly
marker (e.g. a uniquely-named no-op function called once per
request) so the call stack distinguishes requests.

The async-server report
(`notes/async-server-alloc-report.md`) is a worked example: ~120
allocs/req, ~70 % in async fn coroutine state, with the
data-driven argument for why a compiler-level escape analysis
beats user-level helper refactors.
