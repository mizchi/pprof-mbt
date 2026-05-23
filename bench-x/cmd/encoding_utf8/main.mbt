///|
/// UTF-8 encode an ASCII-heavy string. moonbit String is UTF-16
/// internally, so this exercises the surrogate-handling + 1..4 byte
/// emission per char.

///|
fn main {
  // ~4 KiB ASCII payload (Lorem-ish)
  let chunk = "The quick brown fox jumps over the lazy dog. " +
    "0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz.\n"
  let builder = StringBuilder::new(size_hint=chunk.length() * 64)
  for _ in 0..<64 {
    builder.write_string(chunk)
  }
  let payload = builder.to_string()
  let n = 5000
  let mut total = 0
  for _ in 0..<n {
    let b = @encoding.encode(@encoding.UTF8, payload)
    total = total + b.length()
  }
  println("total=\{total} iters=\{n} len=\{payload.length()}")
}
