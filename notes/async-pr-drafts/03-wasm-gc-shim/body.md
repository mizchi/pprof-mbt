## Summary

`integration.run_async_main` is defined only for `target="native"` and `target="js"`. Attempting to build any `async fn main { ... }` for wasm or wasm-gc fails with `Value run_async_main not found in package moonbitlang/async`.

This matters even though wasm/wasm-gc don't get socket / fs / process / timer / signal (they all `abort` via `unimplemented.mbt`). Pure-coroutine code — `pause`, `with_task_group`, `aqueue`, `semaphore`, `cond_var` — does not depend on the event loop and works fine on wasm/wasm-gc.

Add the missing scheduler-only entry point. Mirrors the existing JS shim:

```moonbit
#cfg(target="wasm-gc")
#doc(hidden)
pub fn run_async_main(main : async () -> Unit) -> Unit {
  let _ = @coroutine.spawn(main)
  @event_loop.reschedule()
}
```

Plus a matching `reschedule()` in `internal/event_loop/unimplemented.mbt` that just drains `@coroutine.reschedule()` until no ready work remains.

## Why bother

Two reasons it's useful even without IO:

1. **Embedding**. The `wasm-gc` target is the natural deployment for moonbit code running inside a host (browser, V8, runtime with custom IO). Letting that host call `async fn main` with the standard scheduler — and arrange IO independently — closes the gap.
2. **Cross-backend benchmarks of the scheduler / aqueue / cond_var / semaphore**. With this shim in place, the same coroutine workload can be measured on all four backends and the refcount + event-loop overheads become directly visible.

For (2) I ran 5 pure-coroutine workloads across native / wasm / wasm-gc / js (results table at <https://github.com/mizchi/pprof-mbt/blob/main/notes/async_backend_comparison.md>). Headline: **wasm-gc beats native by 3× on `pause_loop` and 2× on `cond_var_signal`** because the event-loop overhead (`epoll_wait` + `ms_since_epoch` + timer-set scan) drops out. wasm is 3-4× slower than wasm-gc on all five — same refcount story as moonbitlang/core.

## Test results

`moon test --target wasm-gc` runs only the tests that already have `unimplemented`-friendly fallbacks. The patch doesn't touch any existing code path so it can't regress native or js tests.

## Caveats

- This shim has no real event loop. `sleep`, `Timer::*`, sockets, fs, etc. still `abort` via the existing `unimplemented.mbt` stubs.
- A user calling `await async_io_thing()` on wasm-gc gets an abort at runtime, not a build-time error. That's consistent with how the current wasm stubs behave.

The two-line shim itself is just delegating to existing primitives, so the surface for accidental breakage is small.
