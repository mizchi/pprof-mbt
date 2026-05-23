///|
/// PlainDateTime::from_string parsing throughput.

///|
fn main {
  // ISO 8601 like; the format moonbitlang/x/time accepts.
  let sample = "2024-05-23T14:37:12.123456789"
  let n = 200000
  let mut ok = 0
  for _ in 0..<n {
    if (try? @time.PlainDateTime::from_string(sample)) is Ok(_) {
      ok = ok + 1
    }
  }
  println("ok=\{ok} iters=\{n}")
}
