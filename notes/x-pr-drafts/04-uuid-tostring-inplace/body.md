## Summary

`UUID::to_string` builds the UUID hex representation into a `FixedArray[Byte]` of size 72 (= 36 UTF-16 code units × 2 bytes), then emits the result via:

```moonbit
Bytes::from_array(Array::from_fixed_array(rv)).to_unchecked_string()
```

That allocates **twice** for no reason — first to copy the `FixedArray` into a heap `Array`, then again to copy that `Array` into `Bytes` (`Bytes::makei` walks the array element-by-element). The `to_unchecked_string` step at the end correctly reinterprets the byte buffer as a UTF-16 string.

On a `to_string` round-trip benchmark the three downstream symbols add up to **~58% of self time**:

| symbol             | self  |
|--------------------|------:|
| `ArrayView::at<Byte>` | 27.0% |
| `Bytes::from_array`   | 16.9% |
| `Bytes::makei`        | 14.6% |

Skipping both copies — `rv` already *is* the UTF-16 byte buffer:

```moonbit
break rv.unsafe_reinterpret_as_bytes().to_unchecked_string()
```

## Benchmark

Setup: 4 sample UUIDs, parse + to_string × 1 000 000 iters. Native release, Linux x86_64, 3-run median wall time.

|             | baseline | patched | delta  |
|-------------|---------:|--------:|-------:|
| uuid_parse  |  549 ms  | 197 ms  | **-64.1%** |

## Tests

```
moonbitlang/x/uuid    9 / 9 pass
```

Applied to `moonbitlang/x` `main`.

## Background

Investigation log + bench at <https://github.com/mizchi/pprof-mbt> (`bench-x/cmd/uuid_parse/`). The single largest perf win landed in this entire investigation across `moonbitlang/core` + `async` + `x`.
