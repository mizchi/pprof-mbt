## Summary

`HashMap::grow`, `HashSet::grow`, and builtin `Map::grow` currently rehash entries by re-calling the public `set_with_hash` / `add_with_hash` path. Inside that path two checks are dead weight during a rehash:

1. The **key equality check** (`curr_entry.hash == hash && curr_entry.key == key`) — the entries being re-inserted came from an old table where they were already unique by construction, so this can never match.
2. The **load-factor / `grow_at` check** — the capacity just doubled, so the load factor is 50% by construction and no further grow is needed.

Replace each of the three growth helpers with a specialized Robin-Hood swap loop that skips both checks. This also lets the function drop the `Eq` constraint that was only needed for the (never-firing) equality check.

For builtin `Map` (the linked hash map used by `@json`) the change is slightly more careful: it has to preserve the `prev` / `next` linked-list invariants, so it keeps calling `add_entry_to_tail` / `push_away` rather than inlining a fresh probe.

## Benchmarks

Setup: moonbit 0.1.20260522 + this patch, Linux x86_64, `--no-profile` wall time (3-run median).

| workload         | backend  | baseline  | patched   | delta  |
|------------------|----------|----------:|----------:|-------:|
| hashmap_ops      | wasm     | 276.1 ms  | 215.8 ms  | **-21.9%** |
| hashmap_ops      | wasm-gc  | 130.2 ms  |  98.9 ms  | **-24.0%** |
| hashmap_ops      | js       | 162.3 ms  | 134.3 ms  | **-17.3%** |
| hashmap_ops      | native   |  96.1 ms  |  81.1 ms  | **-15.6%** |
| hashmap_string   | wasm     | 171.3 ms  | 149.5 ms  | -12.7% |
| hashmap_string   | wasm-gc  |  70.5 ms  |  65.2 ms  |  -7.5% |
| hashmap_update   | wasm     |  68.7 ms  |  71.8 ms  |  noise |
| hashmap_update   | wasm-gc  |  32.5 ms  |  30.7 ms  |  -5.5% |
| hashset_ops      | wasm     | 279.7 ms  | 222.3 ms  | **-20.5%** |
| hashset_ops      | wasm-gc  | 117.5 ms  |  97.1 ms  | **-17.4%** |
| hashset_ops      | js       | 155.4 ms  | 134.0 ms  | **-13.8%** |
| hashset_ops      | native   |  94.1 ms  |  84.8 ms  |  -9.9% |
| json_parse       | wasm-gc  | 239.7 ms  | 224.9 ms  |  -6.2% |
| json_parse       | native   | 168.2 ms  | 157.9 ms  |  -6.1% |
| json_numbers     | wasm-gc  | 133.9 ms  | 124.8 ms  |  -6.8% |

Workloads (`bench/cmd/*` in <https://github.com/mizchi/pprof-mbt>):
- `hashmap_ops` / `hashset_ops`: 10k distinct Int keys, fill + hit + miss × 50 iter
- `hashmap_string`: 5k distinct String keys, fill + hit + miss × 30 iter — checks the patch is not Int-key-specific
- `hashmap_update`: pre-fill with `capacity = 2 * n`, then 30 rounds of value updates — `grow` is never called after the initial fill, so this is a regression check on the value-update path of `set_with_hash`. The patch is invisible here, confirming the non-rehash path is untouched.
- `json_*`: uses builtin `Map` (linked hash map). 1 grow per object × 1000 objects × 50 iter.

## Test results

`moon test` on all four targets:

| target  | result |
|---------|--------|
| wasm    | 6500 / 6500 pass |
| wasm-gc | 6500 / 6500 pass |
| js      | 6459 / 6459 pass |
| native  | 6411 / 6411 pass |

## Relation to existing work

- #1349 (merged 2024-12): made grow stop re-hashing by passing the stored `hash` through to `set_with_hash`. This PR is the next step on the same path — the inner loop is now lean enough to drop the remaining redundant checks.
- #3164 (merged 2026-01): made the grow check fire only when `set_with_hash` is actually inserting (not when updating). That patch lives in the public path; the new path here lives only in the rehash branch and removes the check entirely.

The three implementations are kept in sync (same Robin-Hood swap, same shape) because they had aligned in #2533.
