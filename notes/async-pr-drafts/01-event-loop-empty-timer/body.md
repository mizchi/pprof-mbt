## Summary

`EventLoop::wait_for_event` runs twice per scheduler tick:

1. `self.timers.iter().head()` — to compute the `poll.wait` timeout
2. `for timer in self.timers` — to fire any expired timer callbacks

When the program does no timer work (IO-only loops, pure compute via `pause`, queue producer/consumer loops, etc.) the `timers : @sorted_set.SortedSet[Timer]` set is empty — but the two iterator-creation and `ms_since_epoch()` calls still fire on every tick. A callgrind profile of a pure-`pause` workload (single coroutine that calls `pause()` 2.5M times) shows the timer scan accounting for **~7% of all instructions executed**.

Short-circuit both spots with `self.timers.is_empty()` so the empty-timer fast path is one branch instead of an `Iter` allocation + `Iter::head()` + `ms_since_epoch()` + loop setup.

## Benchmark

Setup: `/tmp/upstream/async` HEAD `540e94f`, this patch, Linux x86_64, native build, 3-run median wall time.

A new pure-coroutine workload `pause_loop` (single task that calls `@async.pause()` 500000 × 5 iter = 2.5M times, no IO, no timers):

| backend | baseline | patched | delta  |
|---------|---------:|--------:|-------:|
| native  | 637 ms   | 469 ms  | **-26.4%** |

Callgrind top-15 before:

```
14.64%  free
 8.73%  ____moonbit__main async_driver
 7.32%  coroutine::reschedule
 6.75%  _mi_page_malloc_zero
 6.20%  malloc
 4.15%  EventLoop::wait_for_event
 4.01%  SortedSet<Timer>::iter         <-- redundant
 4.01%  coroutine::pause
 3.24%  Deque<Coroutine>::pop_front
 3.10%  SortedSet<Timer>::iter (head)  <-- redundant
 2.68%  EventLoop::poll
 2.53%  Deque<Coroutine>::push_back
 1.90%  epoll_wait
 1.76%  Iter<Timer>::next
 1.55%  moonbitlang_async_get_ms_since_epoch
```

Two `SortedSet<Timer>::iter` entries totaling 7.1% are entirely redundant when the bench never registers a timer. After the patch those (plus the `ms_since_epoch` call inside `wait_for_event`) drop out.

The patch does not affect throughput when the program does register timers; in that case the two `is_empty()` checks are one cheap branch each.

## Test results

```
moonbitlang/async                       87 / 87 pass
moonbitlang/async/aqueue                35 / 35 pass
moonbitlang/async/semaphore              6 /  6 pass
moonbitlang/async/cond_var               5 /  5 pass
moonbitlang/async/internal/coroutine     1 /  1 pass
moonbitlang/async/internal/event_loop    7 /  7 pass
moonbitlang/async/internal/time          3 /  3 pass
moonbitlang/async/timer                  0 /  0 pass
```

Socket / TLS / HTTP / FS / WebSocket / process tests require a network interface that this sandbox can't provide; they're untouched by this patch.

## Background

This came out of a callgrind exercise on a pure-coroutine workload, documented at <https://github.com/mizchi/pprof-mbt> (`bench-async/cmd/pause_loop`). The profile cleanly attributed 7% of total instructions to the timer scan, with no real timers ever registered.
