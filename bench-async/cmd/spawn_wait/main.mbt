///|
/// Spawn N background tasks, each does one pause, then the group waits
/// for all to finish. Exercises task spawn / Task struct allocation /
/// downstream-set bookkeeping in addition to the scheduler.

///|
async fn main {
  let n = 20000
  let iters = 30
  let mut total = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      for _ in 0..<n {
        root.spawn_bg(() => {
          @async.pause()
        })
      }
    })
    total = total + n
  }
  println("tasks=\{total} iters=\{iters}")
}
