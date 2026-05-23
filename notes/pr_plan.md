# 出す PR の確定計画

`moonbitlang/core` の既存 PR/Issue を一通り確認した結果と、ここで持っている
8 つのパッチをどう出すかの最終整理。

## 既存 PR との重複チェック

| 我々のパッチ | 既存 PR/Issue | 重複? |
|---|---|---|
| bigint n×1 fast path | #2736 (closed: grade_school zero-normalize), #1773/#3018 (operator traits) | **重複なし**。Karatsuba/grade_school について議論はあるが n×1 specialization は誰もやってない |
| hashmap grow 専用 rehash | #1349 (merged: pass stored hash on grow), #3164 (merged: defer grow to insertion only), #3330 (merged: from_array no mid-build rehash) | **次の段階**。#1349 は hash 再計算を消した、#3164 は update 時に grow しない。**probe ループから Eq/load チェックを消す** patch は未出 |
| hashset grow 専用 rehash | #3334 (closed: from_array capacity), #1477 (merged: capacity~), #2099 (merged: fix) | **重複なし**。grow ループの specialize は未出 |
| linked_hash_map (Map) grow 専用 | #2533 (merged: align builtin Map with HashMap), #1984 (merged: optimize LL structure) | **重複なし** |
| sorted_map make_tree inline | #3319 (merged: O(n+m) merge), #3320 (merged: physical_equal fast path), 他 sorted_map 系 PR は doc/refactor | **重複なし**。make_tree helper を触る PR は無し |
| sorted_set create inline | #3311 (closed), #3333 (open: split/join recursion), #3324 (open: O(n+m) co-iteration), #3315 (open: difference/inter) | sorted_set は **作業中**。`create` helper の inline は未出だが、open PR 群とのマージ衝突に注意 |
| json safe-int 二度走査排除 | #3593 (merged May 15: fast paths for safe int / Clinger), #3594 (merged May 15: cleanup), #3612 (merged May 22: strconv StringView), #3517 (merged: clamp huge exponent) | **直後の小改善**。#3593 で fast path 入り、`scan_json_number` の `mantissa` を `lex_integer_end` で使い回す改良はまだ |
| deque mod → bitmask | #3458 (closed: drop separator.to_owned), #3331 (closed: avoid allocations), #3263 (merged: regression test for wrap-around), #2137 (open: API consistency) | **重複なし**。但し capacity API 契約変更を要する |

要点:
- **7 つは clean novel**。
- `deque bitmask` は 1 件だけ「API 議論先行」。
- sorted_set は active development 中なので **rebase 注意**。
- linked_hash_map / hashmap / hashset の 3 つは **同じアイデアの parallel application** — 1 PR でまとめるか 3 つに分けるかは方針次第。

## 提出する PR (確定)

優先度順:

### Tier 1 (即出す, big win, clean code)

| # | 内容 | 効果 | 行数 | 状態 |
|---|---|---|---:|---|
| **PR-1** | `bigint`: n×1 grade-school fast path | factorial 3.4× | 48 | ready |
| **PR-2** | `hashmap/hashset/builtin Map`: grow 専用 rehash (3 in 1) | hashmap/hashset workload -17〜22%, json_parse Map -9.5% | 46+49+62 = 157 | ready |

PR-2 は **1 PR にまとめる**: 「`perf(hashmap,hashset,builtin): skip
Eq/load-factor on grow rehash`」。同じアイデア (rehash 中は unique key
+ load 余裕) を 3 つの並行構造に適用するので 1 PR が説明として一貫する。

### Tier 2 (出す, modest win)

| # | 内容 | 効果 | 行数 | 状態 |
|---|---|---|---:|---|
| **PR-3** | `json/lex_number_end`: skip re-scan for safe integers | wasm-gc -7.5%, js -38% (number-heavy) | 17 | ready, but rebase to current #3594 main |
| **PR-4** | `immut/sorted_map/sorted_set`: inline `length()` in make_tree/create | sorted_map -11% wasm, sorted_set -8% wasm | 30+27 = 57 | ready, **sorted_set side は open PR 群と rebase 確認** |

PR-4 も 1 つにまとめてよい (`perf(immut/sorted_map,sorted_set): inline
size extraction in tree builder`)。

### Tier 3 (議論先行, defer)

| # | 内容 | 効果 | 行数 | 状態 |
|---|---|---|---:|---|
| **(issue)** | `deque`: pow-of-2 capacity + bitmask indexing | -8〜24% | 100 | **Issue として議論先行**。`Deque::capacity()` の public 契約を「ユーザ指定値をそのまま返す」から「実装が round up することがある」に変える要 RFC |

## 順序と段取り

1. `pprof-mbt` リポジトリのスクリプト経由で各 patch を独立に再現可能に
   する (1 patch = 1 ブランチ = 1 PR で moonbitlang/core fork へ)。
2. **PR-1 を最初に出す** (= bigint, 一番効果が分かりやすい、ベンチが
   既に core 側 bench にもある)。これでレビュアの反応を見る。
3. PR-1 が merge 寸前まで来たら **PR-2 (hashmap-family)** を出す。
   PR-1 とは独立。
4. PR-3 (json) と PR-4 (sorted) は Tier 1 のレビューが終わってから。
   sorted_set は open PR 群との rebase が要るので最後。
5. deque は別途 Issue で議論を起こす。capacity API 変更の合意が取れる
   なら follow-up PR。

## 各 PR の中身に必ず含める

`CONTRIBUTING.md` より:

- Test: 我々のパッチは全部 6503/6503 pass している。
- `moon fmt` / `moon check` / `moon test` / `moon bundle` / `moon info`
  を CI で走らせる前に手元で確認。
- 公開 API を変えるものは無いので、新規テストは必須ではないが、bigint
  については `bigint_pow` / 大きな factorial の既存テストでカバー済み。
- **Bench 結果を PR description に貼る** (wasm / wasm-gc / native の表)。
  pprof-mbt の `bench/cmd/*` がそのまま再現スクリプトになる。
- `data_structures_comparison.md` や `patch_experiments.md` への
  リンクを description に置いて根拠を示す (pprof-mbt は public repo なので URL で参照可能)。

## 不採用パッチ

- **PriorityQueue pairing → binary heap**: wasm-gc で +135% 退行。
  記録のみ残す。upstream に出さない。
- **Hasher chain #inline**: 効果なし (-0〜7% noise)。記録のみ。
