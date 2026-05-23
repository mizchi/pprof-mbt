# moonbitlang/async 向け PR ドラフト

`bench-async/` で取った callgrind プロファイルから出た patch を
`moonbitlang/async` upstream に出す用の素材。`notes/pr-drafts/` (core 向け)
と同じ形式。

## 一覧

| # | ブランチ名 / Issue | 主な効果 | 行数 |
|---|---|---|--:|
| 01 | `pr-event-loop-empty-timer` | pause_loop -35.6%, cond_var_signal -22.2% (空 timer 時の SortedSet iter を回避) | +14/-7 |
| 02 | `pr-gzip-crc32-index-loop` | gzip_roundtrip -12.8% (`for byte in chunk` の iter overhead を除去) | +13/-5 |
| 03 | `pr-wasm-gc-shim` | wasm/wasm-gc 用の scheduler-only `run_async_main` を追加 (pure-coroutine ワークロードが wasm-gc で動く) | +20/-0 |

詳細な調査ログは `notes/async_investigation.md` 参照。

## 出し方

```sh
# 1. moonbitlang/async を fork
git clone git@github.com:<your-fork>/async.git
cd async
git remote add upstream https://github.com/moonbitlang/async.git
git fetch upstream

# 2. ブランチ + patch
git checkout -b pr-event-loop-empty-timer upstream/main
git am < /path/to/pprof-mbt/notes/async-pr-drafts/01-event-loop-empty-timer/0001-event-loop-empty-timer.patch

# 3. テスト (network が要らないものだけ)
for pkg in moonbitlang/async moonbitlang/async/aqueue moonbitlang/async/semaphore \
           moonbitlang/async/cond_var moonbitlang/async/internal/coroutine \
           moonbitlang/async/internal/event_loop moonbitlang/async/internal/time; do
  moon test --target native -p $pkg
done

# 4. push & PR
git push -u origin pr-event-loop-empty-timer
gh pr create \
  --repo moonbitlang/async \
  --title "$(cat /path/to/pprof-mbt/notes/async-pr-drafts/01-event-loop-empty-timer/title.txt)" \
  --body-file /path/to/pprof-mbt/notes/async-pr-drafts/01-event-loop-empty-timer/body.md
```

## 追加で出す候補 (core 側にぶら下がる)

- `notes/core_set_grow_specialized.diff` (59 行) — async の
  `Set[Coroutine]` 経路に効く。**core 側の PR-02
  (`pr-drafts/02-hashmap-grow-rehash`) に折り込んで出す** 方向で考えている
  (4 並行構造 hashmap/hashset/Map/set を 1 PR にする)。
