## Summary

`lex_number_end` calls `scan_json_number` to scan every digit once and compute `mantissa` / `exponent`. For integers it then dispatches to `lex_integer_end`, which **scans the same digits a second time** to build an `Int64` accumulator with overflow detection.

For safe integers (mantissa ≤ `2^53 − 1`, not `many_digits`) `scan.mantissa` already *is* the absolute value. Use it directly. Overflow / many-digits paths still defer to `lex_integer_end` so the existing behavior is preserved.

This sits on top of #3593 / #3594 (the JSON parser fast-path work merged in May): it removes the remaining redundant scan that those PRs left in place.

## Benchmarks

Setup: moonbit 0.1.20260522 + this patch, Linux x86_64, `--no-profile` wall time (3-run median).

| workload     | backend  | baseline  | patched   | delta   |
|--------------|----------|----------:|----------:|--------:|
| json_numbers | wasm     | 167.4 ms  | 158.2 ms  |  -5.5%  |
| json_numbers | wasm-gc  | 129.9 ms  | 117.6 ms  |  **-9.5%**  |
| json_numbers | js       | 247.6 ms  | 180.8 ms  | **-27.0%**  |
| json_numbers | native   |  54.5 ms  |  55.6 ms  |  noise  |
| json_parse   | wasm     | 578.8 ms  | 566.0 ms  |  -2.2%  |
| json_parse   | wasm-gc  | 222.7 ms  | 223.8 ms  |  noise  |
| json_parse   | js       | 341.8 ms  | 336.8 ms  |  -1.5%  |

- `json_numbers`: flat array of 10000 safe integers — the patch fires on every value.
- `json_parse`: the existing mixed-object array bench. Each object has 4 integers + 2 doubles + several strings, so the fast path covers a smaller fraction of work; gain is consequently smaller.

The js -27% on `json_numbers` is the most striking: V8 inlines the safe-int branch cleanly so dispatch + a second digit walk collapses to a single arithmetic chain.

## Test results

| target  | result |
|---------|--------|
| wasm    | 6500 / 6500 pass |
| wasm-gc | 6500 / 6500 pass |
| js      | 6459 / 6459 pass |
| native  | 6411 / 6411 pass |
