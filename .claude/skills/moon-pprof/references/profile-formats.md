# Profile format conversion

Use this when the user wants a profile visible in another UI, asks
"can pprof see this?", or hands over Chrome trace / Speedscope /
folded stack / V8 / perf artifacts.

## UI selection

| Desired UI | Preferred file | Command |
|---|---|---|
| `go tool pprof` CPU | pprof `cpu` / `wall` | native `profile`, `cpuprofile2pprof`, `firefox2pprof`, `perf2pprof` |
| `go tool pprof` blocking/off-CPU | pprof `delay` | `moon-pprof folded2pprof wait.folded wait.pb.gz` |
| `go tool pprof` allocation | pprof `alloc_space` | `heapprofile2pprof`, `memprofile`, `memprofile-native` |
| `go tool pprof` retained heap | pprof `inuse_space` | `memprofile-native --retained --sample-rate 1` |
| Chrome / Perfetto timeline | trace-event JSON | `memprofile --trace-out alloc.trace.json` |
| Speedscope | Speedscope JSON | `pprof2speedscope` or `speedscope2pprof` |

## Commands

```sh
# Chrome trace-event JSON containing V8 Profile/ProfileChunk -> pprof
moon-pprof chrometrace2pprof trace.json out.pb.gz

# pprof CPU -> synthetic Chrome trace-event V8 Profile/ProfileChunk
moon-pprof pprof2chrometrace in.pb.gz trace.json

# pprof CPU -> folded stacks, and folded stacks -> delay pprof
moon-pprof pprof2folded in.pb.gz out.folded
moon-pprof folded2pprof out.folded delay.pb.gz

# pprof CPU <-> Speedscope sampled JSON
moon-pprof pprof2speedscope in.pb.gz out.speedscope.json
moon-pprof speedscope2pprof out.speedscope.json out.pb.gz
```

## Sample type conventions

- `folded2pprof` defaults to `delay/microseconds`. `go tool pprof`
  shows `Type: delay`; this is suitable for off-CPU / blocking folded
  input, not for allocation data.
- `memprofile` and default `memprofile-native` emit
  `alloc_objects/count` + `alloc_space/bytes`.
- `memprofile-native --retained` emits `inuse_objects/count` +
  `inuse_space/bytes`; use `go tool pprof -inuse_space`.
- `memprofile --trace-out` is not a pprof. Load it in Chrome tracing
  or Perfetto. It records allocation activity from the hook and does
  not imply true GC pause events.

## Verification snippets

```sh
go tool pprof -top delay.pb.gz
go tool pprof -top -alloc_space wasm-mem.pb.gz
go tool pprof -top -inuse_space native-retained.pb.gz
node -e 'const j=require("./alloc.trace.json"); console.log(j.traceEvents.length)'
```
