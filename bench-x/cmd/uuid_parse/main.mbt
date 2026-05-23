///|
/// UUID::from_hex parse + UUID::to_string round-trip.

///|
fn main {
  let samples = [
    "550e8400-e29b-41d4-a716-446655440000",
    "00112233-4455-6677-8899-aabbccddeeff",
    "f47ac10b-58cc-4372-a567-0e02b2c3d479",
    "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  ]
  let n = 1000000
  let mut acc = 0
  for i in 0..<n {
    let s = samples[i % samples.length()]
    let r = try? @uuid.from_hex(s)
    match r {
      Ok(u) => {
        let back = u.to_string()
        acc = acc + back.length()
      }
      Err(_) => ()
    }
  }
  println("acc=\{acc} iters=\{n}")
}
