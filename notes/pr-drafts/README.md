# moonbitlang/core 向け PR ドラフト

`pprof-mbt` の調査から確定した 4 つの upstream PR の素材を 1 PR =
1 ディレクトリにまとめたもの。

## 構成

各ディレクトリは PR-ready な単位:

```
notes/pr-drafts/<NN-name>/
├── title.txt              # PR title (1 行)
├── body.md                # PR description (markdown)
├── patch.diff             # raw unified diff (git apply 用)
└── 0001-<name>.patch      # git format-patch (git am 用)
```

## 一覧

| # | ブランチ名 / Issue | 主な効果 | 行数 | 種類 |
|---|---|---|--:|---|
| 01 | `pr-bigint-mul-single-limb` | factorial 3× 高速化 | +31/-1 | PR |
| 02 | `pr-hashmap-grow-rehash` | hashmap/hashset/builtin Map grow を -15〜-24% | +93/-15 | PR |
| 03 | `pr-json-safe-int-reuse` | json_numbers js -27% / wasm-gc -9.5% | +8/-0 | PR |
| 04 | `pr-sorted-tree-builder-inline` | immut sorted_{map,set} wasm -7〜-9% | +25/-3 | PR |
| 05 | `issue-deque-pow2` | deque mod→bitmask、API 契約議論 | ~100 (参考) | Issue 先行 |

## 出し方 (前提: moonbitlang/core を fork してある)

```sh
# 1. fork を clone
git clone git@github.com:<your-fork>/core.git
cd core
git remote add upstream https://github.com/moonbitlang/core.git
git fetch upstream

# 2. 1 PR ぶんのブランチを切って patch を当てる
git checkout -b pr-bigint-mul-single-limb upstream/main
git am < /path/to/pprof-mbt/notes/pr-drafts/01-bigint-mul-single-limb/0001-bigint-mul-single-limb.patch
# (もしくは)
git apply /path/to/pprof-mbt/notes/pr-drafts/01-bigint-mul-single-limb/patch.diff
git -c user.email=you@example.com -c user.name="You" commit -am "$(cat /path/to/pprof-mbt/notes/pr-drafts/01-bigint-mul-single-limb/title.txt)"

# 3. テストを再確認
moon test --target wasm
moon test --target wasm-gc
moon test --target js
moon test --target native

# 4. fmt
moon fmt

# 5. push & PR
git push -u origin pr-bigint-mul-single-limb
gh pr create \
  --repo moonbitlang/core \
  --title "$(cat /path/to/pprof-mbt/notes/pr-drafts/01-bigint-mul-single-limb/title.txt)" \
  --body-file /path/to/pprof-mbt/notes/pr-drafts/01-bigint-mul-single-limb/body.md
```

他 3 つも同じ要領。

## 提出順 (推奨)

1. **PR-01 bigint** を最初に出してレビュアの反応を見る (一番効果が大きく、変更が小さい)。
2. **PR-02 hashmap-family** を出す (3 並行構造の同種改善)。
3. PR-01/02 が merge に近づいたら **PR-03 json** と **PR-04 sorted**。
4. PR-04 は sorted_set の open PR 群 (#3324, #3333, #3315) と rebase
   が衝突する可能性 — 該当 PR の merge を待つ or 先方に rebase 依頼。
5. **Issue-05 deque** は別途、PR ではなく **Issue として議論先行**で。
   `Deque::capacity()` の public 契約変更が要るので合意を取ってから patch を出す。

## 元の調査ログ

- [data_structures_comparison.md](../data_structures_comparison.md)
- [patch_experiments.md](../patch_experiments.md)
- [pr_numbers.md](../pr_numbers.md)
- [pr_plan.md](../pr_plan.md)
