## Summary

`encoding::encode(UTF8 | UTF16BE, ...)` walks the source string with `for char in src`. That goes through `String::iter` + `Iter::next<Char>` per code point, plus surrogate-pair decoding inside `Iter::next`. On an ASCII-heavy 64 KiB payload the iterator path is **~30% of self time** (`String::iter` 19.6% + `Iter::next<Char>` 10.2%) — more than the actual UTF-8 emission.

Replace with an explicit UTF-16 code-unit walk that handles surrogate pairs inline:

```moonbit
let len = src.length()
let mut i = 0
while i < len {
  let cu = src.unsafe_get(i).to_int()
  i += 1
  let cp = if cu >= 0xD800 && cu <= 0xDBFF && i < len {
    let cu2 = src.unsafe_get(i).to_int()
    if cu2 >= 0xDC00 && cu2 <= 0xDFFF {
      i += 1
      ((cu - 0xD800) * 0x400) + (cu2 - 0xDC00) + 0x10000
    } else { cu }
  } else { cu }
  write(new_buf, cp.unsafe_to_char())
}
```

ASCII code points (`cu < 0xD800`) skip the surrogate-detection arm entirely, which matches the common case.

## Benchmark

Setup: 4 KiB ASCII chunk × 64 = ~256 KiB payload, UTF-8 encoded 5000× per run. Native release, Linux x86_64, 3-run median wall time.

|                | baseline | patched | delta  |
|----------------|---------:|--------:|-------:|
| encoding_utf8  |  528 ms  | 408 ms  | **-22.7%** |

## Tests

```
moonbitlang/x/encoding    71 / 71 pass
```

Applied to `moonbitlang/x` `main`.

## Background

Same iter-overhead pattern as the moonbitlang/async gzip crc32 patch and the moonbitlang/x base64 patch (both also -20% to -37% from the same root cause). Investigation log + bench at <https://github.com/mizchi/pprof-mbt> (`bench-x/cmd/encoding_utf8/`).
