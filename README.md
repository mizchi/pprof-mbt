# pprof-mbt

A profiling toolkit for [MoonBit](https://www.moonbitlang.com/) that builds
the same code across `native` / `wasm-gc` / `wasm` / `js` backends and
normalises every run into the [pprof](https://github.com/google/pprof)
format.

> Japanese version: [README-ja.md](README-ja.md).

## Install

> The fastest path is [`docs/quick-start.md`](docs/quick-start.md) —
> install the CLI, profile the bundled sample wasm, read the summary in
> under a minute.

If you just want the CLI (any wasm → `profile` / `summary` /
`cpuprofile2pprof` / `firefox2pprof`):

```sh
# cargo (requires rustc 1.80+ and `protoc` on PATH)
cargo install --git https://github.com/mizchi/pprof-mbt moon-pprof --locked

# nix (build-time deps live inside the flake)
nix run github:mizchi/pprof-mbt -- --help
nix profile install github:mizchi/pprof-mbt           # persistent install
```

`moon-pprof bench` is the only subcommand that needs external `moon` /
`node` / `samply` at runtime — see the Quickstart below, or `nix develop`
to pull them all in.

## What's inside

The MoonBit-facing parts:

- A single `moon-pprof` CLI for `profile` / `summary` / `bench` (plus
  `cpuprofile2pprof` / `firefox2pprof` converters).
- Demangles MoonBit symbols and lines up all four backends in the same
  pprof schema.
- **baseline ↔ patched comparison workflow** for upstream PR experiments
  (`patched-toolchain` / `patched-mooncakes` / `moon-pprof bench`).

The **internal libraries are MoonBit-agnostic**. The Rust crates
`firefox-to-pprof` / `cpuprofile-to-pprof` / `wasmtime-guest-pprof` work
unchanged for AssemblyScript / Rust / Zig wasm too.
[→ Details](#reusing-for-non-moonbit-wasm)

## Quickstart (cloning the repo)

Inside `nix develop`,
[moonbit-overlay](https://github.com/moonbit-community/moonbit-overlay)
brings in `moon`; the shell also has Node.js, Rust, wasmtime, samply,
wabt, protobuf, and graphviz. (`go` is included only for `go tool pprof`
visualisation — there's no Go code in this repo.)

```sh
nix develop
cargo build --workspace --release
mkdir -p .bin && cp \
  target/release/moon-pprof \
  target/release/http-baseline-server \
  runners/patched-toolchain \
  runners/patched-mooncakes .bin/
chmod +x .bin/patched-toolchain .bin/patched-mooncakes
npm install
```

Capture your first profile:

```sh
npm run build:wasm-gc && npm run profile:wasm-gc     # → wasm-gc.pb.gz
.bin/moon-pprof summary wasm-gc.pb.gz                # Top-N in the terminal
go tool pprof -http :8000 wasm-gc.pb.gz              # browser UI
```

## CLI

### `moon-pprof` — unified CLI

| Subcommand | Purpose |
|---|---|
| `moon-pprof profile <wasm>` | Run a wasm under `wasmtime + GuestProfiler`, write gzip'd pprof. |
| `moon-pprof profile --wasm-gc <wasm>` | Same wasmtime path for wasm-gc binaries (Cranelift baseline, not V8). |
| `moon-pprof profile --no-profile <wasm>` | Wall-time only, no GuestProfiler overhead. |
| `moon-pprof summary <file>` | Top-N self-time + mem-mgmt rollup (CPU profiles); bytes / count per site (heap profiles — auto-detected from sample_type, honors pprof's `drop_frames`). |
| `moon-pprof summary --diff <a> <b>` | Per-function delta (improved / regressed / new / gone). Bytes-formatted when both inputs are heap profiles. |
| `moon-pprof bench` | Multi-workload × multi-backend, baseline ↔ patched, markdown table out. |
| `moon-pprof cpuprofile2pprof <in> <out>` | V8 `.cpuprofile` → pprof gzip (with MoonBit demangle by default; `--no-demangle` to disable). |
| `moon-pprof heapprofile2pprof <in> <out>` | V8 `.heapprofile` (sampling allocations) → pprof gzip with `alloc_objects` / `alloc_space` sample types. |
| `moon-pprof memprofile <wasm>` | Allocation profile via wasm instrumentation. wasm (non-gc): wraps `moonbit.malloc` (covers raw + `moonbit.gc.malloc`). wasm-gc: rewrites every `struct.new` / `array.new*` opcode so the host hook fires with the alloc size. |
| `moon-pprof firefox2pprof <in> <out>` | Firefox Profiler JSON → pprof. `--source samply --syms <sidecar>` for samply (RVA + inline expansion), default `--source wasmtime-guest` for wasmtime guest output. |

`--mem-pattern <regex>` overrides the `summary` mem-mgmt classifier
(default is MoonBit's). `moon-pprof bench` supports two orthogonal
baseline/patched axes: `--baseline-moon` / `--patched-moon` (core
toolchain swap) and `--mooncakes-baseline` / `--mooncakes-patched`
(registry dep swap).

### Helper tools

| Tool | Purpose |
|---|---|
| `patched-toolchain` | Snapshot `~/.moon` to `/tmp`, apply a diff, rebundle every backend (used for core PRs). |
| `patched-mooncakes` | Snapshot `<bench-dir>/.mooncakes/` to `/tmp` and restore (used for registry-dep PRs). |
| `http-baseline-server` | Empty-handler HTTP on port 30003 (axum), baseline for k6 comparisons. |
| `node runners/v8/run-wasm-gc.mjs <wasm>` | Run wasm-gc under Node V8 and emit `.cpuprofile` (`--no-profile` for wall time). Used when you want the V8 numbers that the default wasmtime path won't show. |
| `node runners/v8/run-js.mjs <js>` | Run the js backend under Node V8 (V8 required). |

### Typical workflow: writing an improvement PR

`moonbitlang/core` patches (rewrite the core under `~/.moon`) —
reproducing the bigint PR:

```sh
.bin/patched-toolchain init
.bin/patched-toolchain apply notes/pr-drafts/01-bigint-mul-single-limb/patch.diff
.bin/patched-toolchain rebundle
.bin/moon-pprof bench --workloads bigint_ops,bigint_square --runs 3
```

`moonbitlang/x` patches (rewrite a registry dep) — reproducing the uuid PR:

```sh
.bin/patched-mooncakes init bench-x
cp -r /tmp/pprof-mbt-mooncakes/bench-x /tmp/pprof-mbt-mooncakes/bench-x.patched
( cd /tmp/pprof-mbt-mooncakes/bench-x.patched/moonbitlang/x \
  && patch -p1 < $(pwd)/notes/x-pr-drafts/04-uuid-tostring-inplace/patch.diff )

.bin/moon-pprof bench \
  --bench-dir bench-x \
  --backends native,wasm-gc,js \
  --workloads uuid_parse \
  --mooncakes-baseline /tmp/pprof-mbt-mooncakes/bench-x \
  --mooncakes-patched /tmp/pprof-mbt-mooncakes/bench-x.patched \
  --runs 3
```

→ The markdown table shows native -64% / wasm-gc -45% / js -39%.

## Per-backend profiling

| backend | profile source | sampling | pprof emission |
|---------|---------------|----------|----------------|
| `wasm-gc` (default) | wasmtime `GuestProfiler` (Cranelift) | epoch-tick sampling | `firefox-to-pprof` crate |
| `wasm-gc` (`--via-v8`) | Node inspector (V8) | V8 sampling | `cpuprofile-to-pprof` crate |
| `js`      | Node inspector (V8) | V8 sampling | `cpuprofile-to-pprof` crate |
| `wasm`    | wasmtime `GuestProfiler` (Cranelift JIT) | epoch-tick sampling | `firefox-to-pprof` crate |
| `native`  | samply (Mach-O / ELF) | OS sampling | `firefox-to-pprof::samply` + `firefox-to-pprof` crate |

Every path routes mangled names (e.g. `_M0FP26mizchi5bench9ackermann`)
through the shared demangler to land as `mizchi::bench::ackermann` in
the final pprof.

### wasm-gc (wasmtime, default)

```sh
npm run build:wasm-gc && npm run profile:wasm-gc
```

`moon build --no-strip --target=wasm-gc` keeps function names in the
wasm; `moon-pprof profile --wasm-gc` loads it with `Config::wasm_gc(true)`
+ `wasm_function_references(true)` + `wasm_reference_types(true)` and
samples via GuestProfiler on every epoch tick, then converts to gzip'd
pprof. Host imports (`spectest.print_char` / `wasi fd_write`) come from
the `moonbit-wasm-host` crate in one call.

If you want the wall-time that V8's inline cache hands you, the V8 path
is still around:

```sh
npm run profile:wasm-gc:v8   # legacy Node V8 inspector path (for comparison)
# or:
.bin/moon-pprof bench --backends wasm-gc --wasm-gc-via-v8 ...
```

wasmtime (Cranelift baseline) and V8 (inline-cache-armed) produce
different self-time distributions for the same wasm-gc binary. The
hot-path topology agrees (which functions matter), the absolute numbers
don't — treat them as separate signals.

Note: wasm-gc allocations go through wasm GC instructions (`struct.new`
etc.), so the `--mem-pattern` mem-mgmt classifier doesn't see them. To
track GC overhead you'd need instrumentation at the wasm-instruction
level (out of scope for now).

### js (Node)

```sh
npm run build:js && npm run profile:js
```

MoonBit's JS backend emits mangled symbols verbatim as JS function names
(`function _M0FP26mizchi5bench3fib(n) {...}`). Node's inspector picks
them up directly, so the same converter as wasm-gc applies.

### Memory profiling (js)

```sh
npm run build:js && npm run profile:js:heap
go tool pprof -alloc_space -http :8000 js-heap.pb.gz
```

Drives Node's `HeapProfiler.startSampling` (V8 sampling allocation
profiler), writes a `.heapprofile`, and `moon-pprof heapprofile2pprof`
turns it into a pprof with two sample types: `alloc_objects` (count)
and `alloc_space` (bytes). Demangling reuses `moonbit-demangle`, so
`mizchi::bench::mandel__sum` appears as the top allocator just like in
the CPU view.

V8's default sampling interval is 16 KiB. Pass
`node runners/v8/run-js-heap.mjs … --interval <bytes>` to tighten or
loosen it.

`moon-pprof summary` currently mislabels heap values as nanoseconds
(it's hard-coded for CPU). Use `go tool pprof` for the GUI / top view
until it learns to read `period_type`.

### Memory profiling (wasm and wasm-gc)

```sh
# wasm (non-gc)
npm run build:wasm
.bin/moon-pprof memprofile bench/_build/wasm/release/build/cmd/main/main.wasm \
  --out wasm-mem.pb.gz

# wasm-gc
npm run build:wasm-gc
.bin/moon-pprof memprofile bench/_build/wasm-gc/release/build/cmd/main/main.wasm \
  --out wasm-gc-mem.pb.gz

go tool pprof -alloc_space -http :8000 wasm-mem.pb.gz
```

Same subcommand handles both backends. It rewrites the wasm with
[walrus](https://docs.rs/walrus) to add a host import
`moonbit_profile.alloc_hook(size: i32)` and inject calls to it at
every allocation point:

- **wasm (non-gc)**: prepend `local.get 0; call $hook` to the body of
  `$moonbit.malloc`. `$moonbit.gc.malloc` internally calls
  `$moonbit.malloc(n+8)`, so this one wrap covers raw and
  refcount-managed allocations.
- **wasm-gc**: rewrite every alloc opcode. Static-size cases
  (`struct.new`, `struct.new_default`, `array.new_fixed`) get a
  `i32.const <size>; call $hook` prefix. Dynamic-size cases
  (`array.new`, `array.new_default`, `array.new_data`,
  `array.new_elem`) save the length to a scratch local, push
  `len * elem_size`, call the hook, push the length back, then run the
  original opcode.

At run time the hook captures the current wasm call stack with
`wasmtime::WasmBacktrace::force_capture` and accumulates
`(stack → (count, bytes))`. The pprof's `drop_frames` field hides
`moonbit.malloc` / `moonbit.gc.malloc` so user code is the visible
leaf.

The two backends agree on totals (e.g. ~24.9 MB on the bigint_square
workload), but the **shape differs**:

- wasm non-gc attributes ~99% to the runtime helper
  `moonbit.i32_array_make_raw` (the real `malloc` site), with user
  functions visible via cum%.
- wasm-gc attributes directly to `Add::add` (9 MB) / `Shl::shl` (6 MB)
  / `BigInt::split` (4 MB) etc, because the alloc opcode lives at the
  user site, not in a runtime helper.

Caveat: wasm-gc sizes are a **field-sum proxy**, not the exact
wasmtime GC heap consumption — alignment, object headers, and
ref-slot sizing are engine-defined. The totals are an attribution
signal, not a precise byte count.

#### Sampling for big workloads

The host hook calls `WasmBacktrace::force_capture` on every alloc by
default, which dominates wall time on heavy workloads — a 50-iteration
parse of a 197 KB JSON triggers ~13 M allocs and takes ~8 minutes per
run. Pass `--sample-rate <N>` to capture a stack only every Nth alloc;
the sample is then credited with `N × size` bytes so totals stay
comparable.

```sh
# baseline + patched + diff in ~45 s (vs ~15 min without sampling)
moon-pprof memprofile baseline.wasm --sample-rate 100 --out base.pb.gz
moon-pprof memprofile patched.wasm  --sample-rate 100 --out patched.pb.gz
moon-pprof summary --diff base.pb.gz patched.pb.gz
```

Empirically, on JSON-parse-scale workloads (~13 M allocs) `--sample-rate 100`
matches the unsampled top-site bytes to within 0.1 % and the total
within ~1 %. On smaller workloads (~100 k allocs, e.g. bigint) the
total can drift by ~20 % at `--sample-rate 100` because the law of
large numbers hasn't kicked in — drop to `--sample-rate 10` (still
~2× faster) or `1` (exact).

The sampler is **deterministic 1/N**, not Poisson: rerunning the same
wasm at the same rate produces byte-identical pprof output, but a
workload that allocates A,B,A,B,… correlated with the sampling stride
will be biased. For exact answers, use `--sample-rate 1` on a shorter
iteration count.

### wasm (wasmtime + GuestProfiler)

```sh
npm run build:wasm && npm run profile:wasm
```

The Rust API equivalent of wasmtime CLI's `--profile=guest`: run the
wasm at Cranelift JIT speed, bump `engine.increment_epoch()` from a
side thread, and let `GuestProfiler::sample` fire inside
`epoch_deadline_callback`. The Firefox JSON output is then folded into
pprof + gzip by the `firefox-to-pprof` crate. Host imports come from
`moonbit-wasm-host`.

### native (via samply)

```sh
npm run build:native && npm run profile:native
```

samply records an OS-level sampling profile in Firefox Profiler format.
`--unstable-presymbolicate` produces a `.syms.json` sidecar with
per-symbol info; `moon-pprof firefox2pprof --source samply --syms
<sidecar>` then converts to pprof (inline frames expanded). RVA →
enclosing-symbol binary search is handled by
`firefox-to-pprof::samply::SamplySymsResolver`.

## Using as a library

The Rust crates and the npm package are independently usable from
external projects.

### Rust

```toml
[dependencies]
moonbit-demangle      = "0.1"
firefox-to-pprof      = "0.1"  # generic: samply / wasmtime JSON → pprof
cpuprofile-to-pprof   = "0.1"  # generic: V8 .cpuprofile → pprof
wasmtime-guest-pprof  = "0.1"  # generic: drop into a wasmtime app
moonbit-wasm-host     = "0.1"  # registers the moonbit wasm host imports in one call
```

```rust
use moonbit_demangle::demangle;
assert_eq!(demangle("_M0FP26mizchi5bench9ackermann"), "mizchi::bench::ackermann");
```

### JavaScript

```js
import {
  moonbitWasmImports,
  autoStubMissing,
} from "@mizchi/moonbit-wasm-host";
```

> The pprof / firefox / cpuprofile / demangle utilities have moved to
> Rust crates. From the CLI use `moon-pprof cpuprofile2pprof` /
> `moon-pprof firefox2pprof`. What stays on the npm side is just the
> host import (`spectest.print_char` / WASI `fd_write`) used to run a
> MoonBit wasm under Node V8.

## Reusing for non-MoonBit wasm

The library layer is not MoonBit-specific. If you want to pprof-profile
a Rust / AssemblyScript / Zig wasm:

**Rust (run via wasmtime + Cranelift JIT)**:

```rust
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_guest_pprof::{ProfileSession, ProfilerHost, ProfilerHostExt as _, TakeProfileSession};

// Skip the MoonBit-only bits:
// - replace `moonbit-demangle` with your own demangler (or identity)
// - skip `moonbit-wasm-host`, register your app's own host imports
//
// `wasmtime-guest-pprof` + `firefox-to-pprof` work unchanged.
```

`firefox-to-pprof::Builder::demangle_with()` swaps the symbol decoder:

```rust
let bytes = firefox_to_pprof::Builder::new(&profile, frames, samples)
    .demangle_with(|s| my_demangle(s))   // any language is fine
    .encode()?;
```

**Node / V8 `.cpuprofile`**:

CLI:

```sh
moon-pprof cpuprofile2pprof --no-demangle in.cpuprofile out.pb.gz
```

As a library, the `cpuprofile-to-pprof` crate:

```rust
use cpuprofile_to_pprof::{Builder, CpuProfile};
let profile: CpuProfile = serde_json::from_slice(&bytes)?;
let out = Builder::new(profile)
    .demangle_with(|s| s.to_string())  // disable moonbit demangle
    .encode()?;
std::fs::write("out.pb.gz", out.encoded)?;
```

**`summary` mem-mgmt classifier**:

`moon-pprof summary --mem-pattern <regex>` (or
`$PPROF_SUMMARY_MEM_PATTERN`) replaces the MoonBit-specific defaults
(`moonbit_drop_object` etc.) with any runtime-primitive regex.

## Layout

```
Cargo.toml                              ← Rust workspace
package.json                            ← npm workspace (workspaces: packages/*)

crates/                                 published libraries (Rust)
├── moonbit-demangle/                   mangled symbol → readable name
├── moonbit-wasm-host/                  moonbit wasm host imports (spectest / WASI)
├── firefox-to-pprof/                   Firefox Profiler JSON → pprof (generic)
├── cpuprofile-to-pprof/                V8 .cpuprofile → pprof (generic)
└── wasmtime-guest-pprof/               wasmtime GuestProfiler driver + pprof (generic)

packages/                               published library (npm)
└── moonbit-wasm-host/                  @mizchi/moonbit-wasm-host (Node V8 host imports for moonbit wasm)

runners/                                CLIs / binaries
├── moon-pprof/                         Rust unified CLI
├── http-baseline-server/               Rust (axum + tokio), k6 baseline
├── patched-toolchain                   bash, ~/.moon snapshot / patch / rebundle
├── patched-mooncakes                   bash, .mooncakes/ snapshot / patch / restore
└── v8/                                 Node V8 inspector paths
    ├── run-wasm-gc.mjs                 wasm-gc under V8 (--via-v8)
    └── run-js.mjs                      js under V8
                                        (.cpuprofile → pprof: moon-pprof cpuprofile2pprof;
                                         samply / wasmtime guest JSON → pprof: moon-pprof firefox2pprof)

bench/                                  MoonBit bench workloads (ackermann / fib / mandel)
bench-async/                            moonbitlang/async investigation (coroutine / HTTP server)
bench-x/                                moonbitlang/x investigation (uuid / base64 / encoding / ...)
notes/                                  investigation logs + upstream-PR materials
```

## Bench code

`bench/bench.mbt` contains three CPU-bound workloads invoked from
`bench/cmd/main/main.mbt`:

- `ackermann(3, 10)` — deep recursion
- `fib(32)` — classic recursion
- `mandel_sum(160, 500)` — nested loop + floats

Same code across every backend.

`bench-async/` (moonbitlang/async investigation) and `bench-x/`
(moonbitlang/x investigation) carry their own workloads. See
[`notes/async_investigation.md`](notes/async_investigation.md) and
[`notes/x_investigation.md`](notes/x_investigation.md) for context.

## Investigation logs / upstream patches

`notes/` holds the patch experiments derived from profiles, plus
upstream-PR drafting material.

### `moonbitlang/core`

| Doc | Contents |
|---|---|
| [`notes/data_structures_comparison.md`](notes/data_structures_comparison.md) | 14 workload × 4 backend cross-measurement (refcount hypothesis verification). |
| [`notes/patch_experiments.md`](notes/patch_experiments.md) | 10 patch experiments (7 adopted / 1 still under discussion / 2 rejected). |
| [`notes/pr_numbers.md`](notes/pr_numbers.md) | Clean per-PR numbers measured with `--no-profile`. |
| [`notes/pr_plan.md`](notes/pr_plan.md) | Overlap check against existing upstream PRs/Issues + submission plan. |
| [`notes/pr-drafts/`](notes/pr-drafts/) | PR materials targeting moonbitlang/core (4 PRs + 1 Issue). |

### `moonbitlang/async`

| Doc | Contents |
|---|---|
| [`notes/async_investigation.md`](notes/async_investigation.md) | Profiles via callgrind + 2 patches. |
| [`notes/async_http_server_profile.md`](notes/async_http_server_profile.md) | k6 + callgrind measurement of the HTTP server. |
| [`notes/async_backend_comparison.md`](notes/async_backend_comparison.md) | 4-backend comparison. |
| [`notes/async-pr-drafts/`](notes/async-pr-drafts/) | PR materials targeting moonbitlang/async (1 PR). |

### `moonbitlang/x`

| Doc | Contents |
|---|---|
| [`notes/x_investigation.md`](notes/x_investigation.md) | Profiles + 6 patches. |
| [`notes/x_cross_backend.md`](notes/x_cross_backend.md) | Cross-verification of patches across native / wasm-gc / js. |
| [`notes/x-pr-drafts/`](notes/x-pr-drafts/) | PR materials targeting moonbitlang/x (6 PRs). |

### This repo's own roadmap

| Doc | Contents |
|---|---|
| [`notes/pprof_mbt_roadmap.md`](notes/pprof_mbt_roadmap.md) | v1 roadmap (right after the core investigation). |
| [`notes/pprof_mbt_roadmap_v2.md`](notes/pprof_mbt_roadmap_v2.md) | v2 (updated after the async + x investigations). |

## Known limitations / TODO

- **Memory profiling: js, wasm, and wasm-gc.** `moon-pprof
  heapprofile2pprof` converts a Node V8 sampling allocation profile
  (see [Memory profiling (js)](#memory-profiling-js)). `moon-pprof
  memprofile` instruments allocation sites in either wasm backend (see
  [Memory profiling (wasm and wasm-gc)](#memory-profiling-wasm-and-wasm-gc)) —
  wasm wraps `moonbit.malloc`, wasm-gc rewrites every `struct.new` /
  `array.new*` opcode. wasm-gc sizes are a field-sum proxy. Native is
  still CPU-only (would need samply + LD_PRELOAD / heaptrack).
- **The demangler is heuristic.** Impl / method / generic markers
  (`_M0I…`, `_M0M…`, `GsE`/`GuE` suffixes) are only partially decoded —
  for example, the `core::` prefix on stdlib methods is dropped.
- **The `llvm` backend** (`moon build --target=llvm`) hasn't been
  verified — a MoonBit-side build error blocks it.
- **Native profiling on Linux** (samply-equivalent) depends on the
  environment. A `perf` → pprof converter (`perf-to-pprof` crate) is
  on the TODO list in
  [`notes/pprof_mbt_roadmap_v2.md`](notes/pprof_mbt_roadmap_v2.md).

## License

Apache-2.0
