# [RFC] Deque: round capacity up to power of two so `% cap` can become `& (cap - 1)`

## Context

`@deque.Deque` is implemented as a ring buffer. Every `push_front` / `push_back` / `pop_front` / `pop_back` (and a few less-hot paths like `binary_search`, `as_views`, `extract_if`) computes an array index via `(head + offset) % buf.length()`. On wasm the modulo is a multi-cycle `i32.rem_u`; on a power-of-two buffer it would collapse to a single `i32.and`.

`Deque::realloc` already only ever produces power-of-two sizes (it starts at 8 and doubles). The remaining sources of arbitrary capacity are the user-facing constructor `Deque(arr, capacity?)` and `reserve_capacity` / `shrink_to_fit`, all of which faithfully report `buf.length()` as `Deque::capacity()`.

## Proposal

Round `buf.length()` up to the next power of two everywhere, and switch the hot index calculations to `& (cap - 1)`.

The visible behavior change is that `Deque([], capacity=10).capacity()` would return `16` rather than `10` — the docs would clarify `capacity()` as "the size of the internal buffer, ≥ the requested capacity". This matches the convention used by Rust's `VecDeque` and similar ring buffers.

## Measured benefit

(`bench/cmd/deque_ops`: 50k alternating `push_front` / `push_back` then drain × 80 iter; `--no-profile` wall time, 3-run median, Linux x86_64.)

| backend  | baseline  | patched   | delta   |
|----------|----------:|----------:|--------:|
| wasm     | 246 ms    | 225 ms    |  -8.5%  |
| wasm-gc  | 102 ms    |  81 ms    | -20.6%  |
| js       | 207 ms    | 167 ms    | -19.3%  |
| native   |  58 ms    |  44 ms    | -24.1%  |

The wasm-gc / js / native deltas are much larger than wasm because on wasm the per-op refcount overhead dilutes the algorithmic gain — the saved cycles are still real but show as a smaller fraction.

## Implementation sketch

The diff is around 100 lines across `deque/deque.mbt`:

- Add a small `round_up_capacity` helper.
- `new_deque` / `Deque::Deque` round up; `realloc` already doubles so stays pow-2.
- Switch `tail_index`, `push_back`, `push_front`, `pop_front`, `unsafe_pop_front`, `unsafe_pop_back`, `reserve_capacity`, `shrink_to_fit`, and the few other `% cap` sites to `& mask`.
- `#inline` on `tail_index` (does nothing alone — see #4 below).

The full WIP diff is available at <https://github.com/mizchi/pprof-mbt/blob/main/notes/deque_bitmask.diff>.

## Open questions

1. **Is the `capacity()` contract change acceptable?** The existing tests (24 of them) assume `capacity()` returns the user-supplied value verbatim — e.g. `Deque([], capacity=10).capacity() == 10`. Either (a) accept the change and update those tests, or (b) track a separate `requested_capacity` field that `capacity()` returns while the internal `buf.length()` is rounded up. (a) is cleaner; (b) is API-compatible but adds a field.
2. **`shrink_to_fit` semantics.** Currently shrinks to exactly `len`. To keep the pow-2 invariant it would shrink to `round_up_capacity(len)`, which can be up to 2× larger than `len`. Alternative: leave it shrinking to exactly `len`, and have the hot path detect non-pow-2 capacity and fall back to `%`. The latter defeats the optimization for shrunk deques.
3. **`reserve_capacity`.** Same question — round up or not.

Inline annotations alone don't move the needle here. I tried adding `#inline` to `tail_index` without changing the modulo, and the wall time was unchanged within noise; the win is entirely from `% → &` so the capacity contract has to give one way or the other.

## Why this is an RFC rather than a PR

Because of the contract change in (1) above — I'd rather get a thumbs-up on direction before sending the patch, especially given how mature the deque test suite is. If the answer is "(b) keep `capacity()` returning the user's value", I can prep that flavor of the patch.

Cross-link to the original profile / measurement notes: <https://github.com/mizchi/pprof-mbt/blob/main/notes/patch_experiments.md#実験-2-deque-の-modulo--bitmask>.
