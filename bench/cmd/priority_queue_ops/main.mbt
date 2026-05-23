///|
/// PriorityQueue workload: push N elements in adversarial order, then pop
/// them all. Heap sift-up / sift-down dominates.

///|
fn main {
  let n = 20000
  let iters = 50
  let mut total = 0
  for _ in 0..<iters {
    let q : @pq.PriorityQueue[Int] = @pq.PriorityQueue([])
    for i in 0..<n {
      q.push((i * 1664525 + 1013904223) & 0x7fffffff)
    }
    while !q.is_empty() {
      match q.pop() {
        Some(v) => total = total + (v & 1)
        None => break
      }
    }
  }
  println("total=\{total} iters=\{iters}")
}
