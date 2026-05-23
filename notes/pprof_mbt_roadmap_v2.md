# pprof-mbt の改善案 (v2)

`notes/pprof_mbt_roadmap.md` (core 調査直後に書いたもの) を、async 調査と
x 調査を経た現時点で見直したもの。

## v1 で挙げて、その後 done になった項目

- ✅ **wasmtime-runner `--no-profile`** — 実装済み。x の cross-backend 計測
  でも `runners/run-wasm-gc.mjs --no-profile` を使用。
- ✅ **bench-runner CLI (markdown 出力)** — `.bin/bench-runner` 完成。
  `-bench-dir bench-x` で bench-x も走る。baseline / patched は toolchain
  root を別にする方式。
- ✅ **patched-toolchain helper** — `runners/patched-toolchain` script で
  `~/.moon` → `/tmp/moonbit-patched` の snapshot + diff 適用 + 全 backend
  rebundle が 1 コマンド。
- 🟡 **pprof-summary** — single-profile 版は実装済 (`.bin/pprof-summary
  <profile.pb.gz>`)。`--diff` モードは未着手。

## v1 から残っている + 今回の調査で新たに見えた痛み

### [P0] 1. `.mooncakes/<dep>/` patch workflow が無い

**今回の最大の痛み。** core は `patched-toolchain` で扱えるが、
`moonbitlang/x` や `moonbitlang/async` のような registry dep の中身を
書き換えて測るには `bench-x/.mooncakes/moonbitlang/x/...` を直接 edit
するしかない。問題:

- `moon install` / `rm -rf .mooncakes` で patch が消える。
- baseline ↔ patched の切替が「ファイルコピー」ベースで脆い。
- 複数 PR の独立計測には毎回 diff 適用 → revert が要る。

**提案**: `patched-mooncakes` script を追加。

```sh
.bin/patched-mooncakes snapshot bench-x        # .mooncakes → /tmp/pprof-mbt-mooncakes/bench-x.baseline
.bin/patched-mooncakes apply bench-x notes/x-pr-drafts/04-uuid/patch.diff
.bin/patched-mooncakes restore bench-x         # back to baseline
.bin/patched-mooncakes status bench-x          # which files differ
```

加えて bench-runner にも `-mooncakes-baseline /path/A -mooncakes-patched /path/B`
を足し、registry dep のパッチも cell の半分として扱う。

### [P0] 2. wasm-gc / js host shim が薄い

`path_normalize` が wasm-gc で動かなかった (`__moonbit_sys_unstable` 未提供)。
`path_normalize` js で動かなかった (`require()` を ES module 内で呼んでる)。
moonbit の native 以外で `@env` / `@fs` / `@sys` を触るモジュールは全滅。

**提案**:
- `runners/run-wasm-gc.mjs`: 不足 import (`__moonbit_sys_unstable`,
  `wasi_snapshot_preview1`) を stub で供給。`is_windows` は即 `false` 返し。
- `runners/run-js.mjs`: 起動時に `globalThis.require = createRequire(...)`
  を仕込んで CJS `require()` を許可、あるいは moonbit の js 出力を
  `.mjs` として読めるよう変換 wrapper。

これがあると x の `path`, `fs`, `sys` も 4 バックエンドで計測できる。

### [P1] 3. pprof-summary `--diff baseline.pb.gz patched.pb.gz`

「どの関数が何 ms 改善 / 退行したか」を 1 行で知りたい。Top N 形式で:

```
$ .bin/pprof-summary --diff base.pb.gz patched.pb.gz
function                                    base ms   patched ms   delta
SHA256::transform                           1402.3      1180.1   -222.2 (-15.8%)
moonbit_drop_object                          184.0       142.6    -41.4 (-22.5%)
...
```

`go tool pprof -base` でも出るが、出力フォーマットが調査向けでない。

### [P1] 4. JIT / startup warmup の制御

短いベンチ (例: `bigint_square` 18ms) では V8 / Cranelift の JIT 編集
時間が計測に入る。bench-runner に `--warmup N --measure M` を足し、
warmup を sample から除外する。これで標準偏差が大幅に減るはず。

### [P1] 5. cross-repo / cross-backend matrix の再現性

x 調査で `/tmp/bench-x-cross.sh` という ad-hoc script を書いた。3 backend
× 7 bench × (baseline, patched) の grid を組むのに毎回手作業。

**提案**: bench-runner を拡張して

```sh
.bin/bench-runner \
  -bench-dir bench-x \
  -backends native,wasm-gc,js \
  -mooncakes-baseline /tmp/snap/baseline \
  -mooncakes-patched /tmp/snap/patched \
  -workloads uuid_parse,encoding_utf8,base64_encode \
  -runs 3 -markdown -tsv-out delta.tsv
```

→ Markdown 表 + TSV を同時に出す。`notes/` に貼れる形で。

### [P1] 6. Linux native プロファイル (perf-to-pprof)

samply が Linux container に入らなかったため、x も async も core も
native の callgrind に頼っている。callgrind は CPU instructions で wall
time じゃないし、JIT compile も計測対象外。

**提案**: `crates/perf-to-pprof` を追加 (新規 crate)。`perf record -o
out.perf` → `perf script` → demangle → pprof gzip。これで native も
wasm と同じスキーマで比較可能になる。

### [P2] 7. README → notes/ index

`notes/` は今 38 ファイル + 3 サブディレクトリ。README に index 章を
1 つ足して、各 patch の (diff, bench, PR draft, 結果) を 1 行で
追えるテーブルを置きたい:

| topic | investigation log | PR draft | status |
|---|---|---|---|
| bigint mul single-limb | json_parse_findings.md | pr-drafts/01-... | upstream merged |
| x uuid to_string | x_investigation.md | x-pr-drafts/04-... | draft |
| ... | | | |

これは notes/x_investigation.md の右側に書くより README が良い。

### [P2] 8. per-callsite mem-mgmt rollup

`pprof-summary` は関数粒度。`parse_value` の **どの行** が alloc
発生源か、を見るには pprof 側の line info が要る。今は手作業で
`go tool pprof -peek` している。

`pprof-summary --by-line` を足したい。

### [P2] 9. wasm の overhead 説明を docs に明示

`GuestProfiler` で取った wasm time は ~10〜15% inflate (epoch
callback の overhead 込み)。`--no-profile` で取った wasm の純 wall
time を併記すべき。`notes/backend_comparison.md` には書いたが、
README には書いてない。

### [P3] 10. CI で bench 値を回帰させる

PR ごとに主要 bench を回し過去 commit との delta を表示。moonbit
toolchain の version up 時の影響も自動追跡。リソース消費が大きいので
priority 低い。

## ツール群の自己評価 — 今回の調査で一番効いたもの

| ツール | 効いた場面 | 評価 |
|---|---|---|
| pprof-summary | core / async の "memory-mgmt time 60%" を一発で出した | ★★★ |
| demangle 統合 | 全 pprof view で `mizchi::bench::ackermann` 読める | ★★★ |
| 4 backend pprof スキーマ統一 | wasm vs wasm-gc の "refcount 仮説" 検証 | ★★★ |
| wasmtime-runner --no-profile | wasm の clean wall time 計測 | ★★ |
| bench-runner | core PR の baseline/patched 比較表生成 | ★★ |
| patched-toolchain | core の patch サイクルを 30s → 5s に | ★★ |
| `notes/x-pr-drafts/<N>/` の構造 | 6 PR の素材を機械的に作れた | ★ |
| `valgrind callgrind` (外部) | native プロファイラ代用、本来不要 | (samply / perf 欲しい) |

## 次にやるなら何から

今回の調査 cycle (core → async → x) を見ると、各リポジトリで同じ
パターンを繰り返している:

1. ベンチ追加
2. callgrind / V8 inspector でプロファイル
3. パッチ書く
4. baseline ↔ patched で計測
5. PR draft 作る

このうち **(1)→(2)→(4)** の自動化が一番効く。具体的には:

- **P0-1 (patched-mooncakes)** + **P1-5 (bench-runner registry-aware)**
  をセットで実装すれば「bench-x で patch を当てて 3 backend で測る」が
  1 コマンドに。
- **P0-2 (host shim 充実)** で path/fs/sys モジュールも計測対象に。
- **P1-3 (pprof-summary --diff)** で「どこが改善したか」を即座に
  検証可能に。

この 3 つで x 系の調査 cycle が「半日」→「1時間」になる。残りの core
モジュール (rational, time の残り, json5 lex_ident) の調査もスケール
する。
