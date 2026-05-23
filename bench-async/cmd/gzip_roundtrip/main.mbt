///|
/// gzip encode -> pipe -> gzip decode. Both ends run on coroutines.
/// Tests the gzip Encoder/Decoder hot loops plus the Reader/Writer
/// trait dispatch around them.

///|
async fn main {
  let chunks = 1000
  let chunk_size = 1024
  let iters = 3
  // Roughly compressible payload (alternating runs).
  let payload : Bytes = {
    let buf = FixedArray::make(chunk_size, b'\x00')
    for i in 0..<chunk_size {
      buf[i] = if (i / 32) % 2 == 0 { b'a' } else { b'b' }
    }
    buf.unsafe_reinterpret_as_bytes()
  }
  let mut total : Int = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      let (r, w) = @pipe.pipe()
      let enc = @gzip.Encoder::Encoder(w)
      let dec = @gzip.Decoder::Decoder(r)
      root.spawn_bg(() => {
        for _ in 0..<chunks {
          enc.write(payload)
        }
        enc.end()
        w.close()
      })
      root.spawn_bg(() => {
        let mut local = 0
        while dec.read_some(max_len=8192) is Some(b) {
          local = local + b.length()
        }
        total = total + local
        r.close()
      })
    })
  }
  println("bytes=\{total} iters=\{iters}")
}
