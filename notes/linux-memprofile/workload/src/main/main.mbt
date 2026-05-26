// Minimal allocation-heavy workload to verify moon-pprof
// memprofile-native on Linux. Two named allocation sites so we can
// confirm symbol resolution + stack distinction:
//   - alloc_strings: appends N short strings to a buffer
//   - alloc_arrays:  pushes N small arrays to an outer array
// Without symbol resolution the resulting pprof would have everything
// folded into one anonymous frame.

fn alloc_strings(n : Int) -> Int {
  let buf = StringBuilder::new()
  for i = 0; i < n; i = i + 1 {
    buf.write_string("hello-")
    buf.write_string(i.to_string())
    buf.write_string("\n")
  }
  buf.to_string().length()
}

fn alloc_arrays(n : Int) -> Int {
  let outer : Array[Array[Int]] = []
  for i = 0; i < n; i = i + 1 {
    let inner : Array[Int] = []
    for j = 0; j < 4; j = j + 1 {
      inner.push(i + j)
    }
    outer.push(inner)
  }
  outer.length()
}

fn main {
  let s = alloc_strings(2000)
  let a = alloc_arrays(2000)
  println("strings_len=\{s} arrays_len=\{a}")
}
