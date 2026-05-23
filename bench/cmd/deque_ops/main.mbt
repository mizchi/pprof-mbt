///|
/// Deque workload: alternating push_front / push_back, then drain from
/// both ends. Stresses the ring buffer + grow path.

///|
fn main {
  let n = 50000
  let iters = 80
  let mut total = 0
  for _ in 0..<iters {
    let d : @deque.Deque[Int] = @deque.new()
    for i in 0..<n {
      if (i & 1) == 0 {
        d.push_back(i)
      } else {
        d.push_front(i)
      }
    }
    for i in 0..<n {
      let v = if (i & 1) == 0 { d.pop_back() } else { d.pop_front() }
      match v {
        Some(x) => total = total + (x & 1)
        None => break
      }
    }
  }
  println("total=\{total} iters=\{iters}")
}
