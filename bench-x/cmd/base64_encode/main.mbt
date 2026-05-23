///|
/// base64 encode throughput: 64 KiB payload encoded N times.

///|
fn main {
  let n = 500
  let chunk_size = 65536
  let buf = FixedArray::make(chunk_size, b'\x00')
  for i in 0..<chunk_size {
    buf[i] = (i & 0xff).to_byte()
  }
  let bytes : Bytes = buf.unsafe_reinterpret_as_bytes()
  let view = bytes[:]
  let mut acc = 0
  for _ in 0..<n {
    let s = @base64.encode(view)
    acc = acc + s.length()
  }
  println("acc=\{acc} iters=\{n} bytes=\{chunk_size}")
}
