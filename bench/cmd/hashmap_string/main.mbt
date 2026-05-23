///|
/// HashMap[String, Int] workload: build N distinct string keys, fill the
/// map, then hit-lookup all and miss-lookup all.

///|
fn main {
  let n = 5000
  let iters = 30
  // Pre-build string keys so the bench doesn't get dominated by
  // string allocation inside the timing loop.
  let keys : Array[String] = Array::new(capacity=n)
  for i in 0..<n {
    keys.push("key-\{i}-\{i * 1664525 + 1013904223}")
  }
  let miss_keys : Array[String] = Array::new(capacity=n)
  for i in 0..<n {
    miss_keys.push("missing-\{i}")
  }
  let mut hits = 0
  for _ in 0..<iters {
    let m : @hashmap.HashMap[String, Int] = @hashmap.HashMap([])
    for i in 0..<n {
      m[keys[i]] = i
    }
    for i in 0..<n {
      match m.get(keys[i]) {
        Some(v) => hits = hits + v
        None => ()
      }
    }
    for i in 0..<n {
      if m.get(miss_keys[i]) is Some(_) {
        hits = hits + 1
      }
    }
  }
  println("hits=\{hits} iters=\{iters}")
}
