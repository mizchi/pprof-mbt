///|
/// Number-heavy JSON: a flat array of 10k safe integers. Exercises the
/// `scan_json_number` + safe-int fast path; the dominant ASCII string
/// hot spots of the mixed-object bench should drop out.

///|
fn build_input(count : Int) -> String {
  let buf = StringBuilder::new(size_hint=count * 8)
  buf.write_char('[')
  for i in 0..<count {
    if i > 0 {
      buf.write_char(',')
    }
    let v = i * 1664525 + 1013904223
    buf.write_string(v.to_string())
  }
  buf.write_char(']')
  buf.to_string()
}

///|
fn main {
  let input = build_input(10000)
  let iters = 30
  let mut total = 0
  for _ in 0..<iters {
    let parsed = try? @json.parse(input)
    match parsed {
      Ok(_) => total = total + 1
      Err(_) => ()
    }
  }
  println("ok=\{total} input_len=\{input.length()}")
}
