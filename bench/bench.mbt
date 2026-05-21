///|
/// CPU-bound workloads used to exercise the profiler.
/// Keep these branch-rich and side-effect-free so each backend's
/// sampling profile lines up with the source.

///|
pub fn fib(n : Int) -> Int {
  if n < 2 {
    n
  } else {
    fib(n - 1) + fib(n - 2)
  }
}

///|
/// Iterative ackermann-ish workload — heavy on the call stack so the
/// sampler has plenty of frames to attribute time to.
pub fn ackermann(m : Int, n : Int) -> Int {
  if m == 0 {
    n + 1
  } else if n == 0 {
    ackermann(m - 1, 1)
  } else {
    ackermann(m - 1, ackermann(m, n - 1))
  }
}

///|
/// Mandelbrot iteration count for a single point.
pub fn mandel_point(cx : Double, cy : Double, max_iter : Int) -> Int {
  let mut x = 0.0
  let mut y = 0.0
  let mut i = 0
  while i < max_iter && x * x + y * y <= 4.0 {
    let nx = x * x - y * y + cx
    let ny = 2.0 * x * y + cy
    x = nx
    y = ny
    i = i + 1
  }
  i
}

///|
/// Sum mandelbrot iterations over a grid — single hot loop we can profile.
pub fn mandel_sum(size : Int, max_iter : Int) -> Int {
  let mut total = 0
  let mut iy = 0
  while iy < size {
    let mut ix = 0
    while ix < size {
      let cx = ix.to_double() / size.to_double() * 3.5 - 2.5
      let cy = iy.to_double() / size.to_double() * 2.0 - 1.0
      total = total + mandel_point(cx, cy, max_iter)
      ix = ix + 1
    }
    iy = iy + 1
  }
  total
}
