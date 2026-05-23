## Summary

`crc32_update` iterates the input via `for byte in chunk` where `chunk : BytesView`. That desugars through `BytesView::iter` + `Iter::next`, which allocates an iterator closure and pays a virtual dispatch per byte. In a `gzip_roundtrip` callgrind profile that single loop accounts for **~16% of total instructions** (BytesView::iter 9.80% + Iter::next 6.07%), even though the loop body is a couple of arithmetic ops and a lookup.

Switching to a direct index over the backing `Bytes` removes both the closure allocation and the iterator dispatch. The intrinsic `Bytes[i]` is inlined.

## Benchmark

Setup: `bench-async/cmd/gzip_roundtrip` (3 iter × 1000 chunks × 1024-byte payload, encode → pipe → decode), Linux x86_64, native build, 3-run median wall time.

|        | baseline | patched | delta |
|--------|---------:|--------:|------:|
| native | 178 ms   | 162 ms  | **-9.0%** |

Callgrind self-time delta:

| symbol                | before | after | Δ |
|-----------------------|-------:|------:|---:|
| `BytesView::iter` (closure) |  9.80% |   —   | gone |
| `Iter::next`          |  6.07% |   —   | gone |
| `BytesView::at`       |    —   |   —   | (used `Bytes[i]` instead) |
| `crc32_update`        |  3.55% | 4.57% | +1.02% (body now visible directly) |

Net: -9.85% on this hot path, which lines up with the -9% wall-time drop.

## Test results

```
moonbitlang/async/internal/gzip_internal  31 / 31 pass
moonbitlang/async/gzip                     7 /  7 pass
moonbitlang/async                         87 / 87 pass
moonbitlang/async/aqueue                  35 / 35 pass
moonbitlang/async/semaphore                6 /  6 pass
moonbitlang/async/cond_var                 5 /  5 pass
moonbitlang/async/internal/coroutine       1 /  1 pass
moonbitlang/async/internal/event_loop      7 /  7 pass
moonbitlang/async/internal/time            3 /  3 pass
```

Socket / TLS / HTTP / fs / process / websocket tests require a network interface that the profiling sandbox lacks; they're untouched by this patch.

## Background

Investigation log + new `bench-async/cmd/gzip_roundtrip` workload at <https://github.com/mizchi/pprof-mbt>.
