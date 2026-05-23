///|
/// ChaCha20 encrypt a 64 KiB payload N times. The library returns a
/// fresh FixedArray[Byte] per call, so allocation pressure is on top
/// of the cipher core.

///|
fn main {
  // 8-word key, 1-word nonce
  let key : FixedArray[UInt] = [
    0x00010203U, 0x04050607U, 0x08090a0bU, 0x0c0d0e0fU,
    0x10111213U, 0x14151617U, 0x18191a1bU, 0x1c1d1e1fU,
  ]
  let nonce : UInt = 0xfeedface
  let n = 200
  let chunk_size = 65536
  let payload = FixedArray::make(chunk_size, b'\x00')
  for i in 0..<chunk_size {
    payload[i] = (i & 0xff).to_byte()
  }
  let bytes : Bytes = payload.unsafe_reinterpret_as_bytes()
  let mut acc = 0
  for _ in 0..<n {
    let out = (try? @crypto.chacha20(key, 0, bytes, nonce~)).unwrap()
    acc = acc + out[0].to_int()
  }
  println("acc=\{acc} iters=\{n} bytes=\{chunk_size}")
}
