///|
/// Update-heavy workload: build a map of N keys once, then repeatedly
/// overwrite every value. `grow` is never called after the initial fill,
/// so the grow-patch shouldn't affect this — it's a regression check on
/// the value-update path of `set_with_hash`.

///|
fn main {
  let n = 10000
  // Pre-fill phase (counted separately so we can ignore it).
  let m : @hashmap.HashMap[Int, Int] = @hashmap.HashMap([], capacity=n * 2)
  for i in 0..<n {
    m[i * 1664525 + 1013904223] = i
  }
  let updates = 30
  let mut sum = 0
  for round in 0..<updates {
    for i in 0..<n {
      let k = i * 1664525 + 1013904223
      m[k] = round + i
    }
    for i in 0..<n {
      match m.get(i * 1664525 + 1013904223) {
        Some(v) => sum = sum + v
        None => ()
      }
    }
  }
  println("sum=\{sum} updates=\{updates}")
}
