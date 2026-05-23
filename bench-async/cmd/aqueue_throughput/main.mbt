///|
/// Single-producer / single-consumer aqueue throughput. Hits the
/// async wait + wake path on every get when the queue is empty, and
/// the cond_var-style notify path on put.

///|
async fn main {
  let n = 200000
  let iters = 30
  let mut total = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      let q : @aqueue.Queue[Int] = @aqueue.Queue::Queue(kind=@aqueue.Unbounded)
      root.spawn_bg(() => {
        for i in 0..<n {
          q.put(i)
        }
      })
      root.spawn_bg(() => {
        for _ in 0..<n {
          let _ = q.get()

        }
      })
    })
    total = total + n
  }
  println("items=\{total} iters=\{iters}")
}
