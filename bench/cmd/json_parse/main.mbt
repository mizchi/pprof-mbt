///|
/// json/parse benchmark for profiling moonbitlang/core's @json.parse.
///
/// The workload mirrors core's own
/// `json/parse_bench_test.mbt::"bench json parse mixed object array n=1000"`:
/// build a 1000-element array of mixed object literals as a single string,
/// then call @json.parse on it. Repeat a few times so the profiler picks
/// up enough samples.

///|
let parse_bench_item =
  #|{
  #|  "id": 123456,
  #|  "name": "item-123456",
  #|  "price": 123456.125,
  #|  "ratio": 457e-3,
  #|  "active": true,
  #|  "tags": ["alpha", "beta", "gamma"],
  #|  "meta": {
  #|    "count": 370368,
  #|    "note": null
  #|  }
  #|}

///|
fn repeat_bench_input(count : Int) -> String {
  let buf = StringBuilder::new(size_hint=count * parse_bench_item.length())
  buf.write_char('[')
  for i in 0..<count {
    if i > 0 {
      buf.write_char(',')
    }
    buf.write_string(parse_bench_item)
  }
  buf.write_char(']')
  buf.to_string()
}

///|
fn try_parse(input : String) -> Bool {
  try {
    let _ = @json.parse(input)
    true
  } catch {
    _ => false
  }
}

///|
fn main {
  let input = repeat_bench_input(1000)
  // Warm up + measured loop. The profile sampler picks up samples from
  // the inner @json.parse calls.
  let iters = 50
  let mut total = 0
  for _ in 0..<iters {
    if try_parse(input) {
      total = total + 1
    }
  }
  println("parsed=\{total} input_len=\{input.length()}")
}
