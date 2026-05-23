## Summary

`immut/sorted_map::make_tree` and `immut/sorted_set::create` compute the new node's `size` as `left.length() + right.length() + 1`. `length()` is a one-match function, but the helpers themselves are called for every internal node produced by `add` / `merge` / `union` / `balance`, so the cost is multiplied across the whole recursion.

Inline the size extraction (same shape as `length()`, just spelled out) and add `#inline`. The change is mechanical; the public APIs are unchanged.

## Benchmarks

Setup: moonbit 0.1.20260522 + this patch, Linux x86_64, `--no-profile` wall time (3-run median).

| workload         | backend  | baseline  | patched   | delta  |
|------------------|----------|----------:|----------:|-------:|
| sorted_map_merge | wasm     | 111.9 ms  | 102.0 ms  | **-8.9%** |
| sorted_map_merge | wasm-gc  |  49.2 ms  |  50.5 ms  |  noise |
| sorted_map_merge | native   |  32.5 ms  |  31.0 ms  | -4.4%  |
| sorted_set_union | wasm     | 120.2 ms  | 111.7 ms  | -7.1%  |
| sorted_set_union | wasm-gc  |  54.8 ms  |  54.8 ms  |  0.0%  |
| sorted_set_union | native   |  30.6 ms  |  29.2 ms  | -4.7%  |

Wins land mainly on wasm (no GC backend; refcount-bound) and native; on wasm-gc the JIT already inlines `length()` well, so the patch is mostly invisible there. Workloads (from <https://github.com/mizchi/pprof-mbt>):

- `sorted_map_merge`: two interleaved 10k-key maps merged 30 times — the "rebuild path" stress case noted in `merge_bench_test.mbt`.
- `sorted_set_union`: same shape, sets instead of maps.

## Test results

| target  | result |
|---------|--------|
| wasm    | 6500 / 6500 pass |
| wasm-gc | 6500 / 6500 pass |
| js      | 6459 / 6459 pass |
| native  | 6411 / 6411 pass |
