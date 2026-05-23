///|
/// HashSet workload: add N keys then probe with hits + misses.

///|
fn main {
  let n = 10000
  let iters = 50
  let mut hits = 0
  for _ in 0..<iters {
    let s : @hashset.HashSet[Int] = @hashset.HashSet([])
    for i in 0..<n {
      s.add(i * 1664525 + 1013904223)
    }
    for i in 0..<n {
      if s.contains(i * 1664525 + 1013904223) {
        hits = hits + 1
      }
    }
    for i in 0..<n {
      if s.contains(i * 7 + 1) {
        hits = hits + 1
      }
    }
  }
  println("hits=\{hits} iters=\{iters}")
}
