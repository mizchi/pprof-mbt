///|
/// Writer / Reader roundtrip via an OS pipe. Producer writes a fixed
/// chunk through BufferedWriter, consumer reads via read_some. Tests
/// the buffered writer flush path + read path + actual native pipe IO.

///|
async fn main {
  let n_chunks = 20000
  let chunk_size = 256
  let iters = 30
  let payload : Bytes = Bytes::make(chunk_size, b'a')
  let mut total : Int = 0
  for _ in 0..<iters {
    @async.with_task_group(root => {
      let (r, w) = @pipe.pipe()
      let writer = @io.BufferedWriter::new(w, size=4096)
      root.spawn_bg(() => {
        for _ in 0..<n_chunks {
          writer.write(payload)
        }
        writer.flush()
        w.close()
      })
      root.spawn_bg(() => {
        let mut local = 0
        while r.read_some(max_len=4096) is Some(b) {
          local = local + b.length()
        }
        total = total + local
        r.close()
      })
    })
  }
  println("bytes=\{total} iters=\{iters}")
}
