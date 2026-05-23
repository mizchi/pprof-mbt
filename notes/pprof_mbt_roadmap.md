# pprof-mbt 自身の改善方向

本リポジトリは「moonbit を 4 バックエンドで pprof に正規化する」道具。
今回の core 調査を通して見えた不足点と次にやるべき改善。

## 現状の良い点 (今回効いた所)

- **4 バックエンド共通 pprof スキーマ** が refcount 仮説の証拠を出した。
  これは他の比較ツールでは出せない情報。
- **pprof-summary の "Memory-management self time" rollup** が
  「wasm の 60% が refcount」を 1 行で示した。調査の方向性を決めた最大の値。
- **demangle 統合** で `_M0FP26mizchi5bench9ackermann` ではなく
  `mizchi::bench::ackermann` が読めた。patch 提案までの認知負荷を大幅に減らした。

## 痛かった/不足だった点

### 1. wasmtime-runner に `--no-profile` モードが無い

現状は常に `epoch_interruption(true)` で profile を取るので、wasm の
wall-time が ~10〜15% inflate されている。`bigint_ops` で
「wasm 411ms / wasm-gc 88ms → 4.7×」の比は実際より広めに出ている可能性。

`--no-profile` を足して、profile 不要時は素の wasmtime 実行にしたい。
PR 用のベンチ数値はその方が信頼できる。

### 2. ベンチランナーが手動シェルスクリプト

`baseline 3 run → patched 3 run → 表組み` を毎回 shell で組んでいる。
専用 driver があるべき:

```sh
.bin/bench-runner --workloads bench/cmd \
  --backends wasm,wasm-gc,js,native \
  --toolchains baseline=$HOME/.moon,patched=/tmp/moonbit-patched \
  --runs 3 --markdown
```

→ 直接 markdown 表で出して notes に貼れる。

### 3. pprof diff サポート

「baseline.pb.gz と patched.pb.gz を入れたら top 5 改善/退行関数を
出す」ツールが要る。今は `go tool pprof -base` で手作業。
`pprof-summary --diff` を追加するだけで済む。

### 4. JIT warmup 補正

短いベンチ (bigint_square 18ms など) では V8 / Cranelift の JIT 編集
時間が混入して標準偏差が大きい。**warmup iter + 計測 iter** を分ける
runner オプションが欲しい:

```
node runners/run-wasm-gc.mjs ... --warmup 2 --measure 5
```

これで `compileForInternalLoader` 等が profile から消える。

### 5. Linux ホストでの native プロファイル

samply が `crates.io` の network policy 制約で `cargo install` 出来ず、
今回 native は wall-time だけしか取れなかった。**`perf` 統合**が要る:

```
.bin/perf-to-pprof --binary path/to/exe --output native.pb.gz
```

これがあると native でも `pprof-summary` 出せて、wasm との比較が
「3 backend × 1 profile schema」 → 「4 backend × 1 profile schema」になる。

### 6. moonbit toolchain swap workflow の自動化

毎回:

```sh
cp -r ~/.moon /tmp/moonbit-patched
chmod -R u+w /tmp/moonbit-patched
$EDITOR /tmp/moonbit-patched/lib/core/<pkg>/<file>.mbt
cd /tmp/moonbit-patched/lib/core && moon bundle --release --target wasm
cd $PROJECT/bench && MOON_TOOLCHAIN_ROOT=/tmp/moonbit-patched moon build ...
```

を手で並べているのが面倒。`pprof-mbt` 側に:

```sh
.bin/patched-toolchain init       # snapshot ~/.moon
.bin/patched-toolchain apply <diff>  # apply diff and rebundle
.bin/patched-toolchain bench <workload>  # build + measure baseline vs patched
```

があると patch 実験の cycle が分単位 → 秒単位に縮む。

### 7. wasm の絶対値の inflate 説明をどこかに書く

`backend_comparison.md` で wasm 値が大きいのは GuestProfiler の overhead
込みで、純粋な wall time ではない。読者にこれが伝わる README が要る。

### 8. cumulative-mem-mgmt の per-callsite ビュー

pprof-summary は「mem-mgmt-attributed time」を上位関数で表示するが、
**callsite (関数 + 行番号) 単位** だと「parse_value のどの行が alloc
を発生させているか」が見える。今は関数粒度。

### 9. `notes/` を README から index する

実験ログがどんどん溜まっているが、README から参照が無い。`notes/`
を index 化して、各 patch の diff・bench 結果・PR 状態を 1 ページで
追えるようにする。

### 10. CI で bench 値を回帰させる

現状は手動計測。GitHub Actions で毎 commit に主要ベンチを回し、
過去 commit との delta を PR に貼る仕組みがあると、moonbit toolchain
の version up の影響も自動追跡できる。

## 優先度

| # | 項目 | 効果 | コスト |
|---|---|---|---|
| 1 | wasmtime-runner `--no-profile` | wasm 数値の信頼性 ↑↑ | 小 (1日) |
| 2 | bench-runner CLI (markdown 出力) | 計測 workflow 1/5 | 中 (2-3日) |
| 3 | pprof-summary --diff | patch 検証 cycle 高速化 | 小 (1日) |
| 6 | patched-toolchain helper | patch 実験 cycle ↑↑ | 中 |
| 5 | perf-to-pprof | native profile が取れる | 中 |
| 4 | JIT warmup option | wasm-gc/js の数値安定化 | 小 |
| 8 | per-callsite mem-mgmt view | alloc 起源の特定 | 中 |
| 7,9 | README / notes index | 読みやすさ | 小 |
| 10 | CI bench | 退行検出 | 大 |

1,3,4,6 は **今回の patch PR を出す前に欲しい** (PR description で安定
した数値を出せる)。2,5,7,8,9,10 は後段で良い。
