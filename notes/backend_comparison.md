# 4 バックエンド横断計測: refcount オーバーヘッド検証

`notes/json_parse_findings.md` の結論 — wasm では incref/decref が 30%+ を
占め userland 最適化の効果は小さい — を、**同一 workload を 4 バックエンド
で走らせる** ことで定量的に確かめた。wasm-gc は wasm refcount を一切持た
ないので、もし refcount が真に支配的なら wasm-gc は wasm より大幅に速い
はず。

## 計測条件

- moonbit: 0.1.20260522 (本リポジトリの toolchain)
- moonbitlang/core: そのバージョンの bundle
- ホスト: Linux x86_64 (remote container, Intel)
- profiler:
  - wasm → `.bin/wasmtime-runner` (Cranelift JIT + GuestProfiler, 1ms tick)
  - wasm-gc / js → `node --inspector` Profiler.start (V8 sampling)
  - native → 計測のみ `time` (このホストでは samply 未インストール)
- 全 backend ともサンプリング profile はかけたまま (= overhead 込み)

## 4 workload × 4 backend のウォールタイム

50/30/2000/(各 1) iter のまとめ。中央値 (3 run):

| workload          | wasm   | wasm-gc | js    | native | wasm/native | wasm/wasm-gc |
|-------------------|--------|---------|-------|--------|-------------|--------------|
| `json_parse`      | 619 ms | 188 ms  | 302 ms| 136 ms |  4.6×       |   3.3×       |
| `regex_match`     |  89 ms |  33 ms  | 150 ms|  20 ms |  4.5×       |   2.7×       |
| `sorted_map_merge`| 104 ms |  45 ms  |  63 ms|  27 ms |  3.9×       |   2.3×       |
| `main` (cpu only) | 684 ms | 151 ms  | 567 ms|  62 ms | 11.0×       |   4.5×       |

(native の数値はサンプリング overhead を含まないので比較係数は厳密ではない
が、ホスト内 startup と print を含んだ end-to-end。)

### 観察

1. **wasm-gc が wasm の 2.3 〜 3.3× 速い**。`json_parse` を始め refcount が
   重い workload ほど差が広がる。これは
   [json_parse_findings.md](json_parse_findings.md) の「メモリ管理系が
   ~60%」と整合的: refcount を撤廃するだけで wall time が半分以上消える。

2. **`main` workload (mandel/fib/ackermann) ですら wasm/native が 11×**。
   pure CPU で mem-mgmt 0% にも関わらずこの差。Cranelift JIT vs clang -O2
   の最適化差 + サンプリング overhead が混ざっている。wasmtime-runner は
   epoch_interruption を ON にしているので、JIT 内に periodic check が挿
   入される分のコストもある (TODO: `--no-profile` 経路を追加して切り分け)。

3. **js は wasm-gc より遅い workload と速い workload が混在**。
   - `regex_match`: js 150 / wasm-gc 33 (js が 4.5× 遅い) →
     moonbit の regex エンジンは sorted_map ベースで、enum match と
     immutable map operation が大量に走る。V8 はこのパターン (object
     allocation 多発 + polymorphic call) を JIT で最適化しきれない。
   - `sorted_map_merge`: js 63 / wasm-gc 45 (近い)。
   - `main`: js 567 / wasm-gc 151 (4× 遅い)。再帰深さの問題 (fib(32),
     ackermann(3,10)) で V8 のスタック / 関数呼び出しコストが効く。

4. **`sorted_map_merge` の wasm/wasm-gc 比 (2.3×) は他より小さい**。これ
   は SortedMap が persistent tree で短命オブジェクトが大量に発生する
   workload なので、GC (世代別 V8 GC) が wasm の per-op refcount より
   有利になる典型例。

## refcount コストの内訳 (wasm, json_parse)

`pprof-summary` で見た上位:

| | wasm self | wasm-gc self |
|---|---|---|
| `moonbit.incref` / `decref` | 22.3% | (なし) |
| `moonbit.gc.malloc` / `free` | 10.8% | (GC に統合) |
| `tlsf/insertBlock` / `removeBlock` / `searchBlock` | 10.4% | (なし) |
| `moonbit.make_array_header` / `store_object_meta` / `get_ref_cnt` | 12.1% | 一部残る |
| **mem-mgmt 合計** | **~63%** | **~21% (GC)** |
| `(garbage collector)` | (該当なし) | 21.0% |

つまり wasm の 63% mem-mgmt が wasm-gc では 21% の単一 GC コスト
に置き換わる。差し引き **約 42%** が消えており、それがほぼそのまま
wall time 短縮 (619 → 188 ms, 約 -69%) に効く。サンプリング overhead と
他要因で 42% より大きく見えているのは、refcount を消すと cache 局所性が
上がる ことや、wasmtime のサンプリング overhead 自体が大きいため。

## refcount を消した後に残る hot path (wasm-gc, json_parse)

flat top:

| flat | 関数 |
|---|---|
| 21.0% | `(garbage collector)` |
| 12.8% | `json::ParseContext::parse_object` |
| 10.2% | `post` (V8 internal, GC barrier 含む) |
|  7.8% | `json::ParseContext::lex_value` |
|  7.3% | `json::ParseContext::lex_string` |
|  7.2% | `json::ParseContext::parse_value2` |
|  6.1% | `Map::set_with_hash` |
|  5.1% | `read_char` |
|  5.0% | `lex_skip_whitespace` |
|  4.9% | `scan_json_number` |
|  3.9% | `FixedArray::blit_from_string` (`to_owned()` の memcpy) |

**この時点で初めて真のホットスポットが見える**: parse_value / lex_value
の制御フロー、`Map::set_with_hash` (= 既知の grow + hash 再計算)、
`scan_json_number` (= 既知の二度読み)、`lex_string` (= 既に fast path
あり)。

userland 改善の費用対効果はここから議論できる。例えば `lex_string` の
`to_owned()` (= `blit_from_string` 3.9%) を `String::from_view` の
zero-copy view に置き換えれば、wasm-gc 上で約 4%、wasm 上では (refcount
elision も付随するため) もっと大きく効く可能性がある。

## 結論

- 「core/json 自身の改善で json.parse の wall time を 10% 単位で縮める」
  ことは **wasm では原理的に困難** (refcount の 30% が天井)。
- **wasm-gc 上で計測すると改善余地が初めて見える**。今後の core 最適化
  検証は wasm でなく wasm-gc を main metric にすべき。
- moonc 側の refcount elision / move analysis が入れば、wasm でも
  wasm-gc に近い性能 (= 約 3×) が原理上可能。

## 再現手順

```sh
# wasm
moon build --release --no-strip --target=wasm bench/cmd/json_parse
.bin/wasmtime-runner --interval-us 1000 --out json_parse.pb.gz \
  bench/_build/wasm/release/build/cmd/json_parse/json_parse.wasm

# wasm-gc
moon build --release --no-strip --target=wasm-gc bench/cmd/json_parse
node runners/run-wasm-gc.mjs \
  bench/_build/wasm-gc/release/build/cmd/json_parse/json_parse.wasm \
  json_parse.wasm-gc.cpuprofile 1
node runners/cpuprofile-to-pprof.mjs json_parse.wasm-gc.cpuprofile json_parse.wasm-gc.pb.gz

# js
moon build --release --target=js bench/cmd/json_parse
node runners/run-js.mjs \
  $PWD/bench/_build/js/release/build/cmd/json_parse/json_parse.js \
  json_parse.js.cpuprofile 1
node runners/cpuprofile-to-pprof.mjs json_parse.js.cpuprofile json_parse.js.pb.gz

# native (samply or just time)
moon build --release --target=native bench/cmd/json_parse
time bench/_build/native/release/build/cmd/json_parse/json_parse.exe

# 集計
.bin/pprof-summary json_parse.pb.gz
.bin/pprof-summary json_parse.wasm-gc.pb.gz
.bin/pprof-summary json_parse.js.pb.gz
```
