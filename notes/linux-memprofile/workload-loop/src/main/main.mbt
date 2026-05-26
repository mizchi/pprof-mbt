// Long-running workload to verify `moon-pprof memprofile-native
// --duration N`. The loop allocates on every iteration and never
// returns, so without --duration the profile would never be written.
//
// `hot_alloc` is the named site we want to see in the resulting
// pprof. `outer` keeps a small rolling reference so the optimiser
// can't dead-code-eliminate the allocations.

fn hot_alloc(seed : Int) -> Array[Int] {
  let xs : Array[Int] = []
  for i = 0; i < 16; i = i + 1 {
    xs.push(seed + i)
  }
  xs
}

fn main {
  let mut total = 0
  let mut tick : Int = 0
  for ;; {
    let xs = hot_alloc(tick)
    total = total + xs[0]
    tick = tick + 1
  }
  // Unreachable; here to convince the type checker `total` is used.
  println("total=\{total}")
}
