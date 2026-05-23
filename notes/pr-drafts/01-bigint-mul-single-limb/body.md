## Summary

`BigInt::mul` currently routes everything through `grade_school_mul` unless both operands cross `karatsuba_threshold`. For asymmetric cases where one operand fits in a single radix limb — factorial chains, small-scalar multiplications, etc. — the general nested loop with the per-iteration `j < other_len || carry != 0` guard does measurable extra work.

Add a `mul_single_limb` fast path that runs a single carry-propagating loop in `O(self.len)`. The dispatch in `Mul::mul` checks both operands so the fast path fires regardless of operand order.

## Benchmarks

Setup: moonbit 0.1.20260522 + this patch, Linux x86_64, `--no-profile` wall time (3-run median).

`factorial(800)` (100 iterations), which always multiplies an accumulator by a 1-limb integer:

| backend  | baseline | patched  | delta  |
|----------|---------:|---------:|-------:|
| wasm     | 255.1 ms |  80.2 ms | **-68.6%** |
| wasm-gc  |  93.5 ms |  25.2 ms | **-73.0%** |
| native   |  48.9 ms |  20.0 ms | **-59.1%** |
| js       |  21.2 ms |  22.3 ms |  noise  |

(js is unchanged because the JS backend transpiles `BigInt` to V8's native `BigInt` rather than going through `grade_school_mul`.)

Balanced-multiplication probe (repeated squaring of a 30-digit seed × 11 iterations, both operands grow together so the existing Karatsuba path is exercised) confirms the n×n path is not regressed:

| backend  | baseline | patched  | delta |
|----------|---------:|---------:|------:|
| wasm     | 257.1 ms | 271.9 ms | noise |
| wasm-gc  |  84.8 ms |  78.2 ms | -7.8% |
| native   |  57.0 ms |  58.0 ms | noise |

## Test results

`moon test` against this branch on all four targets (full core suite):

| target  | result |
|---------|--------|
| wasm    | 6500 / 6500 pass |
| wasm-gc | 6500 / 6500 pass |
| js      | 6459 / 6459 pass |
| native  | 6411 / 6411 pass |

## Background

This came out of a cross-backend profiling exercise of `moonbitlang/core` documented at <https://github.com/mizchi/pprof-mbt>. The profile on wasm-gc factorial showed `grade_school_mul` at ~70% of self time — the dominant inner work is the carry-propagation guard, not the actual `mul`. Specializing the asymmetric case removes the guard entirely.

## Reproducing

```sh
# Clone https://github.com/mizchi/pprof-mbt
cd pprof-mbt
.bin/patched-toolchain init
.bin/patched-toolchain apply notes/pr-drafts/01-bigint-mul-single-limb/patch.diff
.bin/patched-toolchain rebundle
.bin/bench-runner --workloads bigint_ops,bigint_square --runs 3
```
