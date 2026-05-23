///|
/// SHA-256 hashing throughput: hash a 64 KiB payload N times.

///|
fn main {
  let n = 500
  let chunk_size = 65536
  // Roughly random-looking payload
  let payload = FixedArray::make(chunk_size, b'\x00')
  for i in 0..<chunk_size {
    payload[i] = (i & 0xff).to_byte()
  }
  let bytes : Bytes = payload.unsafe_reinterpret_as_bytes()
  let mut acc = 0
  for _ in 0..<n {
    let digest = @crypto.sha256(bytes)
    acc = acc + digest[0].to_int()
  }
  println("acc=\{acc} iters=\{n} bytes=\{chunk_size}")
}
