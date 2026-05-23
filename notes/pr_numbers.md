# PR ごとの clean ベンチ数値

`bench-runner --no-profile` (= GuestProfiler / V8 inspector を切った
wall time) で 3 run の中央値を取った。各 PR は独立に
`patched-toolchain init` → `apply <diff>` → `rebundle` した上で計測
しているので、他 patch の影響は混ざっていない。

Linux x86_64, moonbit 0.1.20260522。

## PR-1: bigint n×1 fast path (`bigint_mul_single_limb.diff`, 48 行)

`BigInt::mul` で他方が 1 limb の場合に専用の `mul_single_limb` 経路を
追加。factorial 系の連鎖乗算 (`acc * i`) で常に発火する。

| workload | wasm base | wasm patched | Δ | wasm-gc base | wasm-gc patched | Δ | js base | js patched | Δ | native base | native patched | Δ |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| bigint_ops | 255.1 | 80.2 | **-68.6%** | 93.5 | 25.2 | **-73.0%** | 21.2 | 22.3 | +5.2% | 48.9 | 20.0 | **-59.1%** |
| bigint_square | 257.1 | 271.9 | +5.7% | 84.8 | 78.2 | -7.8% | 13.2 | 12.9 | -2.3% | 57.0 | 58.0 | +1.8% |

- `bigint_ops` (factorial 800): **3× 高速化** on wasm / wasm-gc / native。
- `bigint_square` (n×n 連続二乗): n×n 経路を破壊していない確認。誤差範囲。
- `js` はそもそも V8 native BigInt を使うので変化なし。

## PR-2: hashmap-family grow 専用 rehash (3 in 1, 157 行)

- `hashmap_grow_specialized.diff` (46 行)
- `hashset_grow_specialized.diff` (49 行)
- `linked_hash_map_grow_specialized.diff` (62 行)

3 つの並行構造で同じアイデアを適用: grow 中の rehash は全 entry が
unique key + 新テーブルが 50% load なので、`set_with_hash` の Eq check
と load-factor check を捨てた専用 Robin-Hood swap で十分。

| workload | wasm base | wasm patched | Δ | wasm-gc base | wasm-gc patched | Δ | js base | js patched | Δ | native base | native patched | Δ |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| hashmap_ops | 276.1 | 215.8 | **-21.9%** | 130.2 | 98.9 | **-24.0%** | 162.3 | 134.3 | **-17.3%** | 96.1 | 81.1 | **-15.6%** |
| hashmap_string | 171.3 | 149.5 | -12.7% | 70.5 | 65.2 | -7.5% | 94.8 | 90.5 | -4.5% | 55.6 | 51.7 | -7.0% |
| hashmap_update | 68.7 | 71.8 | +4.5% | 32.5 | 30.7 | -5.5% | 49.3 | 49.0 | -0.6% | 18.9 | 18.8 | -0.8% |
| hashset_ops | 279.7 | 222.3 | **-20.5%** | 117.5 | 97.1 | **-17.4%** | 155.4 | 134.0 | **-13.8%** | 94.1 | 84.8 | -9.9% |
| json_numbers | 179.2 | 167.2 | -6.7% | 133.9 | 124.8 | -6.8% | 266.1 | 247.1 | -7.1% | 54.8 | 57.6 | +5.1% |
| json_parse | 569.3 | 552.5 | -3.0% | 239.7 | 224.9 | -6.2% | 347.5 | 347.2 | -0.1% | 168.2 | 157.9 | -6.1% |

- `hashmap_ops` (10k Int key 投入): 全 backend で **-15〜-24%**。
- `hashset_ops` (10k Int key 投入): 全 backend で **-10〜-21%**。
- `hashmap_update` (capacity pre-sized, grow 起きない): 退行なし
  (`set_with_hash` 通常 path は触っていない確認)。
- `hashmap_string` (String key): String の Eq は重い割に grow 回数が
  Int key より少ない (5k 投入) ため、相対効果は Int 版より小さめ。
- `json_*` は builtin `Map` 経由で恩恵 (1 オブジェクト 8 キー × 1k obj)。

## PR-3: json `lex_number_end` の safe-int 二度走査排除 (17 行)

`scan_json_number` が既に digit を 1 度走査して `mantissa` を組んで
いる。整数経路で `lex_integer_end` に行くとそこからまた digit を
走査する。**安全な整数 (mantissa ≤ 2^53−1, not many_digits)** は
`scan.mantissa` をそのまま使えば良い。

| workload | wasm base | wasm patched | Δ | wasm-gc base | wasm-gc patched | Δ | js base | js patched | Δ | native base | native patched | Δ |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| json_numbers | 167.4 | 158.2 | -5.5% | 129.9 | 117.6 | **-9.5%** | 247.6 | 180.8 | **-27.0%** | 54.5 | 55.6 | +2.0% |
| json_parse | 578.8 | 566.0 | -2.2% | 222.7 | 223.8 | +0.5% | 341.8 | 336.8 | -1.5% | 161.3 | 158.6 | -1.7% |

- `json_numbers` (10k flat ints array): **js -27%, wasm-gc -9.5%**。
  V8 が fast-path を綺麗に inline できた様子。
- `json_parse` (mixed object array): 1 obj に整数 4 + double 2 + string
  8 なので fast-path の比率が低く、wins が薄まる。

## PR-4: immut sorted_{map,set} tree-builder inline (57 行)

- `sorted_map_make_tree.diff` (30 行)
- `sorted_set_create_inline.diff` (27 行)

`make_tree` / `create` ヘルパが size 計算で `length()` を 2 回呼んで
いる。`length()` 自体は match 1 個だが、merge/balance/union ループで
大量に呼ばれるので関数呼び出しを直接展開。`#inline` 注釈も付与。

| workload | wasm base | wasm patched | Δ | wasm-gc base | wasm-gc patched | Δ | js base | js patched | Δ | native base | native patched | Δ |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| sorted_map_merge | 111.9 | 102.0 | -8.9% | 49.2 | 50.5 | +2.6% | 69.0 | 76.3 | +10.6% | 32.5 | 31.0 | -4.4% |
| sorted_set_union | 120.2 | 111.7 | -7.1% | 54.8 | 54.8 | +0.0% | 74.2 | 77.0 | +3.8% | 30.6 | 29.2 | -4.7% |

- wasm で **-7〜-9%**, native で -4〜-5%。
- wasm-gc / js は誤差範囲。`length()` の関数呼び出しコストが
  V8 では既に inline されているため。
- wasm (非 GC, refcount runtime) では関数呼び出し prolog/epilog の
  refcount overhead が消える分が効く。

## 計測再現コマンド

各 PR を独立に再現:

```sh
.bin/patched-toolchain init
.bin/patched-toolchain apply notes/bigint_mul_single_limb.diff
.bin/patched-toolchain rebundle
.bin/bench-runner --workloads bigint_ops,bigint_square --runs 3
```

`--no-profile` のおかげで wasm の数値は profile-on 時より honest
(profile が ~70% inflate していたのを確認済み: profile on 240ms →
off 80ms というケースもあった)。

## 不採用 (記録のみ)

- **PQ pairing→binary heap**: wasm-gc +135% 退行 → 却下
- **Hasher chain #inline**: -0〜-7% (誤差) → moonc が既に inline 済み
- **deque mod→bitmask**: 全 target -8〜-24% だが `Deque::capacity()`
  public 契約を変更する必要があるため Issue 先行
