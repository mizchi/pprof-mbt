# Per-request allocation cost of `moonbitlang/async`'s native HTTP server

Findings from running a fresh allocation profiler (`moon-pprof
memprofile-native`, shipped in this repo) against
[`moonbitlang/async`'s
`http_server_benchmark`](https://github.com/moonbitlang/async/tree/main/examples/http_server_benchmark)
on macOS arm64.

**TL;DR.** A minimal `GET /` → `"OK"` handler issues **~120 heap
allocations per request** on the native target. About **70 % of that
cost is the per-`async fn` coroutine state** that gets allocated at
every `await` point on the request path. The hot sites are all async
helpers — `Sender::send_response`, `Sender::write`, `Reader::read_until`,
`ReaderBuffer::find_opt`, `Reader::read_headers`, `coroutine::pause` —
and none of them allocate anything user-visible. The dominant cost is
infrastructure that, structurally, only the compiler / runtime team can
escape-analyze away.

We're filing this as a heads-up rather than an issue because the fix
isn't user-shaped — but the numbers are reproducible (see "Reproducing"
below) and the gap vs. Go / Rust is large enough to matter for anyone
considering MoonBit for HTTP servers.

---

## How the profiler works

`moon-pprof memprofile-native <exe>` (this repo) does **patch +
relink** instead of `LD_PRELOAD`-interposing `malloc`, because MoonBit
native binaries statically link mimalloc and resolve `malloc` calls to
internal symbols — `DYLD_INSERT_LIBRARIES` / `LD_PRELOAD` of `malloc`
never sees them (we measured zero hits across 197 k JSON-parse calls).

The tool instead:

1. Reads the generated `<cmd>.c`, patches `moonbit_malloc_inlined`'s
   body to call a `__moon_pprof_alloc_hook(size)` before the real
   `libc_malloc`.
2. Compiles a bundled `native_alloc_hook.c` (uses `backtrace(3)` +
   `dladdr(3)`) with the same `cc` flags the project's own build uses.
3. Re-runs the project's exact `cc` command with the patched `.c` +
   hook `.o`, output to `<cmd>.memprof.exe`.
4. Runs the patched binary; the hook writes a raw `{bytes, backtrace}`
   stream to a tempfile via a `__attribute__((destructor))`.
5. Aggregates, demangles MoonBit symbols, and emits gzip'd pprof with
   sample types `alloc_objects/count` + `alloc_space/bytes`.

Sampling (`--sample-rate N`) keeps wall time tractable on big workloads
(`backtrace + dladdr` per alloc is what hurts; aggregation is cheap).

So the numbers below are real per-allocation backtraces on a normally-
linked MoonBit native binary, not synthetic.

---

## Workload

```moonbit
async fn main {
  let server = @http.Server(@socket.Addr::parse("0.0.0.0:30001"))
  server.run_forever() <| ((request, _body, conn) => {
    match (request.meth, request.path) {
      (Get, "/") => conn.send_response(200, "OK")
      _ => conn.send_response(404, "NotFound")
    }
  })
}
```

Direct copy of
[`http_server_benchmark.mbt`](https://github.com/moonbitlang/async/blob/main/examples/http_server_benchmark/http_server_benchmark.mbt).
Wrapped in `@async.with_timeout_opt(10_000, ...)` so the binary exits
after 10 s and the `destructor` fires.

Load:
```
wrk -t 8 -c 128 -d 8s http://127.0.0.1:30001/
```

Bench host: M-series macOS, MoonBit `0.1.20260522 (84aa893)`,
`moonbitlang/async` 0.19.1.

---

## Headline numbers

Throughput, **un-instrumented binary**:

| run | req/s |
|---|---|
| 1 | 162 980 |
| 2 | 128 221 |
| 3 | 149 951 |

~150 k req/s average — comparable to Go `net/http` with `GOMAXPROCS=1`
on similar hardware.

**Allocation profile (instrumented, 8 s @ 128 connections, 124 707
requests served):**

* Total allocations: **14 482 000**
* Total bytes allocated: **436.22 MB**
* **Per request: ~116 allocations, ~3.5 kB**

The instrumentation overhead drops req/s from ~150 k to ~15 k. That's
expected — `backtrace + dladdr` on every Nth alloc is heavy. The
**ratio** of allocations across sites is what we care about, and the
runs were stable enough across reruns that the ranking is reliable.

---

## Where the allocations go

Top 10 sites by bytes, baseline (unmodified async 0.19.1):

| % | allocs | site |
|---|---|---|
| 15.7 % | 1 995 500 | `http::Sender::send_response` (compiled variant 1) |
| 14.8 % | 1 497 000 | `http::Server::run_forever` |
| 13.1 % |   996 900 | `http::Sender::send_response` (compiled variant 2) |
| 12.2 % |   997 700 | `io::Writer::write` (default trait impl, monomorphized for `Sender`) |
|  6.1 % |   499 000 | `http::Reader::read_headers` |
|  6.1 % |   498 600 | `io::ReaderBuffer::find_opt` |
|  5.2 % |   499 000 | `io::Reader::read_until` (default impl for `Tcp`) |
|  3.5 % |   499 200 | `http::Reader::read_request` |
|  3.5 % |   498 700 | `io::Writer::write_once` (monomorphized) |
|  3.5 % |   498 500 | `http::Reader::read_request` (variant) |

Every one of these sites is an `async fn`. The "compiled variant N"
duplication is the MoonBit compiler producing multiple specializations
for the same source-level function (probably one per `await` resume
state machine — the `S2495` / `S2527` suffix is the inner async-driver
state number).

For one request:
* `read_headers` + `find_opt` + `read_until` + `read_request` ≈ 4 allocs
  for parsing the request line and headers (one CRLF / `:` search each)
* `send_response` (×2 variants) ≈ 3 allocs for emitting status line +
  the `..write(b"HTTP/1.1 ")..write(code)..write(b" ")..write(reason)..`
  chain
* `Writer::write` (default impl) ≈ 1 alloc *per byte-literal write call*
  — `b"HTTP/1.1 "`, `b" "`, `b"\r\n"`, etc. send_response does ~12 such
  calls

The repeated `4 ≈ requests` and `8-12 ≈ writes per response` line up
with what the code does. There's no user-visible "wasted" allocation.

---

## What kind of objects are these?

`Writer::write` default impl looks like:

```moonbit
impl Writer with write(self, data) {
  let view = data.to_bytesview()       // (1) BytesView alloc
  let start = view.start_offset()
  let len = view.length()
  guard len > 0 else { return }
  let end = start + len
  let offset = self.write_once(view.data(), offset=start, len~)
  ...
}
```

Per call:

1. **One `BytesView` struct** (≈ 24 B: `{ bytes, start, len }`) from
   `data.to_bytesview()`. The default impl in `io/data.mbt` does
   `self.to_bytes()[:]`, and `[:]` is the `%bytesview.make` intrinsic
   which builds a fresh heap object on the native target.
2. **One `async` coroutine frame** — the `write_once` call is itself
   `async fn`, so the compiler builds a state object that captures
   the locals (`view`, `start`, `len`, `end`, `offset`) and persists
   across the await.

For Bytes inputs, the `BytesView` could in principle be passed by
value; on the native target it always boxes.

Similar story for `read_until` and friends — each is an `async fn`
that awaits `read_once`, and the per-`async fn` state captures the
buffer indices.

---

## Why "just inline it" or "just add a `write_bytes` helper" doesn't help

We tried it. Branch: `http/sender-bytes-fast-path` on `mizchi/async`
(not pushed). The change was straightforward — add

```moonbit
async fn Sender::write_bytes(self : Sender, bytes : Bytes) -> Unit {
  let len = bytes.length()
  guard len > 0 else { return }
  let mut pos = 0
  while pos < len {
    let progress = self.write_once(bytes, offset=pos, len=len - pos)
    pos += progress
  }
}
```

and rewrite the ~21 `..write(b"...")` call sites in `http/send.mbt` to
use it. Result over 8 s of `wrk`:

|                       | baseline | with `write_bytes` |
|-----------------------|---------:|-------------------:|
| total allocations     | 14 482 000 | 14 369 200 |
| total bytes           | 436.22 MB  | 547.99 MB  |
| per request           | 116 allocs | 128 allocs |
| `write_bytes` site    |   —        | 1 794 100 allocs |

**Per-request allocs went up.** Each `Sender::write_bytes` call is
still `async fn`, so it allocates its own coroutine frame — and that
frame is roughly the size of the `BytesView` we saved. We traded one
boxed object (BytesView) for another (coroutine frame), with the same
allocation count and similar size.

This is the structural limit: any helper layered on top of an `async
fn` allocates at the layer boundary on the native target. The default
trait dispatch had been folded into the trait-method coroutine frame
already; adding a new method just adds a new frame.

---

## What user-level PRs *can* fix (and what we already shipped)

The pattern that does work is removing allocations that aren't tied to
an `async fn` call boundary — typically intermediate structs / tuples
in synchronous helpers. We shipped four such PRs against
`moonbitlang/core` tonight:

| PR | scope | impact |
|---|---|---|
| [#3632](https://github.com/moonbitlang/core/pull/3632) | `json/lex_skip_whitespace` — scan UTF-16 directly instead of building a `StringView` per call | −26 % alloc bytes on json_parse |
| [#3633](https://github.com/moonbitlang/core/pull/3633) | `json/lex_number` — kill the `(Double, StringView?)` tuple, `#valtype` on `JsonNumberScan`, NaN-sentinel for `try_fast_double` | −50 % allocs on json_numbers, −29 % on json_parse |
| [#3634](https://github.com/moonbitlang/core/pull/3634) | `builtin/Hash::hash` — inline override for 8 primitives (Int, UInt, Int64, UInt64, String, StringView, Char, Bool) to skip per-call `Hasher::new()` | −98 % allocs on hashmap_update (Int keys), −53 % on hashmap_string |
| [#3635](https://github.com/moonbitlang/core/pull/3635) | `String/StringView::to_lower` — replace `for c in self.view(...)` with manual UTF-16 loop to drop the per-call `Iter` alloc | −12.3 % total allocs / −14 MB on this HTTP bench |

PR #3635 is the only one that fires on the HTTP path; it removes
~1.77 M allocations across the 108 k requests measured (~16
allocs/request, mostly from per-header `to_lower` on the request side).
The rest are bench-specific.

After all four, the HTTP server is still at ~100 allocs/req. The
async-fn frames are not reachable from user code.

---

## What the compiler/runtime could do

Ordered by leverage on this workload:

### 1. Escape-analyze `async fn` coroutine state onto the caller's stack

This is the single biggest lever. Going by the top-10 table above,
~75 % of bytes are async-fn frames. Stack-allocating them when the
coroutine doesn't outlive its caller (or doesn't yield across foreign
calls) would drop per-request allocs from ~120 to ~30 — Go-class.

For a `GET /` handler, the entire async chain is one logical
suspendable thread; almost every async fn finishes within the same
task and the frame is dead immediately. The compiler has enough info
to see this in many cases.

### 2. Unbox enum variants without payloads

`json::Token` has `Null | True | False | LBrace | RBrace | LBracket |
RBracket | Comma` as zero-payload variants and `Number(...) |
String(...)` as carriers. On the native target the zero-payload ones
still box. JSON parse top sites show
`lex_value` 400 k allocs and `parse_value2` 392 k allocs in the
post-PR-#3633 profile — a large fraction is `Number(_, _)` and
`String(_)` boxing, which is unavoidable, but the constant variants
are pure overhead.

Same applies to `Option<T>` for T = primitive — `Option<Double>` is
~16 B per call. We worked around it in PR #3633 with a NaN sentinel,
but that's a per-site hack.

### 3. Real `#valtype` for generic structs

`#valtype` works for non-generic `priv struct` but rejects single-
element newtype-tuples ("Value type is not allowed for new type/tuple
struct with one element (which is guaranteed unboxed at runtime)" — so
that case is already fine). The hole is generics: tuples and `Option<X>`
end up boxed even when X is value-typed.

If `Option<X>` could be value-typed when X is, the strconv pattern
(`Number?` → boxed even with `#valtype priv struct Number`) would
collapse. We measured this in PR #3633's exploration — adding
`#valtype` to the struct did nothing because the immediate
`Some(Number)` wrapper still boxes.

### 4. mimalloc → arena for short-lived per-request objects

The HTTP path produces objects with request-scoped lifetime. A bump
arena reset at the end of each request would eliminate the per-alloc
mimalloc bookkeeping cost (which is what currently shows up in CPU
profiles as a non-trivial fraction of request time).

This is a runtime-level change rather than a compile-time one, but
mimalloc's heap API makes it tractable.

---

## Reproducing

In `mizchi/pprof-mbt`:

```sh
# 1. Build the moon-pprof CLI
cargo build --release -p moon-pprof

# 2. Set up the bench project
mkdir -p /tmp/mbt-http-bench/src
cat > /tmp/mbt-http-bench/moon.mod.json <<EOF
{
  "name": "tmp/mbt-http-bench",
  "version": "0.1.0",
  "deps": { "moonbitlang/async": "0.19.1" }
}
EOF
cat > /tmp/mbt-http-bench/src/moon.pkg.json <<EOF
{
  "is-main": true,
  "supported-targets": ["native"],
  "import": [
    "moonbitlang/async",
    "moonbitlang/async/socket",
    "moonbitlang/async/http"
  ]
}
EOF
cat > /tmp/mbt-http-bench/src/main.mbt <<'EOF'
async fn main {
  let _ = @async.with_timeout_opt(10000, () => {
    let server = @http.Server(@socket.Addr::parse("0.0.0.0:30001"))
    server.run_forever() <| ((request, _body, conn) => {
      match (request.meth, request.path) {
        (Get, "/") => conn.send_response(200, "OK")
        _ => conn.send_response(404, "NotFound")
      }
    })
  })
  println("done")
}
EOF
cd /tmp/mbt-http-bench && moon build --target native --release

# 3. Profile while wrk drives load
target/release/moon-pprof memprofile-native \
  /tmp/mbt-http-bench/_build/native/release/build/src/src.exe \
  --out /tmp/mem.pb.gz --sample-rate 100 &
SPID=$!
sleep 8                          # let the build + relink + bind finish
wrk -t 8 -c 128 -d 8s http://127.0.0.1:30001/
wait $SPID

# 4. Read the result
target/release/moon-pprof summary /tmp/mem.pb.gz
# or open in any pprof UI:
go tool pprof -alloc_space -http :8000 /tmp/mem.pb.gz
```

A 10 s run is enough to get stable rankings on the top 15 sites.

---

## Honest scope notes

* The instrumentation drops throughput ~10×. The numbers above are
  **allocation counts and ratios**, which are stable across reruns;
  they are not direct latency measurements. For latency, run the
  un-instrumented binary under `wrk` or `samply`.
* All measurements are on macOS arm64. Linux numbers should be similar
  in the alloc count column but the allocator (jemalloc vs mimalloc on
  some builds) shifts the bytes column. The per-request alloc *count*
  is what matters for the comparison.
* This investigation explored only the HTTP server. The same
  `memprofile-native` workflow applies to any `--target native` binary;
  the four shipped PRs came out of running it on the JSON-parse and
  hashmap benches in `mizchi/pprof-mbt/bench/cmd/`.
* "Go-class" / "Rust-class" comparisons are reference points, not a
  knock — Go achieves its alloc count via deep compiler work on
  escape analysis, and Rust avoids the question entirely with
  borrow-checked stack frames. The MoonBit native runtime is in good
  shape considering it's recent; the message of this report is that
  the bottleneck is well-localized and tractable.
