# cpuprofile-to-pprof

Convert a Node V8 `.cpuprofile` (the JSON produced by Chrome DevTools /
the Node `--inspect` profiler) into gzip-compressed
[pprof](https://github.com/google/pprof) for analysis with `go tool
pprof`, [pprof](https://github.com/google/pprof), or any pprof viewer.

Used by [`moon-pprof`](https://crates.io/crates/moon-pprof) so the
js-backend of a MoonBit build can be profiled with the same toolchain
as native / wasm / wasm-gc. MoonBit symbols are demangled by default
via [`moonbit-demangle`](https://crates.io/crates/moonbit-demangle); pass
`no_demangle: true` in the options for raw symbols.

## Example

```rust,no_run
let json = std::fs::read_to_string("v8.cpuprofile")?;
let pprof = cpuprofile_to_pprof::convert(&json, &Default::default())?;
std::fs::write("out.pb.gz", pprof)?;
```
