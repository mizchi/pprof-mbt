## Summary

`time/util.mbt`'s `split(s, delimiter)` was implemented as char-by-char `StringBuilder::write_char` + `to_string` + `reset` per segment. The file already had a `FIXME: use split method of String` comment acknowledging this.

```moonbit
// before — per-char work, per-segment new String + grow_if_necessary
let buf = StringBuilder::new(size_hint=0)
for i = 0; i < s.length(); i = i + 1 {
  let code_unit = s.code_unit_at(i)
  if code_unit == delimiter_code {
    spl.push(buf.to_string())
    buf.reset()
  } else {
    buf.write_char(UInt16::unsafe_to_char(code_unit))
  }
}
spl.push(buf.to_string())
```

Replaced with index-tracking + StringView slicing:

```moonbit
let mut start = 0
for i = 0; i < s.length(); i = i + 1 {
  if s.code_unit_at(i) == delimiter_code {
    spl.push(s[start:i].to_owned())
    start = i + 1
  }
}
spl.push(s[start:].to_owned())
```

One `.to_owned()` per segment — no per-character append, no StringBuilder, no per-segment `grow_if_necessary`.

## Why this is hot

`PlainDateTime::from_string` calls `split(str, 'T')` for every parse — both segments then go to `PlainDate::from_string` / `PlainTime::from_string`. `Duration::from_string` also calls `split(s, '.')`. So every datetime / duration parse pays this cost.

## Benchmark

`bench-x/cmd/plain_datetime_parse/` — `PlainDateTime::from_string("2024-05-23T14:37:12.123456789")` × 200 000 iters. Native release, Linux x86_64, 3-run median wall time.

|                      | baseline | patched | delta  |
|----------------------|---------:|--------:|-------:|
| plain_datetime_parse |  179 ms  | 132 ms  | **-26.3%** |

Callgrind total instructions: 2.46 G → 1.93 G (**-21.5%**). `StringBuilder::write_char` (8.42%), `grow_if_necessary` (6.23%), and the per-char `code_unit_at → unsafe_to_char → write_char` chain fall away entirely.

## Tests

```
moonbitlang/x/time    148 / 148 pass
```

Applied to `moonbitlang/x` `main`.

## Background

Investigation log + bench at <https://github.com/mizchi/pprof-mbt> (`bench-x/cmd/plain_datetime_parse/`). Same family as the base64 / json5 / encoding / path patches in this series: replace per-element StringBuilder loops with sliced reads from the original string.
