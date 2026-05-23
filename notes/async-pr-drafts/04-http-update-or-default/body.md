## Summary

`Reader::read_headers` in `src/http/parser.mbt` does two `Map` operations per header:

```moonbit
match headers.get(key) {
  None => headers[key] = value
  Some(value0) => headers[key] = "\{value0},\{value}"
}
```

`Map::get` walks the Robin-Hood probe sequence once; the subsequent `headers[key] = ...` walks it again from scratch. `Map::update_or_default(key, default, update_fn)` already exists for exactly this pattern and does a single probe.

```moonbit
headers.update_or_default(key, value, value0 => "\{value0},\{value}")
```

## Why bother

Headers are parsed for every HTTP request. The save is small per call (one extra hash + one probe walk) but compounds across the request lifetime — and the new version is semantically identical and arguably clearer.

In a callgrind profile of `examples/http_server_benchmark/http_server_benchmark.mbt` driven by k6 (3,330 req/s under valgrind, 67 kreq/s native), `read_headers` itself is 2.40% of instructions and the Map insert path inside it is a slice of that. Wall-time delta is within the run-to-run noise (~±2%) on a `GET /` workload — the visible work is dominated by `moonbit_drop_object` + malloc — but the patch costs nothing.

## Test results

```
moonbitlang/async/http                   51 / 51 pass
moonbitlang/async                        87 / 87 pass
moonbitlang/async/aqueue                 35 / 35 pass
moonbitlang/async/cond_var                5 /  5 pass
moonbitlang/async/semaphore               6 /  6 pass
moonbitlang/async/internal/coroutine      1 /  1 pass
moonbitlang/async/internal/event_loop     7 /  7 pass
```

Network tests (socket / tls / http server roundtrips with real interfaces) require a network interface the profiling sandbox lacks; this patch doesn't touch any code on that path.

## Background

Came out of an HTTP server benchmark + callgrind sweep done as part of the `moonbitlang/async` investigation at <https://github.com/mizchi/pprof-mbt> (see `notes/async_http_server_profile.md` for the full profile shape and call-site breakdown).
