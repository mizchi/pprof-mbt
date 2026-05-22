///|
/// BigInt workload: factorial chain. Each multiplication grows the
/// limb count, so this stresses big-num multiplication + allocation.

///|
fn factorial(n : Int) -> @bigint.BigInt {
  let mut acc = @bigint.BigInt::from_int(1)
  for i in 2..=n {
    acc = acc * @bigint.BigInt::from_int(i)
  }
  acc
}

///|
fn main {
  let n = 800
  let iters = 100
  let mut total = 0
  for _ in 0..<iters {
    let f = factorial(n)
    total = total + f.bit_length()
  }
  println("bitlen_sum=\{total} iters=\{iters}")
}
