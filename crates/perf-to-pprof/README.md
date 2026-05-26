# perf-to-pprof

Convert Linux `perf script` textual output into gzip'd
[pprof](https://github.com/google/pprof). Used to round out
cross-backend profiling on Linux where
[samply](https://github.com/mstange/samply) isn't an option (e.g.
inside containers, on kernels without certain perf features, or
when you're stuck with `perf` because that's what the host has).

Standalone — does not require the rest of the `moon-pprof` workspace
beyond the in-tree pprof proto. MoonBit symbol demangling is
applied automatically; non-MoonBit symbols pass through unchanged.

## Recommended capture recipe

```sh
# 999 Hz CPU sampling, call graphs via frame pointers (or `--call-graph dwarf`).
# `--weight` is what makes `perf script` emit the per-sample period — without
# it every sample shows up with period=1 and the resulting pprof has no
# wall-time scale.
perf record -F 999 -g --weight -e cpu-clock -o perf.data -- ./main.exe

# `period` in `-F` is required to surface the recorded weight in the textual
# output. The other fields are the minimum perf-to-pprof needs.
perf script -i perf.data -F comm,pid,tid,time,event,period,ip,sym,dso > script.out
```

Then either use the library directly:

```rust
use perf_to_pprof::{convert, ConvertOptions};
let pprof = convert(&std::fs::read_to_string("script.out")?, &Default::default())?;
std::fs::write("perf.pb.gz", pprof)?;
```

…or use the `moon-pprof perf2pprof` CLI which wraps it:

```sh
moon-pprof perf2pprof script.out --out perf.pb.gz
moon-pprof summary perf.pb.gz
go tool pprof -http :8000 perf.pb.gz
```

## Symbol resolution notes

`perf` resolves symbols by opening the DSO referenced in each
sample's `MMAP` event and reading its `.symtab` / `.dynsym`. There
are two ways frames come back as `[unknown]`:

1. **Stripped binaries.** `moon build --release` ships a not-stripped
   binary with `.symtab` populated, so MoonBit symbols (`_M0FP…`)
   should resolve. If they don't…
2. **Container / cross-VM path mismatch.** Inside Docker Desktop the
   binary's mmap path looks like `/run/host_virtiofs/<host-path>`,
   and some `perf` builds can't always open it post-hoc. Either run
   `perf script` inside the same container that recorded the
   profile, or use `perf script --symfs=/path/to/symbol/root`.

If frames stay `[unknown]` after that, the fallback is to
post-process the perf output with `addr2line -e <binary> 0xADDR…`
before piping it into this crate.
