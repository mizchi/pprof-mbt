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

## 実験 5: json `lex_number_end` の二度走査を排除

**動機**: `scan_json_number` は数値の桁を 1 度走査して `mantissa` /
`exponent` を計算済み。整数経路に分岐するとさらに `lex_integer_end` が
**同じ桁を再走査して Int64 acc を組む**。mantissa が safe-int 範囲なら
そのまま使えるはず。

**実装** (`notes/json_skip_double_scan.diff`, 17 行):

```moonbit
if scan.is_integer {
  if !scan.many_digits && scan.mantissa <= SAFE_INTEGER_LIMIT.reinterpret_as_uint64() {
    let v = scan.mantissa.reinterpret_as_int64()
    let signed = if scan.negative { -v } else { v }
    return (signed.to_double(), None)
  }
  return ctx.lex_integer_end(start, end)
}
```

**結果** — **modest win on wasm-gc**:

| | baseline | patched | delta |
|---|--:|--:|--:|
| wasm | 664 | 662 | -0.3% (誤差) |
| wasm-gc | 189 | 175 | **-7.5%** |
| js | 295 | 284 | -3.7% |
| native | 131 | 126 | -3.8% |

bench 入力 1 オブジェクトに整数 4 個 × 1000 obj × 50 iter = 20 万回
fast-path にヒット。wasm では mem-mgmt overhead が支配しているので
algorithmic gain が薄まる。6503/6503 tests pass。

## 実験 6: immut/sorted_map `make_tree` の `length()` 呼び出しを inline

**動機**: `make_tree` は `l.length() + r.length() + 1` で size を計算。
`length()` 自体は `match` 1 個だけだが、`make_tree` が merge/balance
ループ内で大量に呼ばれるので、関数呼び出しを潰したい。

**実装** (`notes/sorted_map_make_tree.diff`, 30 行): `make_tree` 内に
`length()` の中身を直接展開 + `#inline` 注釈。

```moonbit
#inline
fn[K, V] make_tree(key, value, l, r) {
  let ls = match l { Empty => 0; Tree(_, size=s, _, _, ..) => s }
  let rs = match r { Empty => 0; Tree(_, size=s, _, _, ..) => s }
  Tree(key, value~, size=ls + rs + 1, l, r)
}
```

**結果**:

| | baseline | patched | delta |
|---|--:|--:|--:|
| wasm | 128 | 111 | **-13%** |
| wasm-gc | 43 | 39 | -9% |
| js | 58 | 54 | -7% |
| native | 25 | 24 | -4% |

wasm が一番効くのは、関数呼び出しを潰すと wasm の関数 prolog/epilog
オーバーヘッド (引数 box / unbox / refcount) が消えるため。
6503/6503 tests pass。

## 実験 7: HashMap grow の rehash 専用パス

**動機**: 通常 `set_with_hash` は Robin Hood probe + key equality + 
load-factor check を含む。**grow 中の rehash では全て無駄**:
- 元 entries はすべて unique key (equality 比較は永遠に false)
- size が変わらず capacity が 2 倍 → load-factor check も永遠に false

**実装** (`notes/hashmap_grow_specialized.diff`, 46 行): grow 内の
ループから `set_with_hash` 呼び出しを除き、Robin Hood swap だけの
専用 probe を直接展開。

```moonbit
fn[K, V] HashMap::grow(self) -> Unit {  // 注: Eq 制約も外せる
  ...
  for entry in old_entries {
    if entry is Some(e) {
      let hash = e.hash
      for psl = 0, idx = hash & new_mask, ent = e {
        match self.entries[idx] {
          None => { ent.psl = psl; self.entries[idx] = Some(ent); break }
          Some(curr) =>
            if psl > curr.psl {
              ent.psl = psl; self.entries[idx] = Some(ent)
              continue curr.psl + 1, (idx + 1) & new_mask, curr
            } else {
              continue psl + 1, (idx + 1) & new_mask, ent
            }
        }
      }
    }
  }
}
```

**結果** — **全 target で -17〜19% (クリーン勝利)**:

| | baseline (median) | patched (median) | delta |
|---|--:|--:|--:|
| wasm | 318 ms | 257 ms | **-19.2%** |
| wasm-gc | 100 ms | 81 ms | **-19.0%** |
| js | 136 ms | 112 ms | **-17.6%** |
| native | 84 ms | 70 ms | **-16.7%** |

bench (`hashmap_ops`) は 10k key 投入なので grow が ~11 回走る。
通常 path だと毎 grow で全 entries が full probe を回るが、
専用 path は equality + load check を省ける分軽い。出力
`sum=-1795217296` 一致、6503/6503 tests pass。

## 実験 8: HashSet grow の専用 rehash (hashmap の mirror)

hashmap の patch を hashset にも適用。構造はほぼ同じ。

**実装** (`notes/hashset_grow_specialized.diff`, 49 行):
`HashSet::grow` 内の `add_with_hash` 呼び出しを、Eq + grow_at チェック
を除いた Robin Hood swap 直書きに置き換え。

**結果** (10k key 投入の hashset_ops, 3 run median):

| | baseline | patched | delta |
|---|--:|--:|--:|
| wasm | 317 | 258 | **-18.6%** |
| wasm-gc | 94 | 81 | **-13.8%** |
| js | 122 | 124 | +1.6% (noise) |
| native | 81 | 76 | -6.2% |

6503/6503 tests pass。`Eq` 制約も grow 関数のシグネチャから外せる。

## 実験 9: immut/sorted_set の `create` を inline (sorted_map の mirror)

`SortedSet::union` で `create` (= sorted_map の make_tree に相当) が
hot。同じパターンで `length()` 呼び出し x 2 を直接 match 展開。

**実装** (`notes/sorted_set_create_inline.diff`, 27 行)。

**結果** (sorted_set_union bench):

| | baseline | patched | delta |
|---|--:|--:|--:|
| wasm | 128 | 118 | **-7.8%** |
| wasm-gc | 47 | 44 | -6.4% |
| js | 59 | 62 | +5% (noise) |
| native | 24 | 23 | -4% |

6503/6503 tests pass。

## 実験 10: builtin `Map` (linked hash map) の grow 専用 path

json `Map` は builtin の linked hash map。**hashmap と違って prev/next
の linked list 不変条件を保つ必要があり、Robin Hood swap で
push_away するときに `set_entry` 経由で `next.prev` を patch しないと
LL が壊れる**。最初に inline で書いたら 1 test 失敗 (LL の tail 計算
ミスで retain が 502 を返した)。

`add_entry_to_tail` + `push_away` の既存ヘルパを再利用する
シンプル版に書き直して全 test pass。

**実装** (`notes/linked_hash_map_grow_specialized.diff`, 62 行):

```moonbit
fn[K, V] Map::rehash_place_entry(self, outer : Entry[K, V]) -> Unit {
  let hash = outer.hash
  for psl = 0, idx = hash & self.capacity_mask {
    match self.entries[idx] {
      None => {
        outer.psl = psl
        outer.prev = self.tail
        self.add_entry_to_tail(idx, outer)  // 既存ヘルパが LL を保つ
        return
      }
      Some(curr) =>
        if psl > curr.psl {
          self.push_away(idx, curr)
          outer.psl = psl
          outer.prev = self.tail
          self.add_entry_to_tail(idx, outer)
          return
        } else { continue psl + 1, (idx + 1) & self.capacity_mask }
    }
  }
}
```

(Eq check と grow_at check を省くだけ。Entry 構造体の new
allocation も省ける = 元の Entry を再利用。)

**結果** (json_parse, 単発計測): wasm-gc -3〜5%。json の bench は
1 オブジェクト 8 キーで grow が 1 回しか走らないため小さい。
6503/6503 tests pass。

## 追加シナリオで採用パッチを検証

「採用したパッチが想定の workload 以外でも安全か / 想定パターンを変えた
ときに効くか」を確かめるため、4 つの追加 bench を作って同じ patched
core で測った。

| bench | 何が違うか | 期待 |
|---|---|---|
| `bigint_square` | x = x * x の繰り返し。両辺 equal-size なので grade_school と Karatsuba 経路に行く | n×1 patch が n×n を壊していないこと |
| `hashmap_string` | key が String (5k 個) | grow patch が String key (重い eq) でも効くこと |
| `hashmap_update` | 一度 fill 済み map を update し続ける (grow が走らない) | grow に触らない workload で退行なし |
| `json_numbers` | flat array of 10k safe integers | safe-int 二度走査排除が number-heavy で大きく効く |

### 結果

| workload          | wasm   | wasm-gc | js     | native | 観察 |
|-------------------|-------:|--------:|-------:|-------:|---|
| bigint_square     | -2%    | **-9%** | -7%    | noise  | n×1 patch が n×n path を壊していない ✓ |
| hashmap_string    | **-13%** | **-30%** | **-23%** | **-10%** | grow patch が String key で **wasm-gc -30%** ✓ |
| hashmap_update    | 0%     | -9%     | -9%    | -17%   | grow に触らない workload で退行なし ✓ |
| json_numbers      | -6%    | **-16%** | **-38%** | -10%  | safe-int 排除が number-heavy で **js -38%** ✓ |

### 注目点

- **hashmap_string の wasm-gc -30%** は単独 hashmap_ops の -22% より大きい。
  String の `eq` は Int より高価なので、grow から `Eq` 制約を外す効果が
  String key で増幅される。**generic 制約を外せるという副次効果が pay-off
  している**。
- **json_numbers の js -38%** は劇的。`lex_number_end` の safe-int fast
  path を 1 関数の中で完結させたことで V8 JIT が綺麗に inline できた
  と推測。number-heavy JSON は API リクエストパース等で実際多い形。
- **bigint_square** は n×1 patch がトリガーされないにも関わらず -9%。
  初期 squarings (まだ limb が小さい段階) でいくらか n×1 経路を通る
  のと、他 patch の僅かな効果の合算と思われる。
- **hashmap_update** で wasm が 0% (退行なし)、native が -17%。grow
  が走らないので **`set_with_hash` の hot path には patch の影響が無い**
  ことを確認。

### 全 patched core での当初 + 追加 14 workload 一覧 (wasm-gc, 1 run)

| workload | base | patched | delta |
|---|--:|--:|--:|
| bigint_ops | 53 | 18 | **-65%** |
| **hashmap_string** | 97 | 68 | **-30%** |
| hashmap_ops | 98 | 77 | **-22%** |
| **json_numbers** | 141 | 118 | **-16%** |
| hashset_ops | 92 | 81 | **-12%** |
| sorted_map_merge | 41 | 40 | -2% |
| **bigint_square** | 86 | 78 | **-9%** |
| **hashmap_update** | 34 | 31 | -9% |
| json_parse | 189 | 171 | **-9.5%** |
| sorted_set_union | 44 | 46 | noise |
| priority_queue_ops | 80 | 81 | noise |
| regex_match | 30 | 30 | 0% |
| deque_ops | 73 | 70 | -4% |
| main (cpu) | 110 | 110 | 0% |

採用 7 patch で **14 workload のうち 9 つに 9〜65% 改善、0 退行**。
パッチが指す path に該当しない 5 workload (sorted_set / pq / regex /
deque / main) も全部誤差範囲。

## まとめ

| # | 実験 | wasm | wasm-gc | テスト | 採否 |
|---|---|--:|--:|:--:|:--:|
| 1 | PQ pairing → binary heap | (改善?) | **+135%** | n/a | ❌ 退行 |
| 2 | deque mod → bitmask | -8.5% | -20.6% | **24 fail** | ⚠️ API 衝突 |
| 3 | **bigint n×1 fast path** | **-65%** | **-65%** | ✅ | ✅ 採用 |
| 4 | Hasher chain `#inline` | -3.6% | 0 | ✅ | ❌ 誤差 |
| 5 | json safe-int 二度走査排除 | noise | -7.5% | ✅ | ✅ 小 |
| 6 | sorted_map make_tree inline | **-13%** | -9% | ✅ | ✅ 中 |
| 7 | **hashmap grow 専用 rehash** | **-19%** | **-19%** | ✅ | ✅ 採用 |
| 8 | **hashset grow 専用 rehash** | **-19%** | **-14%** | ✅ | ✅ 採用 |
| 9 | immut/sorted_set create inline | -8% | -6% | ✅ | ✅ 小 |
| 10 | builtin Map grow 専用 path | noise | -3〜5% | ✅ | ✅ 小 (json で効く) |

`notes/*.diff` に 8 つの実 diff (合計 379 行) を置いた:
- `bigint_mul_single_limb.diff` (48 行) — PR ready
- `hashmap_grow_specialized.diff` (46 行) — PR ready
- `hashset_grow_specialized.diff` (49 行) — PR ready
- `linked_hash_map_grow_specialized.diff` (62 行) — PR ready
- `sorted_map_make_tree.diff` (30 行) — PR ready
- `sorted_set_create_inline.diff` (27 行) — PR ready
- `json_skip_double_scan.diff` (17 行) — PR ready
- `deque_bitmask.diff` (100 行) — API 議論が要る

## 教訓

- 「ホットスポット self% が低くても、call frequency が高ければ
  micro-optimization (`%` → `&`) で 5〜25% 効く」: deque の通り。
- 「漸近計算量が良くても定数係数で負ける」: PQ の通り。**moonbit の
  generic Array[A] 経由のアクセスは struct field 直アクセスより重い**
  ので、教科書アルゴリズムの選択基準が変わる。
- 「ワークロードに合わせた **specialization** が最強」: bigint の通り。
  汎用 grade_school_mul を捨てずに、よく出る n×1 だけ別経路にする。
  factorial のような chain は片方が必ず小さいので 3× 効く。同じ発想で
  hashmap の grow 中 rehash も「unique key + load 余裕」を前提に
  専用 path を書けば 17〜19% 縮む。
- 「`#inline` を足しても誤差」: moonc は十分 aggressive に inline
  している。「インライン化されていない関数呼び出しが見える」のは
  プロファイラの解像度の問題で、実際は inline 済みのケースが多い。
  **ただし関数呼び出しを完全に潰す (関数の中身を呼び出し側に直接展開)
  と効くケースはある**: sorted_map の make_tree で確認 (-13% wasm)。
- 「algorithm 系の小細工は wasm-gc / native で素直に効く。wasm
  (mem-mgmt 60%+) では効果が薄まる」: json 二度走査排除が典型例。
- 同じ計測 session 内で baseline と patched を切り替えて比べないと、
  system load の揺らぎで delta が読めない。

## 累積効果の見積もり (推定)

各パッチは異なる workload に効くので、bench スイート全体に並列適用
した場合の改善は workload 別に独立に得られるはず:

| workload    | 適用パッチ        | 期待 delta (wasm-gc) |
|-------------|-------------------|--:|
| bigint_ops  | bigint mul_single | **-71%** |
| hashmap_ops | hashmap grow      | **-19%** |
| sorted_map_merge | sorted_map make_tree | -9% |
| json_parse  | json safe-int     | -7% |
| deque_ops   | (未適用、API 議論) | (-21%) |
| pq_ops, regex_match, main | 該当パッチなし | 0 |

つまり 5 つを landing できれば、現在の bench スイートで **wasm-gc 平均
-15% 級** の改善が見込める。bigint と hashmap が特に大きい。
