///|
/// SortedMap.merge benchmark for profiling moonbitlang/core.
///
/// Mirrors the "bench merge interleaved n=5000" case from core's own
/// immut/sorted_map/merge_bench_test.mbt — two interleaved 5k-key maps
/// merged together, repeated enough times for the profile to be
/// meaningful. Interleaved keys hit the rebuild path harder than
/// disjoint maps, which is where the costs live.

///|
fn make_map(
  start : Int,
  end_ : Int,
  step : Int,
) -> @sorted_map.SortedMap[Int, Int] {
  let mut m : @sorted_map.SortedMap[Int, Int] = @sorted_map.new()
  let mut i = start
  while i < end_ {
    m = m.add(i, i)
    i = i + step
  }
  m
}

///|
fn main {
  let m1 = make_map(0, 10000, 2)
  let m2 = make_map(1, 10000, 2)
  let iters = 30
  let mut total = 0
  for _ in 0..<iters {
    let merged = m1.merge(m2)
    total = total + merged.length()
  }
  println("merged_size_sum=\{total} iters=\{iters}")
}
