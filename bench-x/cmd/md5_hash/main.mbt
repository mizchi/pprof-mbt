///|
/// MD5 hashing throughput.

///|
fn main {
  let n = 1000
  let chunk_size = 65536
  let payload = FixedArray::make(chunk_size, b'\x00')
  for i in 0..<chunk_size {
    payload[i] = (i & 0xff).to_byte()
  }
  let bytes : Bytes = payload.unsafe_reinterpret_as_bytes()
  let mut acc = 0
  for _ in 0..<n {
    let digest = @crypto.md5(bytes)
    acc = acc + digest[0].to_int()
  }
  println("acc=\{acc} iters=\{n} bytes=\{chunk_size}")
}
