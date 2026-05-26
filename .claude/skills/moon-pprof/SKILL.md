---
name: "moon-pprof"
description: "Use when investigating MoonBit performance, locating allocation hot spots, comparing baseline vs patched, or profiling MoonBit code across native / wasm-gc / wasm / js backends. Trigger on requests like 'find what's slow / allocating', 'tune this', 'why is X expensive', 'profile this binary', 'where do allocs go', 'does this PR actually improve perf', or whenever a MoonBit benchmark or upstream-core / x / async investigation is on the table. Out of scope: non-MoonBit code that doesn't go through `moon build`."
---

# moon-pprof — MoonBit performance profiler

A single CLI that produces pprof from all four MoonBit backends, plus
allocation profilers for wasm / wasm-gc / native, plus a baseline ↔
patched harness for upstream PR experiments. Built on top of
samply / wasmtime GuestProfiler / Node V8 inspector — the value is
that everything lands in the same pprof schema with MoonBit
demangling applied.

## Decide what you actually need

Don't reach for the heavy tools first. Pick by what you want to learn:

| You want to know | Run |
|---|---|
| "Where does this MoonBit binary spend CPU on native?" | `moon-pprof memprofile-native <exe>` (uses alloc count as a CPU proxy on macOS / Linux) **or** `perf record … && moon-pprof perf2pprof` on Linux |
| "Where does this wasm spend CPU?" | `moon-pprof profile <wasm>` (wasmtime + GuestProfiler) |
| "Where does the js backend spend CPU?" | `runners/v8/run-js.mjs … && moon-pprof cpuprofile2pprof` |
| "What's allocating in wasm / wasm-gc?" | `moon-pprof memprofile <wasm>` |
| "What's allocating in native?" | `moon-pprof memprofile-native <exe>` |
| "Did this patch actually help?" | `moon-pprof summary --diff baseline.pb.gz patched.pb.gz` |
| "Compare baseline vs patched across all backends" | `moon-pprof bench` with `--baseline-moon` / `--patched-moon` / `--mooncakes-baseline` / `--mooncakes-patched` |

When in doubt: **start with `summary`**, drill down only when a hot
site is in question. `go tool pprof -http :8000 <file>` is the
graphical fallback.

## The five canonical workflows

Most past investigations boiled down to one of these. They're
documented in detail in `references/`:

1. **Allocation hunt** (`references/allocation-hunt.md`) — wasm or
   native, find which user-level function the bytes flow into.
2. **CPU hot-spot identification** (`references/cpu-hotspots.md`) —
   wasmtime / samply / perf paths.
3. **Baseline ↔ patched verification** (`references/baseline-patched.md`)
   — try a patch against `moonbitlang/core` or a `.mooncakes/` dep,
   measure, decide whether to PR.
4. **Server / long-running binary** (`references/long-running.md`) —
   `memprofile-native --duration N`, plus `perf record --weight`
   guidance.
5. **Cross-backend bench** (`references/cross-backend-bench.md`) —
   prove the same MoonBit code behaves consistently (or doesn't)
   across native / wasm-gc / wasm / js.

## Optimisation patterns worth knowing

Hard-won from prior PRs to `moonbitlang/core` / `x` / `async` and
the in-repo investigations:

- `for c in self.view(...)` desugars to an `Iter` heap alloc per call.
  Manual UTF-16 loops via `self.unsafe_get(i)` skip it — see
  PR #3635 (`to_lower`).
- `Hash::hash` default implementation allocates a `Hasher` struct.
  Override per-type with the xxHash math inlined — PR #3634
  (-56 % hashset, -98 % hashmap_update).
- `priv struct X { ... }` does *not* automatically stack-allocate.
  Apply `#valtype` to multi-field non-generic priv structs.
- `Option<X>` wraps even when `X` is `#valtype`. NaN sentinel pattern
  (`Double::nan()`) or a dedicated `is_valid` bit avoids the box —
  PR #3633 (`try_fast_double`).
- `async fn` allocates its coroutine state per call on native.
  Wrapping helper `async fn`s can *backfire* (`Sender::write_bytes`
  experiment, net **+10 %** allocs/req). See
  `notes/async-server-alloc-report.md` for the full story.
- `Int::to_string`, `StringBuilder::write_*`, etc. don't always go
  through `moonbit_malloc_inlined`, so `memprofile-native` will
  under-count them. Cross-check with overall RSS or `valgrind`
  callgrind if numbers look suspicious.

## Gotchas before recording

- `memprofile` `--sample-rate N` (N>1) is the right move on large
  workloads — within 0.1 % of exact top sites but 20–70× faster.
- `memprofile-native` on Linux needs the relink output to have
  `-rdynamic -ldl -lpthread` (handled automatically since
  `b115521`) so `dladdr` sees MoonBit symbols.
- `perf record` needs `--weight` for `perf script` to emit periods.
  Without it every sample becomes period=1 and the pprof has no
  wall-time scale. `moon-pprof perf2pprof` warns on this.
- `perf` may show `[unknown]` for MoonBit frames inside Docker
  Desktop (virtiofs path quirk) even though the binary has full
  `.symtab`. Try `perf script --symfs=<root>` or work outside the
  container if possible.
- moon's build dir changed from `target/` to `_build/` — older docs
  may say `target/` for the `.exe` / `.wasm` location.
- `moon clean` between baseline and patched runs is mandatory if you
  swapped toolchains or `.mooncakes/`.

## Reading a summary

`moon-pprof summary <file>` rolls up self time AND classifies
memory-management frames (mimalloc, refcount, dealloc) separately.
The "Memory-management self time" % is how much CPU is in pure
alloc/free plumbing — if it's > 30 %, allocation reduction beats
algorithm tweaks. Past investigations on `core` showed mem-mgmt
self-time of 50–60 % on json / hashmap workloads, which is what
made the Hash + numeric-parse PRs land big numbers.

`summary --diff baseline.pb.gz patched.pb.gz` prints per-function
delta in both directions, sorted by absolute change. Use this
*before* drafting a PR description so the numbers in the description
match what reviewers will see if they re-run.

## When you'd skip moon-pprof entirely

- Micro-bench where wall-clock noise dominates → `hyperfine` /
  `criterion` instead.
- Suspected runtime / mimalloc bug → `valgrind callgrind` for an
  instruction-count view that's invariant under load.
- "Why does this fail to compile" type questions → not a pprof
  question, drop the skill.
