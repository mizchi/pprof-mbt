# `moonbitlang/x` investigation

Same workflow as the core / async investigations: add bench workloads,
profile native via valgrind callgrind, look for tractable patches.

[`moonbitlang/x`](https://github.com/moonbitlang/x) is the experimental
package set destined for eventual graduation to `moonbitlang/core`.
Profiled four representative workloads:

## Bench setup

`bench-x/` is a new moon project that depends on
`moonbitlang/x` 0.4.43 (registry latest at time of writing).

| workload | what it does |
|---|---|
| `sha256_hash` | hash a 64 KiB payload 500× via `@crypto.sha256` |
| `md5_hash` | same, MD5, 1000× |
| `chacha20` | encrypt 64 KiB 200× via `@crypto.chacha20` |
| `json5_parse` | parse a 1000-element JSON5 array of mixed objects 50× (uses unquoted keys, single-quoted strings, trailing commas) |

## Profile shape (callgrind native, single run)

### sha256_hash (3.42 G Ir)

| % | symbol |
|---|---|
| **82.53%** | `SHA256::transform` — the compression function |
| 11.26% | `bytes_u8_to_u32be` — 4-byte → u32 big-endian conversion |
|  2.82% | `moonbit_drop_object` |
|  0.72% | `SHA256::update` |

Cleanly algorithmic — transform is the inner SHA-256 loop. Alloc is <3%.

### md5_hash (2.86 G Ir)

| % | symbol |
|---|---|
| **51.67%** | `md5_update` |
| **24.69%** | `u8_to_u32le` — 4-byte → u32 little-endian conversion |
| 23.54% | `md5_transform` |

Similar shape to SHA-256 but the LE conversion is a higher fraction.

### chacha20 (2.01 G Ir)

| % | symbol |
|---|---|
| **60.29%** | `quarterRound` (the ChaCha quarter-round) |
| 13.04% | `chacha` |
|  9.06% | `chachaBlockRound` |
|  6.44% | `stateToBytes` (output u32 → bytes) |
|  4.53% | `chachaBlock` |
|  1.34% | drop_object |

Algorithmic core. The 6.44% in `stateToBytes` is the only non-algorithmic
slice.

### json5_parse (2.63 G Ir)

| % | symbol |
|---|---|
| 10.27% | `read_char` |
|  7.83% | `moonbit_drop_object` |
|  7.67% | `String::offset_of_nth_char_forward` |
|  7.61% | `free` |
|  7.36% | `lex_value` |
|  4.10% | `strconv::fold_digits` |
|  3.50% | `_mi_page_malloc_zero` |
|  3.22% | `strconv::check_underscore` |
|  3.22% | `malloc` |
|  2.78% | `String::sub` |
|  2.66% | `lex_ident` |
|  2.60% | `lex_skip_whitespace` |
|  2.40% | `String::char_length_ge` |

Same shape as core's `json_parse` (see
`notes/json_parse_findings.md`): UTF-16 char indexing is expensive,
and the number-parsing path goes through `strconv::fold_digits` etc.
on `StringView`.

## Patches tried

### ❌ `crypto/utils.mbt` micro tweak (no-op)

`u8_to_u32le` calls a local helper `uint32(b) -> UInt` defined as
`b.to_int().reinterpret_as_uint()`. Replaced with `b.to_uint()` directly
and added `#inline` to the helper + the conversion functions.

Result: wall time unchanged (within run-to-run noise: sha256 192ms both
ways, md5 220ms both ways, chacha20 109→106ms which is noise). moonc
apparently compiles both sequences to the same machine code. Same
lesson as the earlier Hasher `#inline` experiment on core. **Dropped**.

### ✅ `uuid/uuid.mbt` to_string in-place reinterpret (PR-04, largest win)

`UUID::to_string` builds the hex representation into a `FixedArray[Byte]`
of size 72, then does:

```moonbit
Bytes::from_array(Array::from_fixed_array(rv)).to_unchecked_string()
```

Two unnecessary copies. The FixedArray already *is* the UTF-16 byte
buffer. Replace with `rv.unsafe_reinterpret_as_bytes().to_unchecked_string()`.

|             | baseline | patched | delta   |
|-------------|---------:|--------:|--------:|
| uuid_parse  |  549 ms  | 197 ms  | **-64.1%** |

9/9 uuid tests pass. **Single largest win across all three repos.**

### ✅ `encoding/encoding.mbt` UTF-8 code-unit walk (PR-03)

`encoding::encode(UTF8, ...)` had `for char in src` over `String`,
which iterates by Char (with surrogate decode per char). Replaced
with an explicit `while i < len` walk over UTF-16 code units that
assembles surrogate pairs inline.

|                | baseline | patched | delta   |
|----------------|---------:|--------:|--------:|
| encoding_utf8  |  528 ms  | 408 ms  | **-22.7%** |

71/71 encoding tests pass.

### ✅ Cross-repo cascade: `moonbitlang/core` PR-01 helps `moonbitlang/x/decimal`

`decimal_arith` (factorial-style `acc * Decimal::from_int(i)`) is 90.78%
in `BigInt::grade_school_mul`. With our pending `moonbitlang/core`
PR-01 (`bigint mul_single_limb`) applied — no x-side change at all —
the same bench drops from **170 ms to 47 ms (-72%)**. The decimal value
holds a `BigInt` coefficient + a scale, so each step is exactly the
(n-limb) × (1-limb) pattern PR-01 specialized for.

This makes core PR-01 a stronger PR (it doesn't just help bigint
benches; it cascades into x/decimal users).

### ✅ `codec/base64/base64.mbt` index-based loop (PR-02)

Both `Encoder::encode_to` and `Decoder::decode_to` walked their inputs
via `for byte in bytes` / `for ch in input`. Both iterators are
~32–35% of the respective self time even though the inner-loop body
is a small match. Switched to:

- `encode_to`: index the backing `Bytes` via `data() + start_offset()`,
  iterate `0..<len`.
- `decode_to`: iterate `0..<input.length()` and read
  `input.unsafe_get(off)` (UTF-16 code unit) directly. base64 alphabet
  is pure ASCII so reading by code unit is safe.

Result on `bench-x/cmd/base64_*` (64 KiB × 500 iters):

|               | baseline | patched | delta   |
|---------------|---------:|--------:|--------:|
| base64_encode |  710 ms  | 564 ms  | **-20.6%** |
| base64_decode |  685 ms  | 432 ms  | **-36.9%** |

Same iter-overhead pattern as moonbitlang/async's gzip `crc32_update`
patch and the json5 `to_string` patch below. 3/3 `codec/base64` tests
pass on upstream main with this applied.

### ✅ `json5/lex_number.mbt` lazy `to_string` (PR-01)

`lex_number_end` did `let s = ctx.input[start:end].to_owned()` before
calling `@string.parse_double(s)`. `parse_double` already accepts
`StringView`, so the owned-String materialization is only useful for
the error payload (`parse_error(InvalidNumber(..., s))`) — which is
cold. Pushed the `to_string` inside the catch arm.

Result on `json5_parse`:

|        | baseline | patched | delta   |
|--------|---------:|--------:|--------:|
| native |  231 ms  | 219 ms  | **-5.2%** |

74/74 `moonbitlang/x/json5` tests pass.

The saved work is 400 000 String allocations per run (8 numbers per
object × 1000 objects × 50 iters). The relative gain matches what we
saw in the `notes/async_http_parser_update_or_default.diff` patch:
freeing one allocation per hot path step.

## Future directions

Things noticed but not pursued:

1. **`crypto::SHA256::transform` and `md5_transform`** dominate their
   profiles (>50%). They're the algorithmic core. Loop-unrolling or
   inlining the round helpers could shave a few %, but the work is
   inherent. SIMD wouldn't be portable to wasm.
2. **`json5::lex_ident` calls `read_char` per char** to walk an
   identifier (object key). Each `read_char` allocates an `Option<Char>`.
   For all-ASCII identifiers (the common case), a code-unit fast path
   that skips the surrogate-pair handling would be faster. Larger
   patch, not attempted.
3. **`bytes_to_hex_string`** in `crypto/utils.mbt` is `ret = high + low + ret`
   in a loop — O(n²) string concatenation. Not on a hot bench path here
   (it's only called when stringifying the digest at the end), but it's
   a separate cleanup opportunity.
4. **json5 number parsing has no `scan_json_number`-style fast path**
   like core json's PR-03 added. json5 always defers to `@string.parse_double`.
   Adding a safe-int short-circuit would help, but it duplicates code
   from core. Probably wait for the convergence between core json
   and x/json5 before chasing this.

## PR artifacts

`notes/x-pr-drafts/01-json5-lex-number-lazy-tostring/` is shaped exactly
like the other PR drafts: `title.txt`, `body.md`, `patch.diff`, and
`0001-*.patch` for `git am`.
