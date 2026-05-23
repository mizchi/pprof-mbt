# `moonbitlang/async` の http server を k6 で負荷 + callgrind で profile

`bench-async/cmd/http_server_benchmark/` (上流の
`examples/http_server_benchmark/http_server_benchmark.mbt` をそのまま
コピー) を:
1. k6 で生 RPS 計測
2. valgrind callgrind 下で再起動 → k6 で軽負荷をかけて関数別 instruction
   カウント取得

の 2 段でやった。

## サーバーの中身

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

GET / だけ 200 で返す最小エンドポイント。HTTP パースと socket I/O の
オーバーヘッドが主な計測対象。

## 生 RPS (Linux x86_64, native release build)

`pprof-mbt/bench-async/cmd/http_server_benchmark/k6_script.js`:

```js
import http from 'k6/http';
export const options = {
  scenarios: {
    constant: { executor: 'constant-vus', vus: __ENV.VUS, duration: __ENV.DURATION },
  },
};
export default function () { http.get('http://127.0.0.1:30001/'); }
```

| VUs | baseline (pristine async) | patched (Patch B + C) |
|----:|--------------------------:|----------------------:|
|  64 | 62,696 req/s              | 61,700 req/s          |
| 128 | 65,578 req/s              | 65,851 req/s          |
| 256 | 66,717 req/s              | 64,949 req/s          |

- 5s × 3 VU レベル
- パッチは誤差範囲。HTTP server は **常に timer (request idle timeout等)
  を抱えている** ので Patch B (空-timer short-circuit) は発火しない。
  Patch C (crc32) は gzip 使わないので無関係。
- ピーク ~67 kreq/s @ 128 VUs (single core 想定の HTTP server として
  なかなか良い数字)。

### `wrk` との対応

リポジトリ既存の `examples/http_server_benchmark/bench.sh` は `wrk` で
同じサーバーに対して 8 thread × 64/128/256 conn × 15s を回す。k6 の
VU model は wrk の connection model と直交しないが、同 64 conn の
RPS は概ね一致するレンジに収まった。

## Callgrind プロファイル

callgrind は ~20× の slowdown が入るので RPS は 3,220/s に落ちる
(VUs=8, 3s)。Profile 形状は valid。

Top 25 関数 (3,220 req/s 中で 363M instructions 集計):

| %      | symbol |
|-------:|--------|
| 14.02% | `moonbit_drop_object` (runtime, refcount drop) |
| 11.55% | `free` |
|  5.29% | `_mi_page_malloc_zero` |
|  4.89% | `malloc` |
|  2.99% | `BytesView::find` |
|  2.62% | `ReaderBuffer::find_opt` (探す `\r\n\r\n` 等) |
|  2.40% | `http::Reader::read_headers` |
|  2.39% | `FixedArray::blit_from_string` |
|  2.31% | `http::Sender::send_response` |
|  2.15% | `StringView::iter` |
|  1.69% | `Writer::write` (trait default impl) |
|  1.66% | `http::Server::run_forever` |
|  1.49% | `StringView::to_lower` |
|  1.40% | `simdutf::haswell::utf16_length_from_utf8` |
|  1.38% | `StringBuilder::write_char` |
|  1.37% | `Hasher::combine_string` |
|  1.32% | `simdutf utf8_to_utf16 (scalar)` |
|  1.29% | `Reader::read_until` |
|  1.16% | `simdutf utf8_to_utf16le_with_errors` |
|  1.12% | `_mi_malloc_generic` |
|  1.10% | `Iter::next` on Char |
|  1.09% | (other) |
|  1.06% | `Writer::write` continuation |
|  1.02% | `StringBuilder::grow_if_necessary` |
|  0.96% | `brute_force_find` |

### グルーピング

| グループ | 合計 % | 主な symbol |
|---|--:|---|
| **alloc / free** | **~32%** | malloc / free / mi_page / drop_object |
| **HTTP パース** | **~13%** | BytesView::find, ReaderBuffer::find_opt, read_headers, StringView::iter, to_lower, brute_force_find |
| **simdutf (UTF-8 ↔ UTF-16)** | **~3.7%** | utf16_length_from_utf8 + utf8_to_utf16 paths |
| **trait dispatch** | ~3% | Writer::write default impl + continuations |
| **StringBuilder** | ~2.4% | write_char, grow_if_necessary, blit_from_string |
| **Map / Hash** | ~1.4% | Hasher::combine_string |

## 観察と patch 候補

### 1. UTF-16 化が高コスト (~3.7%)

HTTP messages は UTF-8 bytes。moonbit `String` は UTF-16。`http::Reader`
が header value を `String` に変換するときに simdutf で utf8→utf16 が
走り、`to_lower` で再びイテレートする。

**潜在的勝ち筋**: header の name/value を `String` でなく `BytesView` /
`Bytes` のまま扱う API があれば simdutf を回避できる。これは
async/http の API 変更レベル。

### 2. `StringView::to_lower` 1.49% + `StringView::iter` 2.15%

ヘッダー名を case-insensitive 比較するために毎リクエストでメソッド名
等を lower 化している。よく見るアプローチ:
- ヘッダー名を **小文字で正規化済み Bytes** で保持
- ハッシュも Bytes-ベース ASCII 専用
- ASCII fast path: 非 ASCII を検出しない限り `to_lower` を per-byte で
  処理 (UTF-16 経由しない)

### 3. `BytesView::find` 2.99%

`\r\n\r\n` の境界探し。`brute_force_find` 0.96% が裏で動く。memchr 系の
最適化された delimiter search に降ろせれば数 % 削れる可能性。

### 4. `Hasher::combine_string` 1.37%

ヘッダー Map のキー hash。コアで `combine_string` を改善すれば波及する。
core PR-02 (hashmap-family grow rehash) のスコープを越えるが、
`Hasher` 自体を高速化するなら別 PR 候補。

### 5. `moonbit_drop_object` 14% + alloc 32% は全パッチ共通の上限

これは moonc 側の refcount elision / move analysis 問題。HTTP リクエス
ト 1 つあたりに作る Bytes / String / List / Map / Coroutine 等が全部
incref/decref を踏む。**全 workload で見える native の天井**。

## 再現

```sh
# 1. server build
cd bench-async
moon build --release --target=native cmd/http_server_benchmark

# 2. RPS 計測 (k6 が要る; v0.50.0 で動作確認)
./_build/native/release/build/cmd/http_server_benchmark/http_server_benchmark.exe &
SERVER=$!
sleep 0.5
VUS=128 DURATION=15s k6 run --quiet cmd/http_server_benchmark/k6_script.js
kill $SERVER

# 3. callgrind プロファイル (slowdown ~20x なので軽負荷で短時間)
valgrind --tool=callgrind --callgrind-out-file=/tmp/http.callgrind \
  ./_build/native/release/build/cmd/http_server_benchmark/http_server_benchmark.exe &
SERVER=$!
sleep 5  # callgrind init
VUS=8 DURATION=3s k6 run --quiet cmd/http_server_benchmark/k6_script.js
kill -INT $SERVER
callgrind_annotate /tmp/http.callgrind | head -30
```

## なぜ wrk でなく k6 か

`pprof-mbt` リポジトリの上位コンテキスト (この調査全体) では再現性と
スクリプト性を重視している。k6 は JS で負荷シナリオが書け、CI 連携・
複数 endpoint 同時実行・段階負荷 (`ramping-vus`) などを 1 ファイルに
書ける。今回の単一エンドポイントでは wrk と差は無いが、将来 POST
/JSON / 認証付き等で複雑になっても k6 の `k6_script.js` を書き換える
だけで済む。
