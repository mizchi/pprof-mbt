///|
/// Spawn N tasks that each `sleep(0)`. Exercises the timer SortedSet
/// add/remove path — counter to `pause_loop` which has zero timers.
/// Validates that the empty-timer fast path doesn't penalise the
/// timer-present case.

///|
async fn main {
  let n = 5000
  let iters = 20
  let mut total = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      for _ in 0..<n {
        root.spawn_bg(() => {
          @async.sleep(0)
        })
      }
    })
    total = total + n
  }
  println("timers=\{total} iters=\{iters}")
}
