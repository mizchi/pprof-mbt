# firefox-to-pprof

Convert [Firefox Profiler "processed profile" JSON][1] into gzip'd
[pprof][2].

[1]: https://github.com/firefox-devtools/profiler/blob/main/docs-developer/processed-profile-format.md
[2]: https://github.com/google/pprof/blob/main/proto/profile.proto

## Why

Several profilers emit the same Firefox-Profiler JSON format:

- [samply](https://github.com/mstange/samply) for native code
- [wasmtime](https://wasmtime.dev/)'s `GuestProfiler` for wasm
- (potentially) any other tool that targets `profiler.firefox.com`

All of them are usable from `go tool pprof` only after a JSON → pprof
conversion. This crate is that converter, with the producer-specific bits
moved behind two small traits.

## Usage

```rust
use firefox_to_pprof::{Builder, FirefoxProfile, FuncTableResolver, SampleWeighting};

let json = std::fs::read("wasmtime-guest.json")?;
let profile: FirefoxProfile = serde_json::from_slice(&json)?;

let bytes = Builder::new(
    &profile,
    FuncTableResolver,                                          // wasmtime: names already in funcTable
    SampleWeighting::PerSampleTimeDeltas { default_interval_ns: 1_000_000 },
)
.encode()?;
std::fs::write("wasmtime-guest.pb.gz", bytes)?;
```

For samply-style address lookups against a `.syms.json` sidecar, implement
[`FrameResolver`] yourself — see the `wasmtime-runner` crate in this repo
for a worked example.

## What you get

- All function names pass through [`moonbit_demangle`][md] by default;
  swap in your own with `Builder::demangle_with`.
- Content-based Location dedup: two distinct frame ids with identical
  resolved content collapse into one pprof Location.
- Pure Rust, no `protoc` runtime dependency for callers (we compile the
  vendored `pprof.proto` at build time).

[md]: https://crates.io/crates/moonbit-demangle

## License

Apache-2.0
