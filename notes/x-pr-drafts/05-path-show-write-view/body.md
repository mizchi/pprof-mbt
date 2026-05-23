## Summary

`UnixPath`'s `Show::output` writes each path component via:

```moonbit
logger.write_string(x.to_owned())
```

where `x` is a `StringView`. The `to_owned()` step copies the StringView into a newly allocated `String`, which `write_string` then immediately copies into the underlying builder. That's **one allocation + two copies per component**, where one copy would suffice.

`Logger` already has a `write_view(StringView)` method that targets exactly this case — no intermediate `String`.

```moonbit
logger.write_view(x)
```

The same pattern appears in `path/win32/win_path.mbt`'s `WinPath::output` (writes each component as `write_string(component.to_owned())`); fixed identically in this patch.

## Why this is hot

`Path::normalize(p)` is `UnixPath::parse(p.0).to_string()` — so every normalize call goes through `Show::output` and pays the per-component copy cost. On a realistic path with ~5–10 components the redundant work adds up fast.

## Benchmark

`bench-x/cmd/path_normalize/` — `Path::normalize` + `dirname` + `basename` + `extname` on a 10-segment path with `..` / `.`, × 200 000 iters. Native release, Linux x86_64, 3-run median wall time.

|                | baseline | patched | delta  |
|----------------|---------:|--------:|-------:|
| path_normalize |  282 ms  | 209 ms  | **-25.9%** |

Callgrind total instructions: 3.73 G → 2.71 G (**-27.4%**). `FixedArray::blit_from_string` falls from 15.19% of the profile to 8.99% (and to a much smaller absolute number).

## Tests

```
moonbitlang/x/path/posix    38 / 38 pass
moonbitlang/x/path/win32    50 / 50 pass
```

Applied to `moonbitlang/x` `main`.

## Background

Investigation log + bench at <https://github.com/mizchi/pprof-mbt> (`bench-x/cmd/path_normalize/`). Same iter-overhead / unnecessary-copy pattern as the base64, json5, encoding, and uuid patches in this series.
