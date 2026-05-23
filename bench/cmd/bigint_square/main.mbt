///|
/// Repeated squaring: x = x * x for K iterations. Each step doubles the
/// limb count, so both operands always have equal size — this is the
/// shape Karatsuba is designed for, and it exercises the n*n grade-school
/// path (and Karatsuba above the threshold).

///|
fn main {
  // Start with a 64-bit-ish seed; after 12 squarings we're at ~256k bits
  // (8k limbs), well above the karatsuba_threshold = 50.
  let seed = @bigint.BigInt::from_string("123456789012345678901234567890")
  let k = 11
  let iters = 5
  let mut total = 0
  for _ in 0..<iters {
    let mut x = seed
    for _ in 0..<k {
      x = x * x
    }
    total = total + x.bit_length()
  }
  println("bitlen_sum=\{total} iters=\{iters} k=\{k}")
}
