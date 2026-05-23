///|
/// Decimal add / mul / round chain — factorial-style.

///|
fn main {
  let n = 1500
  let iters = 100
  let mut total = 0
  for _ in 0..<iters {
    let mut acc = @decimal.one()
    for i in 2..<n {
      acc = acc * @decimal.Decimal::from_int(i)
    }
    total = total + acc.scale()
  }
  println("scale_sum=\{total} iters=\{iters} n=\{n}")
}
