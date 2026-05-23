# 他のデータ構造でも横断計測

`backend_comparison.md` で見た「wasm は refcount で 2〜3× 遅い」の傾向が
core の他データ構造でも成り立つかを確認するため、`hashmap` / `priority_queue` /
`deque` / `bigint` の bench を追加した。各 workload は対象モジュール 1 つに
だけ依存し、main がそれを呼ぶ。

## 追加 bench

| dir | 概要 |
|---|---|
| `bench/cmd/hashmap_ops` | 10k key insert → hit lookup → miss lookup, 50 iter |
| `bench/cmd/priority_queue_ops` | 20k push (LCG order) → drain via `pop`, 50 iter |
| `bench/cmd/deque_ops` | 50k 交互 `push_front` / `push_back` → drain 両端, 80 iter |
| `bench/cmd/bigint_ops` | `factorial(800)` (= ~1980-bit BigInt), 100 iter |

## ウォールタイム (3 run の中央値)

| workload          | wasm   | wasm-gc | js     | native | wasm/wasm-gc | wasm/native |
|-------------------|-------:|--------:|-------:|-------:|-------------:|------------:|
| hashmap_ops       | 309 ms |  95 ms  | 128 ms |  80 ms |    3.3×      |    3.9×     |
| priority_queue_ops| 306 ms |  80 ms  |  91 ms |  87 ms |    3.8×      |    3.5×     |
| deque_ops         | 184 ms |  76 ms  | 132 ms |  46 ms |    2.4×      |    4.0×     |
| bigint_ops        | 320 ms |  50 ms  |  17 ms |  37 ms |  **6.4×**    |  **8.6×**   |
| (参考) json_parse | 619 ms | 188 ms  | 302 ms | 136 ms |    3.3×      |    4.6×     |
| (参考) regex_match|  89 ms |  33 ms  | 150 ms |  20 ms |    2.7×      |    4.5×     |
| (参考) sorted_map |104 ms |  45 ms  |  63 ms |  27 ms |    2.3×      |    3.9×     |

(native は profile overhead 抜き。startup + print 込み。)

## 観察

### 1. wasm/wasm-gc は **2.3〜6.4×**, 中央値 ~3×

7 workload 中 6 つで wasm-gc が 2〜4× 高速。`backend_comparison.md` で
得た json_parse 3.3× が外れ値ではないことを確認。**refcount 撤廃で約 3×**
が一般的な目安。

### 2. `bigint` は別格 — wasm 6.4×, **js が wasm-gc より 3× 速い**

bigint だけ js (17 ms) が wasm-gc (50 ms) を **3× 上回り native (37 ms) も
超える**。これは V8 の `BigInt` がネイティブで Karatsuba / FFT ベースの
乗算を持つ一方、moonbit は wasm/wasm-gc/native すべて自前の
`grade_school_mul` (`O(n²)`) で動くため。wasm-gc 上で
`grade_school_mul` 自身が flat 69.5% を占めており、アロケーションを
全部消したとしても乗算アルゴリズムを Karatsuba に差し替えない限り
js には届かない。

- 改善候補: `bigint::BigInt::grade__school__mul` を limbs ≥ ~30 で
  Karatsuba に切り替える。factorial(800) なら limbs が ~62 まで伸びるので
  即時効くはず。

### 3. `priority_queue` は `merges` が支配

`meld` (ペアワイズ meld) は wasm 上では mem-mgmt を含んで 14.8% に見え
るが、wasm-gc では再帰の親 `priority_queue::merges` が **51.6%** で
ぶっち切り。pairing heap の再帰ペアリングが重い。push/pop 自体は数 % しか
食わないので、最適化対象は明確に `merges`:

- 改善候補: pairing heap → flat な binary heap (`Array[T]` で sift-up /
  sift-down)。`O(log n)` per op, alloc は amortized constant。pairing heap
  は理論計算量は良いが alloc が多く、moonbit のように alloc が
  refcount/GC を伴う runtime では不利。

### 4. `hashmap` は `Hasher::*` chain が見える

wasm-gc 上の flat:

| % | 関数 |
|---|---|
| 21.8% | (garbage collector) |
| 16.5% | `HashMap::get` |
| 15.7% | `HashMap::grow` |
| 12.5% | `HashMap::set_with_hash` |
|  6.2% | `Hasher::new` |
|  3.1% | `Hash::hash` |
|  2.2% | `Hasher::new_2einner` |

- `HashMap::grow` が 15.7% も食っているのは `new()` で capacity 指定なしのため。
  10k 要素入れる前から `capacity=10000` を渡せば消える。
- `Hasher::new` + `new_2einner` + `Hash::hash` + `Hasher::finalize`
  (wasm では 4.6%) で **1 操作あたり 4 関数呼び出し**。インライン化されて
  いれば 1 関数になるはず。core 側で `@inline` 注釈、または compiler
  側の inlining 強化候補。

### 5. `deque` は内部 index 計算が重い

wasm 上で `Deque::tail_index` 単体で 3.5%、`push_back`/`pop_*` 各々 3〜4%。
deque はリングバッファなので index 計算が `mod capacity` でほぼ全部。
moonbit が `i % cap` をオーバーフロー checked で出している場合、unchecked
arithmetic primitive (`Int::wrap_*`) に降ろせば数 % 縮む。

## 改善余地のまとめ (core 側)

| 構造 | 改善 | 期待効果 (wasm-gc 比) |
|---|---|---|
| bigint | grade_school → Karatsuba (limbs ≥ 30) | factorial(800) で >2× |
| priority_queue | pairing heap → binary heap (Array-backed) | merges 51% 消去で ~2× |
| hashmap | 既定 capacity 拡大 / Hasher inline | grow 15.7% + Hasher 11% 縮小 |
| deque | unchecked mod / branch reduction | tail_index 3% を 1% に |
| json (lex_string) | `to_owned()` → `String::from_view` zero-copy | blit_from_string 3.9% を 0 に |

bigint の Karatsuba と priority_queue の binary heap 化は、moonc を
触らなくても **core/library パッチだけで 2× レベルの改善が得られそう**
な候補。json/regex のように refcount が天井になっている workload と違って、
ここはアルゴリズム選択の問題なので userland の修正が直接ウォールタイムに
効く。

## 再現

```sh
PATH="$HOME/.moon/bin:$PATH"
cd bench
for w in hashmap_ops priority_queue_ops deque_ops bigint_ops; do
  for t in wasm wasm-gc native; do
    moon build --release --no-strip --target=$t cmd/$w
  done
  moon build --release --target=js cmd/$w
done
cd ..
# 計測は backend_comparison.md の手順と同じ
```
