# heapprofile-to-pprof

Convert a Node V8 `.heapprofile` (sampling allocation profile from
`--heapsnapshot-near-heap-limit` / `Sampling Heap Profiler`) into
gzip-compressed [pprof](https://github.com/google/pprof) with
`alloc_objects` / `alloc_space` sample types.

Used by [`moon-pprof`](https://crates.io/crates/moon-pprof) to surface
js-backend allocation hotspots alongside the wasm / wasm-gc / native
allocation profilers. MoonBit symbols are demangled by default via
[`moonbit-demangle`](https://crates.io/crates/moonbit-demangle).

## Example

```rust,no_run
let json = std::fs::read_to_string("v8.heapprofile")?;
let pprof = heapprofile_to_pprof::convert(&json, &Default::default())?;
std::fs::write("out.pb.gz", pprof)?;
```
