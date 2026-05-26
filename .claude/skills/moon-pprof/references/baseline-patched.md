# Baseline ↔ patched verification

When you have a candidate optimization (a diff against
`moonbitlang/core` or a registry dep), measure before opening a
PR. The number you put in the description has to survive a
re-runner — that means the baseline and patched runs must use
identical workloads, harness, and reporting.

## Two orthogonal axes

`moon-pprof bench` supports both:

- `--baseline-moon` / `--patched-moon` — swap the core toolchain.
  Backed by `runners/patched-toolchain` which snapshots `~/.moon`
  to `/tmp/moonbit-patched`, applies a diff, and rebundles all
  backends. Use for `moonbitlang/core` patches.
- `--mooncakes-baseline` / `--mooncakes-patched` — swap per-project
  registry deps (`.mooncakes/<owner>/<pkg>/`). Backed by
  `runners/patched-mooncakes`. Use for `moonbitlang/x`,
  `moonbitlang/async`, etc.

## Quick recipe

```sh
# Snapshot baseline state once. Both helpers are at .bin/ after the
# usual `cargo build --release && cp target/release/... .bin/`.
.bin/patched-toolchain init                       # for core diffs (snapshot $HOME/.moon → /tmp/moonbit-patched)
.bin/patched-mooncakes init <bench-dir>           # for registry-dep diffs (snapshot <bench-dir>/.mooncakes/)

# Apply your patch in-place. patched-toolchain edits /tmp/moonbit-patched/lib/core;
# patched-mooncakes edits <bench-dir>/.mooncakes/<pkg> directly.
.bin/patched-toolchain apply /path/to/patch.diff
.bin/patched-toolchain rebundle                   # required after .core edits — rebuild bundles for all backends
# or for a registry dep:
.bin/patched-mooncakes apply <bench-dir> moonbitlang/x /path/to/patch.diff

# Drive the bench. Output is markdown by default; redirect to a file.
moon-pprof bench \
  --baseline-moon ~/.moon \
  --patched-moon /tmp/moonbit-patched \
  --bench-dir bench \
  --backends native,wasm-gc \
  --workloads json_parse,json_stringify \
  --runs 3 \
  > delta.md

# Clean up:
.bin/patched-toolchain reset            # rm -rf /tmp/moonbit-patched
.bin/patched-mooncakes restore <bench-dir>   # copy snapshot back to live
.bin/patched-mooncakes reset <bench-dir>     # rm -rf the scratch snapshot
```

Note the asymmetry: `patched-toolchain reset` deletes the patched copy
(baseline lives at `~/.moon` untouched), while `patched-mooncakes
restore` copies the snapshot back into the live `.mooncakes/` because
the patch was applied in-place to it.

## Diffing two raw pprof files

When you don't need the full bench harness — e.g. you took a wasm
profile twice (once with each toolchain swapped in manually):

```sh
moon-pprof summary --diff baseline.pb.gz patched.pb.gz
```

Prints per-function delta sorted by absolute change, both directions.
Use this output verbatim in PR descriptions so reviewers see the
same numbers.

## When the numbers move the wrong way

This happens more often than you'd think. Examples from past work:

- Adding a helper `async fn Sender::write_bytes` to dedupe call sites
  in `moonbitlang/async`'s HTTP server *increased* per-req allocs by
  ~10 % — every async fn allocates its own coroutine state on
  native, so the helper added one alloc per call without saving
  any. Reverted, not submitted.
- A `#valtype` annotation on a single-element newtype was rejected
  by the compiler ("already guaranteed unboxed at runtime"), but
  the equivalent change on a multi-field priv struct worked and
  saved ~40 % on the json number parser hot path (PR #3633).

Rule of thumb: a patch that doesn't show in `summary --diff` shouldn't
go upstream. Skip the wishful-thinking PRs.

## Run hygiene

- `moon clean` between every measurement. moon's incremental
  rebuild may keep stale `.core` files when the toolchain changes.
- Warm-up runs matter for short benches (~20 ms). For now, run 3+
  and take the median; `--warmup N --measure M` is on the roadmap.
- Pin background load — close the browser tab that's running an
  npm dev server, etc. wall-time numbers move 5–10 % under load.
