# Profile formats

`moon-pprof` normalizes several profiler outputs into pprof and can
also export pprof to formats used by browser-based viewers.

## What `go tool pprof` can read

| Profile | Command | `go tool pprof` view |
|---|---|---|
| CPU / wall time | `profile`, `cpuprofile2pprof`, `firefox2pprof`, `perf2pprof` | default top / flame graph |
| Blocking / off-CPU folded stacks | `folded2pprof wait.folded wait.pb.gz` | `Type: delay` |
| Allocation heap | `heapprofile2pprof`, `memprofile`, `memprofile-native` | `-alloc_space` / `-alloc_objects` |
| Retained native heap | `memprofile-native --retained --sample-rate 1` | `-inuse_space` / `-inuse_objects` |

Examples:

```sh
go tool pprof -http :8000 cpu.pb.gz
go tool pprof -http :8001 -alloc_space wasm-mem.pb.gz
go tool pprof -http :8002 -inuse_space native-retained.pb.gz
go tool pprof -http :8003 folded-delay.pb.gz
```

## Format converters

```sh
# Chrome trace-event JSON with V8 Profile/ProfileChunk -> pprof
moon-pprof chrometrace2pprof trace.json out.pb.gz

# pprof CPU -> synthetic Chrome trace-event JSON
moon-pprof pprof2chrometrace in.pb.gz trace.json

# pprof CPU -> folded stacks, and folded stacks -> pprof delay profile
moon-pprof pprof2folded in.pb.gz out.folded
moon-pprof folded2pprof out.folded folded-delay.pb.gz

# pprof CPU <-> Speedscope sampled JSON
moon-pprof pprof2speedscope in.pb.gz out.speedscope.json
moon-pprof speedscope2pprof out.speedscope.json out.pb.gz
```

`folded2pprof` defaults to `delay/microseconds`, which matches
blocking/off-CPU-style folded stacks. If a folded file represents
something else, override the pprof axis:

```sh
moon-pprof folded2pprof in.folded out.pb.gz \
  --sample-type cpu --unit microseconds
```

## Allocation timeline

`memprofile --trace-out` writes Chrome trace-event JSON:

```sh
moon-pprof memprofile main.wasm \
  --out wasm-mem.pb.gz \
  --trace-out wasm-alloc.trace.json
```

Load `wasm-alloc.trace.json` in Chrome tracing or Perfetto. It
contains allocation counter events (`bytes`, `objects`) plus
per-sampled allocation instant events. It is allocation activity from
the instrumentation hook, not a true runtime GC pause trace.

## Retained heap

Native retained heap is supported by patching generated C allocation
and free paths:

```sh
moon-pprof memprofile-native path/to/cmd.exe \
  --retained \
  --sample-rate 1 \
  --out native-retained.pb.gz
go tool pprof -top -inuse_space native-retained.pb.gz
```

Use `--sample-rate 1` for exact retained heap. Larger sample rates
track only sampled allocation pointers and scale the remaining live
bytes/counts, so they are estimates.
