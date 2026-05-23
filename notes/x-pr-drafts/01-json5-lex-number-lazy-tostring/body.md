## Summary

`lex_number_end` currently does:

```moonbit
let s = ctx.input[start:end].to_owned()
@string.parse_double(s) catch {
  _ => parse_error(InvalidNumber(..., s))
}
```

`@string.parse_double` already accepts `StringView`. The `.to_owned()` materializes a fresh `String` for every parsed number — but the only consumer that actually needs an owned string is the *error* `parse_error` call, which is the cold path. Push the `to_owned` / `to_string` inside the `catch` arm and pass the `StringView` directly on the happy path.

## Benchmark

Setup: a copy of moonbitlang/core's `json_parse` bench but parsing 1000-element JSON5 object arrays (unquoted keys, trailing commas, single-quoted strings) × 50 iters. Native release, Linux x86_64, 3-run median wall time.

|           | baseline | patched | delta  |
|-----------|---------:|--------:|-------:|
| native    | 231 ms   | 219 ms  | **-5.2%** |

The patched payload contains 8 numbers per object × 1000 objects × 50 iters = 400 000 number parses; that's exactly the number of `to_owned`/`to_string` allocations skipped.

Callgrind profile before this change (top 15 of 2.63 G Ir for the same workload):

```
10.27% read_char
 7.83% moonbit_drop_object
 7.67% String::offset_of_nth_char_forward
 7.61% free
 7.36% lex_value
 4.10% strconv::fold_digits
 3.50% _mi_page_malloc_zero
 3.22% strconv::check_underscore
 3.22% malloc
 2.78% String::sub
 ...
```

The malloc/free + drop_object band (~26%) is partly the `to_owned` churn this patch removes.

## Tests

```
moonbitlang/x/json5    74 / 74 pass
```

Applied to `moonbitlang/x` `main`. `moon fmt` is a no-op on the change.

## Background

Investigation at <https://github.com/mizchi/pprof-mbt> (`bench-x/cmd/json5_parse/`).
