# moonbitlang/core `@json.parse` を pprof で計測した結果

`bench/cmd/json_parse/main.mbt` で core 自身の bench
(`json/parse_bench_test.mbt`) と同じ 1000 要素オブジェクト配列を 50 回パース
する workload を組み、`wasmtime-runner` (wasm) と `samply` (native) で
プロファイルを取った。

## 計測条件

- 入力: 1000 要素の `[{...}, {...}, ...]` 形式 (197 KB)
- 反復: 50 回 (試行間に GC や allocator のウォームアップが収束する程度)
- moonbit: 0.1.20260512
- moonbitlang/core: `main` (snapshot 384ba72)
- ホスト: macOS aarch64

| backend | 50 iter | per iter | サンプル数 |
|---|---|---|---|
| wasm (wasmtime, sampling 1ms) | 498 ms | 9.96 ms | 498 |
| native (samply, sampling 1ms) | 148 ms | 2.96 ms | 135 |

native は wasm の **約 3.4× 速い**。

## ホットスポット (wasm)

`go tool pprof -top -flat` 上位:

| flat | 関数 |
|---|---|
| 18.04% | `moonbit.incref` |
| 12.35% | `moonbit.decref` |
|  6.29% | `moonbit.gc.malloc` |
|  4.54% | `moonbit.gc.free` |
|  4.28% | `tlsf/removeBlock` |
|  4.04% | `tlsf/insertBlock` |
|  3.53% | `string::unsafe_char_at` |
|  3.27% | `moonbit.make_array_header` |
|  2.77% | `moonbit.free` (deref path) |
|  2.77% | `json::ParseContext::lex_string` |
|  2.52% | `Map::set_with_hash` |
|  2.52% | `string::offset_of_nth_char_forward` |
|  2.25% | `moonbit.array_length` |
|  2.02% | `tlsf/searchBlock` |

**メモリ管理系 (incref / decref / malloc / free / TLSF / array_header) の合計が
約 59%**。実際のパース処理は残り 4 割しか走っていない。

## ホットスポット (native)

| flat | 関数 |
|---|---|
| 10.14% | `moonbit_drop_object` |
|  6.76% | `malloc` |
|  5.41% | `free` |
|  5.41% | `string::offset_of_nth_char_forward` |
|  4.73% | `parse_int64_inner` |
|  4.73% | `json::ParseContext::lex_string` |
|  4.73% | `json::ParseContext::read_char` |
|  4.05% | `Map::add_entry_to_tail` |
|  3.38% | `Map::grow` |
|  3.38% | `fold_digits` |

native のメモリ管理系合計は **約 22%**。wasm の 1/3。

## 改善候補

### 1. wasm refcount オーバーヘッド (最大の効果)

- wasm では incref/decref が **30% 以上**を占める。wasm 自体に refcount 命令
  がなく、関数前後で読み書きを発行しているため。
- core 側で対応するなら: パース時に作る中間値 (`Json` enum の variant 内側
  に持つ `String`/`Array`/`Map`) のうち、即座にコンテナへ move される
  ものはレファレンスカウントを elide できる可能性。コンパイラ側の最適化
  (move analysis / escape analysis) が効けばここはほぼゼロにできる。
- 実験: `lex_property_name2` → `Map::set_with_hash` の経路で同じキー文字列が
  作られて捨てられている。`set_with_hash` が `&str` を受け取れる API なら
  short string 用に 0-copy できる。

### 2. UTF-8 char indexing (`offset_of_nth_char_forward`)

- wasm/native 共通で 3〜5% を食う。
- JSON 文法は ASCII 限定なので、`lex_string` 等は **byte 単位** に降ろせる
  はず。`StringView::char_at` 系を `byte_at` に置き換えると `offset_of_nth_…`
  の O(n) 検索が消える。
- 一方で文字列ペイロードは UTF-8 のままで OK。ASCII 範囲だけバイト走査して、
  非 ASCII 出現時にだけ char 境界を計算する分岐構造が定石。

### 3. `StringBuilder::grow_if_necessary` (lex_string flush で 59%)

- 1 文字ずつ flush するため、Builder の容量見積もりが頻繁に外れて grow。
- 改善案:
  - `lex_string` の入口で、`"` までの距離を 1 度走査して `size_hint` を渡す。
  - もしくは `lex_string` 自体を「先頭 `"` から終端 `"` までを 1 度走査して
    エスケープ無しなら `String::from_view` で zero-copy 切り出し、エスケープ
    があったときだけ Builder にフォールバック」の 2-path 構造に。
    多くの JSON 文字列はエスケープを含まないので fast path が支配的になる。

### 4. `Map::set_with_hash` / `Map::grow`

- パース対象の各オブジェクトは 8 キー。grow しないサイズで作られていれば
  insert はほぼ O(1)。
- `parse_object` で `{` を見た瞬間に capacity を見積もる方法はないが、
  `Map::with_capacity(8)` 程度のヒントは入れられる。最初の 1〜2 オブジェクトで
  容量が固まれば、以降のオブジェクトでは grow 回数が減る。

### 5. `parse_int64_inner` / `parse_double`

- native で 4.7%、wasm で 9.5% (cum)。
- 整数 → double の単一パスにできれば早い。例: `123456` を `i64` で読みつつ
  最後に `.` か `e` を見た瞬間に double に切り替える、というのは既にやって
  そう。中身を読まないと判断できないが、`fold_digits` が再度走るのが見えて
  いるので二度読みしている可能性が高い。

## 提案

最大の効果がありそうなのは **(2) bytewise lex** と **(3) StringBuilder の
size_hint / zero-copy fast path** で、core 側の小規模なパッチで対応可能。
**(1) refcount elision** はコンパイラ側の最適化なので moonbit 本体への提案
だが、JSON だけでなくあらゆる workload に効く。

このリポジトリの `bench/cmd/json_parse/` を使えば
パッチ前後の差分が wasmtime-runner ですぐ可視化できる:

```sh
moon build --release --no-strip --target=wasm cmd/json_parse
.bin/wasmtime-runner --interval-us 1000 --out before.pb.gz \
  bench/_build/wasm/release/build/cmd/json_parse/json_parse.wasm
# (patch core, rebuild)
.bin/wasmtime-runner --interval-us 1000 --out after.pb.gz \
  bench/_build/wasm/release/build/cmd/json_parse/json_parse.wasm
go tool pprof -base before.pb.gz after.pb.gz
```
