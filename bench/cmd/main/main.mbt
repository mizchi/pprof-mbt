///|
/// Entry point. The CLI runs each workload a fixed number of times so the
/// profile has enough samples to be meaningful. Tune via env if needed.
fn main {
  let mandel_total = @bench.mandel_sum(160, 500)
  let fib_total = @bench.fib(32)
  let ack_total = @bench.ackermann(3, 10)
  println("mandel_sum=\{mandel_total} fib=\{fib_total} ack=\{ack_total}")
}
