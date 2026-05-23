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

## 実験 3: BigInt の n×1 fast path

**動機**: factorial(800) は 798 回の乗算すべてが (acc × i) で
**片方が 1 limb**。`grade_school_mul` の nested loop は `j < other_len`
チェック + carry 伝搬ループを含むが、other_len = 1 のときは特化できる。
Karatsuba は片方が 1 limb の場合は無効 (Karatsuba は n×n 用)。

**実装** (`notes/bigint_mul_single_limb.diff`, 48 行):

```moonbit
// bigint_nonjs.mbt
fn BigInt::mul_single_limb(self : BigInt, x : UInt) -> BigInt {
  let n = self.len
  let limbs = FixedArray::make(n + 1, 0U)
  let xq = x.to_uint64()
  let mut carry = 0UL
  for i in 0..<n {
    let product = self.limbs[i].to_uint64() * xq + carry
    limbs[i] = (product & radix_mask).to_uint()
    carry = product >> radix_bit_len
  }
  let len = if carry == 0UL { n } else { limbs[n] = carry.to_uint(); n + 1 }
  { limbs, sign: Positive, len }
}

pub impl Mul for BigInt with mul(self, other) {
  if self.is_zero() || other.is_zero() { return zero }
  let ret = if other.len == 1 {
    self.mul_single_limb(other.limbs[0])
  } else if self.len == 1 {
    other.mul_single_limb(self.limbs[0])
  } else if self.len < karatsuba_threshold || other.len < karatsuba_threshold {
    self.grade_school_mul(other)
  } else {
    self.karatsuba_mul(other)
  }
  ...
}
```

**結果** — **クリーンな大勝利**:

| | baseline | patched | delta |
|---|--:|--:|--:|
| wasm | 411 ms | 139 ms | **-66% (2.96×)** |
| wasm-gc | 88 ms | 26 ms | **-71% (3.4×)** |
| js | 21 ms | 20 ms | -3% (誤差 — JS は元から native BigInt) |
| native | 61 ms | 18 ms | **-70% (3.4×)** |

**6503/6503 tests pass**。core 全体のテストスイートに退行なし。
`bitlen_sum=656800` も baseline と一致。

js が動かないのは moonbit JS backend が V8 BigInt にトランスパイル
しているため (`bigint_js.mbt` 経由)。`bigint_nonjs.mbt` は wasm/wasm-gc/
native だけが使う。

## 実験 4: Hasher chain に #inline

**動機**: hashmap_ops の wasm-gc profile で
`Hasher::new` 6.2%, `Hasher::new_2einner` 3.1%, `Hash::hash` 3.1%,
`Hasher::avalanche` 2.2% など、Hasher pipeline の関数呼び出しが
合計 11%+ を占めていた。

**実装**: `Hasher::new` / `combine_int` / `combine_uint` / `consume4` /
`consume1` / `avalanche` / `finalize` / `rotl` の 8 関数に `#inline`。

**結果** — **誤差レベル**:

| | baseline | patched | delta |
|---|--:|--:|--:|
| wasm | 388 | 374 | -3.6% |
| wasm-gc | 127 | 127 | 0% |
| js | 168 | 165 | -2% |
| native | 105 | 98 | -7% |

moonc は既に十分 inline している模様で `#inline` は no-op に近い。
self-time % が見えていても、それは "プロファイラがその symbol で
止まっただけ" でコンパイラがインライン後の coalesced 命令を
カウントしていただけ、というケース。

## まとめ

| 実験 | 想定 | 実測 | 結論 |
|---|---|---|---|
| PQ binary heap | merges 51% を消す | wasm-gc +135% (退行) | **却下**。pairing heap が moonbit ランタイムでは constant factor 勝ち |
| deque bitmask | mod の自己 3.5% を 0 に | 全 target -8〜24% | **採用候補だが capacity API 契約変更要** |
| **bigint n×1 fast path** | factorial の主乗算を特化 | **全 (非-js) target で 3.4×** | **採用**。テスト全 pass、48 行の diff |
| Hasher #inline | 11% の関数呼び出しを潰す | -0〜7% (誤差) | **無効**。moonc が既に inline 済み |

`notes/bigint_mul_single_limb.diff` と `notes/deque_bitmask.diff` に
実 diff を置いた。bigint のはそのまま upstream PR にできる規模。

## 教訓

- 「ホットスポット self% が低くても、call frequency が高ければ
  micro-optimization (`%` → `&`) で 5〜25% 効く」: deque の通り。
- 「漸近計算量が良くても定数係数で負ける」: PQ の通り。**moonbit の
  generic Array[A] 経由のアクセスは struct field 直アクセスより重い**
  ので、教科書アルゴリズムの選択基準が変わる。
- 「ワークロードに合わせた **specialization** が最強」: bigint の通り。
  汎用 grade_school_mul を捨てずに、よく出る n×1 だけ別経路にする。
  factorial のような chain は片方が必ず小さいので 3× 効く。
- 「`#inline` を足しても誤差」: moonc は十分 aggressive に inline
  している。「インライン化されていない関数呼び出しが見える」のは
  プロファイラの解像度の問題で、実際は inline 済みのケースが多い。
- 同じ計測 session 内で baseline と patched を切り替えて比べないと、
  system load の揺らぎで delta が読めない。
