# `moonbitlang/async` investigation

Same workflow as the `moonbitlang/core` work in `notes/patch_experiments.md`,
applied to the async runtime. async only supports profile on **native**
in this sandbox (samply / perf are unavailable for samp; valgrind
callgrind is what we got).

## Bench scaffolding

A separate workspace under `bench-async/` depends on
`moonbitlang/async` 0.19.1 via the registry. Three workloads, all
pure-coroutine (no socket / fs / process):

| workload | what it does | iter × n |
|---|---|---|
| `pause_loop` | one task calls `@async.pause()` in a loop | 5 × 500k = 2.5M pauses |
| `spawn_wait` | spawn N background tasks each doing 1 pause | 30 × 20k = 600k tasks |
| `aqueue_throughput` | SPSC `@aqueue.Queue::put/get` | 30 × 200k = 6M items |

native binaries built via `moon build --release --target=native`.

## Profile results (callgrind, instruction counts)

### pause_loop (3.55 B Ir)

Top:

| % | symbol |
|---|---|
| 14.6% | `free` |
|  8.7% | `____moonbit__main` async_driver |
|  7.3% | `coroutine::reschedule` |
|  6.8% | `_mi_page_malloc_zero` |
|  6.2% | `malloc` |
|  4.2% | `EventLoop::wait_for_event` |
|  4.0% | `SortedSet<Timer>::iter` (timer scan) |
|  4.0% | `coroutine::pause` |
|  3.2% | `Deque<Coroutine>::pop_front` |
|  3.1% | `SortedSet<Timer>::iter`#2 (timeout calc) |
|  2.7% | `EventLoop::poll` |
|  2.5% | `Deque<Coroutine>::push_back` |
|  1.9% | `epoll_wait` |
|  1.8% | `Iter<Timer>::next` |
|  1.5% | `moonbitlang_async_get_ms_since_epoch` |

**Headline**: `SortedSet<Timer>::iter` x 2 = 7.1% — wasted because no
timer ever registered. Plus `Iter<Timer>::next` 1.8% and
`get_ms_since_epoch` 1.5% are downstream of the timer scan.

### spawn_wait (214 M Ir)

Top:

| % | symbol |
|---|---|
| 18.9% | `moonbit_drop_object` |
| 14.2% | `free` |
|  6.5% | `_mi_page_malloc_zero` |
|  6.0% | `malloc` |
|  3.9% | `coroutine::reschedule` |
|  3.7% | `Set<Coroutine>::add_entry_to_tail` |
|  3.1% | `Set<Coroutine>::set_entry` |
|  3.1% | `Set<Coroutine>::clear` |
|  2.8% | `TaskGroup::spawn_coroutine[worker]` |
|  2.2% | `Deque<Coroutine>::pop_front` |
|  2.2% | `coroutine::spawn` |
|  2.1% | `Hash<Coroutine>::hash` |
|  1.9% | `Set<Coroutine>::add_with_hash` |
|  1.8% | `Set<Coroutine>::shift_back` |

**Headline**: `Set<Coroutine>` ops sum to ~21%. The `downstream:
Set[Coroutine]` field on `TaskGroup` does linked-hash-set insertion +
removal per spawn / finish. `moonbit_drop_object` 18.9% reflects the
refcount churn on the `Coroutine` struct + its `Set::Entry` wrappers.

### aqueue_throughput (505 M Ir)

Top:

| % | symbol |
|---|---|
| 15.5% | `free` |
| 12.0% | `____moonbit__main` driver |
| 11.9% | `____moonbit__main` other driver |
|  7.1% | `_mi_page_malloc_zero` |
|  6.5% | `malloc` |
|  5.8% | `Queue::put` |
|  5.4% | `Queue::try_put` |
|  5.2% | `Queue::get` |
|  4.2% | `Queue::try_get` |
|  3.2% | `Deque<Int>::push_back` |
|  2.9% | `Deque<Int>::pop_front` |
|  1.5% | `Deque<Reader<Int>>::pop_front` |
|  1.5% | `Deque<Writer<Int>>::pop_front` |

**Headline**: distributed across `Queue::put/get/try_*` (20%+). The
hot allocations are `Reader::{value: None, coro: Some(coro)}` per
blocked `get` and `Writer::{value: data, coro: Some(coro)}` per blocked
`put`. Hard to reduce without API redesign.

## Patches landed in this investigation

### Patch A: `moonbitlang/core/set` grow specialized rehash

Mirror of the `core/hashmap` and `core/builtin/linked_hash_map` patches
from the earlier `pr-drafts/02-hashmap-grow-rehash` PR. `Set::grow`
calls `add_with_hash` on every existing entry, which has redundant Eq
+ load-factor checks. Replace with a dedicated Robin Hood swap path.

Diff at `notes/core_set_grow_specialized.diff` (59 lines).

This is **a follow-up to the existing core PR-02**: the patch family
should grow to four targets (`hashmap`, `hashset`, `builtin/Map`,
`set`) at once. I'll fold it into PR-02 before submitting.

Effect on async benches (the `Set[Coroutine]` user):

| workload | baseline | patched core | delta |
|---|--:|--:|--:|
| pause_loop | 637 ms | 627 ms | -1.6% (noise) |
| spawn_wait | 280 ms | 270 ms | **-3.6%** |
| aqueue_throughput | 486 ms | 478 ms | noise |

Modest because grow only fires log₂(N) times per group, while
`add_with_hash` (which the patch doesn't touch) runs every spawn.

### Patch B: `moonbitlang/async` event_loop empty-timer short-circuit

`EventLoop::wait_for_event` always scans `self.timers` twice per
scheduler tick — once for the timeout calc and once to fire expired
timers. When `self.timers` is empty (the common case for IO-only or
compute-only tasks) both scans are pure overhead. Add an `is_empty()`
short-circuit.

Diff at `notes/async_event_loop_empty_timer.diff` (39 lines).

Effect (cumulative on top of Patch A):

| workload | baseline | patched async | total delta |
|---|--:|--:|--:|
| `pause_loop` | 637 ms | **469 ms** | **-26.4%** |
| `spawn_wait` | 280 ms | 280 ms | noise (already not bottlenecked here) |
| `aqueue_throughput` | 486 ms | 489 ms | noise |

Tests: full `moon test --target native` passes for the 8 non-network
packages (144 tests). Network tests can't run in this sandbox.

## PR artifacts

`notes/async-pr-drafts/01-event-loop-empty-timer/` is shaped exactly
like `notes/pr-drafts/`: `title.txt`, `body.md`, raw `patch.diff`, and
`0001-*.patch` for `git am`.

The `core/set` grow patch (Patch A) folds into the existing
`pr-drafts/02-hashmap-grow-rehash` rather than getting its own PR — the
four-target version is the right shape to submit.

## Future directions

Things I noticed but didn't implement:

1. **`TaskGroup.downstream: Set[Coroutine]`** — used only to track
   pending children for `with_task_group` to wait on. A counter or a
   linked-list would be cheaper. ~20% of spawn_wait. Requires an
   async-side redesign with API care.
2. **`Reader`/`Writer` allocation per blocked aqueue op**. A pooled
   freelist would save 1 alloc per op. ~5% of aqueue_throughput.
3. **`moonbit_drop_object` 19% on spawn_wait**. Same refcount story as
   core json_parse — the compiler-side fix is the lever.
