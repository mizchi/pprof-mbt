///|
/// base64 decode throughput.

///|
fn main {
  // Prepare a base64 string of ~87 KiB (encoded from a 64 KiB binary).
  let chunk_size = 65536
  let raw = FixedArray::make(chunk_size, b'\x00')
  for i in 0..<chunk_size {
    raw[i] = (i & 0xff).to_byte()
  }
  let encoded : String = @base64.encode(raw.unsafe_reinterpret_as_bytes()[:])
  let view = encoded[:]
  let n = 500
  let mut acc = 0
  for _ in 0..<n {
    let r = try? @base64.decode(view)
    if r is Ok(b) {
      acc = acc + b.length()
    }
  }
  println("acc=\{acc} iters=\{n} encoded_len=\{encoded.length()}")
}
