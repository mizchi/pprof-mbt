# core パッチ実験

`data_structures_comparison.md` で挙げた 2 つの改善候補に実際にパッチを
当てて測定した。結果はミックス: 1 つは予想外の negative result、もう 1 つ
は -20〜24% の改善だが API 契約と衝突する。

## ワークフロー

```sh
# 1. ~/.moon を書き換え可能な場所にコピー
cp -r ~/.moon /tmp/moonbit-patched
chmod -R u+w /tmp/moonbit-patched

# 2. core を編集 → bundle を全 target で再ビルド
cd /tmp/moonbit-patched/lib/core
for t in wasm wasm-gc js native; do
  moon bundle --release --target $t
done

# 3. bench を patched toolchain で再ビルド
cd /home/user/pprof-mbt/bench
PATH=/tmp/moonbit-patched/bin:$PATH \
MOON_TOOLCHAIN_ROOT=/tmp/moonbit-patched \
moon build --release --no-strip --target=wasm cmd/<workload>

# 4. 同じ session 内で baseline と patched を切り替えて測る
```

## 実験 1: PriorityQueue を pairing heap → Array-backed binary heap

**動機**: `merges` (pairing meld の再帰) が wasm-gc 上で flat 51.6%。
教科書的には Array-backed binary heap が constant factor で勝つはず。

**実装**: `priv struct Node` を捨て、`struct PriorityQueue[A] { data : Array[A] }` に。
push/pop は sift-up / sift-down の hole technique (1 level あたり 1 read + 1 write)。

**結果** — **遅くなった**:

| | baseline (pairing) | patched (binary) | delta |
|---|--:|--:|--:|
| wasm | 306 ms | 256 ms | -16%? |
| wasm-gc | 80 ms | 188 ms | **+135%** (退行) |
| js | 91 ms | 256 ms | +180% (退行) |
| native | 87 ms | 108 ms | +24% (退行) |

(wasm が改善したのは mem-mgmt 比率が変わったため。pairing heap は push ごと
に Node を alloc。binary heap は Array 1 つだけ alloc。)

**ホットスポット** (patched wasm-gc):

```
70.2%  sift_down
 9.7%  sift_up
 4.5%  ____moonbit__main
 3.5%  array::Array::resize_buffer
```

**原因分析**:
- 20k push + 20k drain の合計操作数:
  - pairing heap ≈ O(n log n) = 20000 × ~14 ≈ 280k 操作、ただし各 meld は
    Node 3 フィールド書き換えのみで cache-local
  - binary heap = 同じ ~280k iteration だが、各 iteration で
    `Array[A]` index 経由で 3〜5 access。**generic `Array[A]` の bounds
    check + Compare trait dispatch が per-op で重い**
- pairing heap は struct field 直アクセスで dispatch なし
- 結論: **moonbit core の現状ランタイムでは pairing heap の方が constant
  factor で勝つ**。教科書の O 漸近を信じすぎた。

特殊化された `PriorityQueue[Int]` (UninitializedArray ベース) なら binary
heap が勝つかもしれないが、generic 型では負ける。

## 実験 2: Deque の modulo → bitmask

**動機**: `Deque::tail_index` が wasm self 3.5%、push/pop_back/_front も
全部 `% cap` を含む。リングバッファは普通 cap を 2^n に揃えて `& (cap-1)`
で書く。

**実装**:
- `Deque::Deque(capacity?)`, `realloc`, `new_deque` で常に `buf.length()`
  を 2 の冪に揃える (`round_up_capacity` を追加)
- `tail_index` / `push_front` / `push_back` / `pop_front` /
  `unsafe_pop_front` の各 `% cap` を `& (cap - 1)` に
- `tail_index` に `#inline`

**結果** — **クリーンな改善**:

| | baseline | patched (bitmask) | delta |
|---|--:|--:|--:|
| wasm | 246 ms | 225 ms | **-8.5%** |
| wasm-gc | 102 ms | 81 ms | **-20.6%** |
| js | 207 ms | 167 ms | **-19.3%** |
| native | 58 ms | 44 ms | **-24.1%** |

(同じ session で baseline を再計測した数値。`data_structures_comparison.md`
の表とは絶対値が違うが、ratio はクリーン。)

`#inline` 単独 (modulo は `%` のまま) では効果なし: bitmask 化が本体。
ターゲット別の差は **mem-mgmt 比率の高い wasm でやや薄まる** という
これまでと同じパターン。

**ホットスポット** (patched wasm-gc):

```
34.5%  ____moonbit__main
20.6%  Deque::push_back
16.3%  post (V8 internal)
 7.7%  Deque::pop_back
 7.5%  (garbage collector)
 4.3%  Deque::push_front
 2.4%  Deque::realloc
```

`tail_index` が top10 から消えた (wasm-gc では完全インライン化)。wasm
target では `tail_index` がまだ 2.8% 見えており、`#inline` が target に
よって効果が違う模様。

**問題**: 24 個の core test が失敗。

```
test "shrink_to_fit": @deque.Deque([], capacity=10).capacity() expected 10, got 16
test "drain when wrapped around": (wrap 後の要素順序が変わる)
...
```

理由:
1. `capacity?` を round up するので `dv.capacity()` がユーザ指定値より大きい
2. `reserve_capacity` / `shrink_to_fit` は任意 size の buf を作るので、
   その後 mask 演算が誤動作

修正は可能 (reserve_capacity / shrink_to_fit も 2 冪に丸める + test 期待値
更新) だが、**capacity の public API 契約変更**になる。Rust の `VecDeque`
や C++ `std::deque` は内部の buf cap を公開せず実装詳細にしているが、
moonbit `Deque::capacity()` は実装詳細を公開している。

## まとめ

| 実験 | 想定 | 実測 | 結論 |
|---|---|---|---|
| PQ binary heap | merges 51% を消す | wasm-gc -135% (退行) | **却下**。pairing heap が moonbit ランタイムでは constant factor 勝ち |
| deque bitmask | mod の自己 3.5% を 0 に | 全 target -8〜24% | **採用候補だが capacity API 契約変更が要る** |

`deque_bitmask.diff` に実 diff を置いた。100 行程度で大きくない。
upstream に出すなら capacity() の動作を実装詳細扱いに変える RFC が必要。

## 教訓

- 「ホットスポット self% が低くても、call frequency が高ければ
  micro-optimization (`%` → `&`) で 5〜25% 効く」: deque の通り。
- 「漸近計算量が良くても定数係数で負ける」: PQ の通り。**moonbit の
  generic Array[A] 経由のアクセスは struct field 直アクセスより重い**
  ので、教科書アルゴリズムの選択基準が変わる。
- 同じ計測 session 内で baseline と patched を切り替えて比べないと、
  system load の揺らぎで delta が読めない (今回 absolute 値が
  `data_structures_comparison.md` と違うのもそれ)。
