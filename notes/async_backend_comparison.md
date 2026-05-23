# `moonbitlang/async` × 4 バックエンド比較

`bench-async/` の 5 つの pure-coroutine workload を **native / wasm /
wasm-gc / js** で測った。IO を使う `timer_burst` / `buffered_io_pipe`
/ `gzip_roundtrip` は native のみで動くので除外。

## wasm / wasm-gc を動かすための小さなパッチ

upstream の `moonbitlang/async/src/integration.mbt` は `run_async_main`
を `#cfg(target="native")` と `#cfg(target="js")` でしか定義していない。
wasm-gc / wasm 用に **JS と同じシム** を追加:

```moonbit
#cfg(target="wasm-gc")
#doc(hidden)
pub fn run_async_main(main : async () -> Unit) -> Unit {
  let _ = @coroutine.spawn(main)
  @event_loop.reschedule()
}
```

そして `internal/event_loop/unimplemented.mbt` に `reschedule()` を追加:

```moonbit
pub fn reschedule() -> Unit {
  while !@coroutine.no_more_work() {
    @coroutine.reschedule()
  }
}
```

これで IO 不要な workload (pause / spawn / aqueue / semaphore / cond_var)
は wasm/wasm-gc で動く。Timer や socket を呼ぶ workload は既存の
`abort("...unimplemented on WASM backend")` で死ぬ — 想定通り。

## 測定結果

3 run の中央値、`--no-profile` 経路 (wasmtime-runner / run-wasm-gc.mjs
/ run-js.mjs の inspector を切る)。Linux x86_64, moonbit 0.1.20260522。

| workload              | native | wasm   | wasm-gc | js          |
|-----------------------|-------:|-------:|--------:|------------:|
| pause_loop            |  96 ms |  97 ms |  **32 ms** | **> 30 s** (timeout) |
| spawn_wait            |  63 ms | 241 ms |   90 ms |        8 ms |
| aqueue_throughput     |  72 ms | 285 ms |   74 ms |        9 ms |
| semaphore_throughput  |  63 ms | 249 ms |   61 ms |        9 ms |
| cond_var_signal       | 136 ms | 279 ms |   68 ms |        9 ms |

## 観察

### 1. wasm-gc が native より速いケースが多い

| workload | native | wasm-gc | wasm-gc / native |
|---|--:|--:|--:|
| **pause_loop**     |  96 |  **32** | **0.33×** (wasm-gc が 3× 速い) |
| spawn_wait         |  63 |   90 | 1.43× (native 勝ち) |
| aqueue_throughput  |  72 |   74 | 1.03× (互角) |
| semaphore          |  63 |   61 | 0.97× (互角) |
| **cond_var_signal**| 136 |  **68** | **0.50×** (wasm-gc が 2× 速い) |

理由: native は `wait_for_event` で `epoll_wait` + `ms_since_epoch` +
timer SortedSet 走査を毎ティック実行する (Patch B 適用後でも timer scan
は省けるが epoll/time は残る)。wasm-gc は `@coroutine.reschedule()` を
直接ループするだけなので、event loop オーバーヘッドが丸ごと消える。

`pause_loop` は wait_for_event を毎 pause で踏むので差が顕著
(native の 1/3)。`cond_var_signal` も同様。`spawn_wait` は spawn が
重く wait_for_event 比率が低いので native が勝つ。

### 2. wasm は wasm-gc の 3〜4× 遅い (refcount オーバーヘッド)

| workload | wasm-gc | wasm | wasm / wasm-gc |
|---|--:|--:|--:|
| pause_loop  | 32 |  97 | **3.0×** |
| spawn_wait  | 90 | 241 | 2.7× |
| aqueue      | 74 | 285 | 3.9× |
| semaphore   | 61 | 249 | **4.1×** |
| cond_var    | 68 | 279 | **4.1×** |

`pprof-mbt` の `notes/backend_comparison.md` で見た core 同等の比率。
async でも **refcount オーバーヘッドが wasm を一貫して 3〜4× 遅くする**。
coroutine struct / Set[Coroutine] / Deque<Waiter> 等の参照が毎 op
incref/decref されるため。

### 3. JS は workload 性質で 1000× 差

JS で同じ workload を走らせると `spawn_wait` 等は **8〜9 ms** で
終わる一方、`pause_loop` は **30 s 経っても終わらない**。

これは `event_loop.js.mbt` の `reschedule()` 実装が:

```moonbit
pub fn reschedule() -> Unit {
  ...
  @coroutine.reschedule()  // 1 ラウンド
  if @coroutine.has_immediately_ready_task() {
    ignore(set_timeout(0, reschedule))  // 次は JS event loop 経由
  }
}
```

意図的に `setTimeout(0)` で JS event loop に yield する設計のため。
コメント曰く:

> "Remaining tasks are delayed until the next js event loop,
>  so that those blocking jobs got a chance to execute instead of starving."

これの効果:
- `pause_loop` (1 タスクが 500k 回 pause): 各 pause が 1 ラウンド =
  1 setTimeout(0) = 1ms+ → 500k × 1ms = 500 秒級 → timeout
- `spawn_wait` (5000 タスク同時 pause): 5000 タスクが 1 ラウンドで全部
  処理される → setTimeout(0) は数回しか発生しない → 8ms 完走

つまり **JS は per-pause コスト** が高い設計だが、**並列タスクが多い
ほど効率が良い** (1 ラウンドで複数進む)。これは設計選択であって
バグではない。

### 4. spawn_wait の native が wasm-gc に勝つ理由

spawn_wait は 150k タスクの spawn + wait を行う。spawn は:
- `Coroutine` struct alloc
- `Set[Coroutine].add` (downstream tracking)
- run_later への push

これらは event_loop ではなく scheduler のホットパス。native の C コンパイル
(clang -O2 + 直接 struct field access) が wasm-gc の JIT より効率的。
特に Set[Coroutine] の Robin-Hood probe で。

### 5. pause_loop の wasm-gc がここでだけ native を上回るのは event_loop コスト

native 96ms = pure scheduler 32ms (wasm-gc と同等と仮定) + event_loop
overhead 64ms。500k pause × 0.13 μs/pause = ~64ms。1 pause あたり
~130 ns の event_loop オーバーヘッドという見積もり。

## まとめ表 (relative to wasm-gc)

| workload              | native | wasm  | wasm-gc | js         |
|-----------------------|-------:|------:|--------:|-----------:|
| pause_loop            |  3.0×  |  3.0× |   1.0×  | (timeout)  |
| spawn_wait            |  0.7×  |  2.7× |   1.0×  | ~0.09× (条件付き) |
| aqueue                |  1.0×  |  3.9× |   1.0×  | ~0.12×     |
| semaphore             |  1.0×  |  4.1× |   1.0×  | ~0.15×     |
| cond_var              |  2.0×  |  4.1× |   1.0×  | ~0.13×     |

**結論**: 純コルーチンワークロードでは
- **wasm-gc が最良** (event_loop オーバーヘッドなし + GC で refcount なし)
- **native** は IO ありなら勝つが、IO なしなら event_loop コストで負ける
- **wasm** は refcount で常に 3〜4× 遅い (core の話と一致)
- **js** は workload の並列度で挙動が両極端 (per-pause が高コスト、
  per-task が低コスト) — 設計上の意図的なトレードオフ

## 再現

```sh
cd bench-async
# wasm-gc 用に async に小パッチ (notes/async-pr-drafts/02-wasm-gc-shim/ TODO)
# native は既存通り
for w in pause_loop spawn_wait aqueue_throughput semaphore_throughput cond_var_signal; do
  for t in native wasm wasm-gc; do
    moon build --release --no-strip --target=$t cmd/$w
  done
  moon build --release --target=js cmd/$w
done
# 測定は notes/async_backend_comparison.md の表の作り方を参照
```
