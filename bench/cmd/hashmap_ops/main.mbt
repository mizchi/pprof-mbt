///|
/// HashMap workload: fill, hit-lookup, miss-lookup. Stresses Hash + Eq
/// and the open-addressing probe path.

///|
fn main {
  let n = 10000
  let iters = 50
  let mut sum = 0
  for _ in 0..<iters {
    let m : @hashmap.HashMap[Int, Int] = @hashmap.new()
    for i in 0..<n {
      m[i * 1664525 + 1013904223] = i
    }
    // hit
    for i in 0..<n {
      match m.get(i * 1664525 + 1013904223) {
        Some(v) => sum = sum + v
        None => ()
      }
    }
    // miss
    for i in 0..<n {
      match m.get(i * 7 + 1) {
        Some(_) => sum = sum + 1
        None => ()
      }
    }
  }
  println("sum=\{sum} iters=\{iters}")
}
