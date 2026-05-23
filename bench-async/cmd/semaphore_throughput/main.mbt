///|
/// Semaphore with capacity 1 used as a mutex: one task acquires/releases
/// in a tight loop while another does the same. Stresses the
/// acquire/release path + the wakeup mechanism.

///|
async fn main {
  let n = 50000
  let iters = 20
  let mut total = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      let sem = @semaphore.Semaphore::Semaphore(1)
      root.spawn_bg(() => {
        for _ in 0..<n {
          sem.acquire()
          sem.release()
        }
      })
      root.spawn_bg(() => {
        for _ in 0..<n {
          sem.acquire()
          sem.release()
        }
      })
    })
    total = total + 2 * n
  }
  println("ops=\{total} iters=\{iters}")
}
