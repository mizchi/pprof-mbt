# PR 提出ステータス

upstream main HEAD `2fb72935` (`Document from_str as default parser`) に対し、
全 4 PR で以下を確認済:

| # | ブランチ | apply | `moon fmt` | `moon info` (.mbti 変化) | `moon test` (wasm / wasm-gc / js / native) |
|---|---|:--:|:--:|:--:|:--:|
| 01 | `pr-bigint-mul-single-limb` | ✅ clean | ✅ no-op | ✅ なし | ✅ 6500 / 6500 / 6459 / 6411 |
| 02 | `pr-hashmap-grow-rehash` | ✅ clean | ✅ no-op (※) | ✅ なし | ✅ 6500 / 6500 / 6459 / 6411 (`core/set` fold-in 込み) |
| 03 | `pr-json-safe-int-reuse` | ✅ clean | ✅ no-op (※) | ✅ なし | ✅ 6500 / 6500 / 6459 / 6411 |
| 04 | `pr-sorted-tree-builder-inline` | ✅ clean | ✅ no-op | ✅ なし | ✅ 6500 / 6500 / 6459 / 6411 |

(※) 初回作成時は `moon fmt` が cosmetic な改行 (multi-line signature の
1 行化、長い `&&` 式の改行) を入れた。patch を amend して取り込み済。

## Issue

| # | 名前 | 状態 |
|---|---|---|
| 05 | `issue-deque-pow2` | RFC として議論を起こす。`Deque::capacity()` の API 契約変更が要るので合意先行 |

## upstream の参照 base

- HEAD: `2fb72935` (`Document from_str as default parser`)
- bundled (`~/.moon`) snapshot: `384ba726` — upstream は docs 1 commit のみ進んでいる
- ソース差分は string/README.mbt.md のみ → 4 patch のいずれも本流に当てて変化なし

## 提出フロー (推奨)

1. `notes/pr-drafts/verify.sh all` を fork checkout で 1 度走らせる
2. README.md の "提出順 (推奨)" に沿って PR-01 → PR-02 → PR-03 → PR-04
3. PR-04 は sorted_set の open PR (#3324 / #3333 / #3315) と rebase 確認
4. Issue-05 は PR 群が落ち着いてから別途
