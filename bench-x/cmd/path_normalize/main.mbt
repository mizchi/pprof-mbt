///|
/// Path::normalize + dirname + basename + extname throughput on a
/// realistic-looking path with `..` / `.` segments.

///|
fn main {
  let sample = @path.Path("/home/user/projects/../bench-x/./_build/native/release/build/cmd/path_normalize/main.exe")
  let n = 200000
  let mut acc = 0
  for _ in 0..<n {
    let normalized = sample.normalize()
    let dn = normalized.dirname()
    let bn = normalized.basename()
    let ext = normalized.extname()
    acc = acc + bn.length() + ext.length() + dn.0.length()
  }
  println("acc=\{acc} iters=\{n}")
}
