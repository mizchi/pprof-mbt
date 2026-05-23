///|
/// json5 parser bench — mirrors core's json_parse bench (1000 element
/// object array) so we can compare core json vs x/json5 on the same
/// payload shape. json5 allows trailing commas, comments, unquoted
/// keys etc.

///|
let parse_bench_item =
  #|{
  #|  id: 123456,
  #|  name: 'item-123456',
  #|  price: 123456.125,
  #|  ratio: 457e-3,
  #|  active: true,
  #|  tags: ['alpha', 'beta', 'gamma',],
  #|  meta: {
  #|    count: 370368,
  #|    note: null,
  #|  },
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
fn main {
  let input = repeat_bench_input(1000)
  let iters = 50
  let mut ok = 0
  for _ in 0..<iters {
    let res = try? @json5.parse(input)
    if res is Ok(_) {
      ok = ok + 1
    }
  }
  println("parsed=\{ok} input_len=\{input.length()}")
}
