///|
/// Pure-scheduler workload: a single task that pauses N times. Stresses
/// the suspend/resume + ready-queue path in the coroutine scheduler with
/// no IO involved.

///|
async fn main {
  let n = 100000
  let iters = 5
  let mut total = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      root.spawn_bg(() => {
        for _ in 0..<n {
          @async.pause()
        }
      })
    })
    total = total + n
  }
  println("pauses=\{total} iters=\{iters}")
}
