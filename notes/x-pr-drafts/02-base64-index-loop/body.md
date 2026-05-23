## Summary

Both `Encoder::encode_to` and `Decoder::decode_to` in `codec/base64/base64.mbt` walk their input with `for byte in bytes` / `for ch in input`. That desugars through `BytesView::iter` / `StringView::iter` plus an `Iter::next` call per element. On a 64 KiB benchmark those iterators account for **~32% of the encode self time** and **~35% of the decode self time** — even though the inner-loop body is a tiny match.

Switch both to index loops:

- `encode_to`: pull the backing `Bytes` and `start_offset` once and index `data[start + idx]` per byte.
- `decode_to`: iterate `0..<input.length()` and read `input.unsafe_get(off)` (UTF-16 code unit) directly. base64's alphabet is pure ASCII, so reading by code unit is safe; if a payload contains a high surrogate it isn't valid base64 anyway and `char_to_index` raises `InvalidChar` as before.

## Benchmark

Setup: 64 KiB payload, 500 iterations, Linux x86_64 native release. 3-run median wall time.

| workload      | baseline | patched | delta   |
|---------------|---------:|--------:|--------:|
| base64_encode |  710 ms  | 564 ms  | **-20.6%** |
| base64_decode |  685 ms  | 432 ms  | **-36.9%** |

Callgrind self-time delta (encode, top symbols):

| symbol                                | before  | after  |
|---------------------------------------|--------:|-------:|
| `BytesView::iter` closure (`for byte in`) | 19.68%  | gone   |
| `Iter::next<Byte>`                    | 12.20%  | gone   |
| **`Encoder::encode_to`**              | 14.56%  | (now sees the loop body directly) |

Same shape on decode: 23.37% `StringView::iter` + 11.91% `Iter::next<Char>` collapse to a single index step.

## Tests

```
moonbitlang/x/codec/base64    3 / 3 pass
```

## Background

Came out of the moonbitlang/x sweep. Same iter-overhead pattern as the moonbitlang/async gzip `crc32_update` patch (see <https://github.com/mizchi/pprof-mbt/blob/main/notes/async_crc32_index_loop.diff>): `for byte in BytesView` / `for ch in StringView` is convenient to write but, in tight per-byte loops, the iter closure allocation + `Iter::next` dispatch dominates over the loop body. Direct indexing of the backing buffer is intrinsic.

Profile + notes at <https://github.com/mizchi/pprof-mbt/blob/main/notes/x_investigation.md>.
