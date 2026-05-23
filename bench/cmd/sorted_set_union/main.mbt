///|
/// SortedSet union benchmark: two interleaved n-element sets unioned.

///|
fn make_set(start : Int, end_ : Int, step : Int) -> @sset.SortedSet[Int] {
  let mut s : @sset.SortedSet[Int] = @sset.new()
  let mut i = start
  while i < end_ {
    s = s.add(i)
    i = i + step
  }
  s
}

///|
fn main {
  let s1 = make_set(0, 10000, 2)
  let s2 = make_set(1, 10000, 2)
  let iters = 30
  let mut total = 0
  for _ in 0..<iters {
    let u = s1.union(s2)
    total = total + u.to_array().length()
  }
  println("union_total=\{total} iters=\{iters}")
}
