///|
/// Cond::signal ping-pong: producer signals N times, consumer waits N
/// times. Stresses the Waiter struct alloc + wake path.

///|
async fn main {
  let n = 20000
  let iters = 20
  let mut total = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      let cv = @cond_var.Cond::Cond()
      let done = Ref::new(0)
      root.spawn_bg(() => {
        for _ in 0..<n {
          cv.wait()
          done.val = done.val + 1
        }
      })
      root.spawn_bg(() => {
        for _ in 0..<n {
          @async.pause()
          cv.signal()
        }
      })
    })
    total = total + n
  }
  println("ops=\{total} iters=\{iters}")
}
