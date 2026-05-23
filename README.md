# pprof-mbt

[MoonBit](https://www.moonbitlang.com/) を `native` / `wasm-gc` / `wasm` / `js`
の 4 バックエンドでビルドし、各実行を [pprof](https://github.com/google/pprof)
形式に正規化するプロファイリング道具一式。

中身は **MoonBit 用の既製品**:

- `moon-pprof` CLI 1 本で `profile / summary / bench` を実行
- MoonBit のシンボルマングルを demangle し、4 バックエンドのプロファイルを
  同じ pprof スキーマに揃える
- 改善 PR 作成のための **baseline ↔ patched 比較ワークフロー** (`patched-toolchain`
  / `patched-mooncakes` / `moon-pprof bench`)

ですが、**内部ライブラリは MoonBit 非依存にしてある**。`firefox-to-pprof` /
`wasmtime-guest-pprof` / `@mizchi/pprof-tools` の generic 部分は AssemblyScript /
Rust / Zig などの wasm にも転用可。[→ 詳細](#汎用-wasm-に転用する)

## クイックスタート

```sh
nix develop
cargo build --workspace --release
mkdir -p .bin && cp \
  target/release/moon-pprof \
  target/release/http-baseline-server \
  runners/patched-toolchain \
  runners/patched-mooncakes .bin/
chmod +x .bin/patched-toolchain .bin/patched-mooncakes
npm install
```

最初のプロファイルを取る:

```sh
npm run build:wasm-gc && npm run profile:wasm-gc     # → wasm-gc.pb.gz
.bin/moon-pprof summary wasm-gc.pb.gz                # ターミナルで Top-N
go tool pprof -http :8000 wasm-gc.pb.gz              # ブラウザで UI
```

## CLI

### `moon-pprof` — 統合 CLI

| サブコマンド | 用途 |
|---|---|
| `moon-pprof profile <wasm>` | wasm を `wasmtime + GuestProfiler` で実行 → pprof gzip |
| `moon-pprof profile --no-profile <wasm>` | profile を切ってクリーンな wasm wall-time 計測 |
| `moon-pprof summary <file>` | self-time / mem-mgmt rollup を Top-N 表示 |
| `moon-pprof summary --diff <a> <b>` | 関数毎の改善 / 退行 / 出現 / 消失を Top-N で表示 |
| `moon-pprof bench` | 複数 workload × 複数 backend で baseline ↔ patched 自動切替 → markdown 表 |

`--mem-pattern <regex>` で `summary` の mem-mgmt 分類 regex を上書き
可能 (default は MoonBit 用)。`moon-pprof bench` は `--baseline-moon` /
`--patched-moon` (core toolchain swap) と `--mooncakes-baseline` /
`--mooncakes-patched` (registry-dep swap) の 2 軸サポート。

### 補助ツール

| ツール | 用途 |
|---|---|
| `patched-toolchain` | `~/.moon` を `/tmp` に snapshot → diff 適用 → 全 backend rebundle (core PR 用) |
| `patched-mooncakes` | `<bench-dir>/.mooncakes/` を `/tmp` に snapshot → restore (registry dep PR 用) |
| `http-baseline-server` | port 30003 で空ハンドラ HTTP (axum)、k6 比較の baseline 用 |
| `node runners/run-wasm-gc.mjs <wasm>` | wasm-gc を Node V8 で実行 + .cpuprofile 出力 (`--no-profile` で wall time) |
| `node runners/run-js.mjs <js>` | 同 js |
| `node runners/cpuprofile-to-pprof.mjs <in> <out>` | V8 .cpuprofile → pprof gzip |
| `node runners/samply-to-pprof.mjs ...` | samply Firefox JSON → pprof |

### 典型ワークフロー: 改善 PR を作る

`moonbitlang/core` 系 (`~/.moon` の core を書き換える) — bigint PR の再現:

```sh
.bin/patched-toolchain init
.bin/patched-toolchain apply notes/pr-drafts/01-bigint-mul-single-limb/patch.diff
.bin/patched-toolchain rebundle
.bin/moon-pprof bench --workloads bigint_ops,bigint_square --runs 3
```

`moonbitlang/x` 系 (registry dep を書き換える) — uuid PR の再現:

```sh
.bin/patched-mooncakes init bench-x
cp -r /tmp/pprof-mbt-mooncakes/bench-x /tmp/pprof-mbt-mooncakes/bench-x.patched
( cd /tmp/pprof-mbt-mooncakes/bench-x.patched/moonbitlang/x \
  && patch -p1 < $(pwd)/notes/x-pr-drafts/04-uuid-tostring-inplace/patch.diff )

.bin/moon-pprof bench \
  --bench-dir bench-x \
  --backends native,wasm-gc,js \
  --workloads uuid_parse \
  --mooncakes-baseline /tmp/pprof-mbt-mooncakes/bench-x \
  --mooncakes-patched /tmp/pprof-mbt-mooncakes/bench-x.patched \
  --runs 3
```

→ markdown 表に native -64% / wasm-gc -45% / js -39% が出る。

## バックエンド別プロファイル

| backend | profile source | サンプル方式 | pprof 化 |
|---------|---------------|---------|---------|
| `wasm-gc` | Node inspector (V8) | V8 sampling | `cpuprofile-to-pprof.mjs` |
| `js`      | Node inspector (V8) | V8 sampling | `cpuprofile-to-pprof.mjs` |
| `wasm`    | wasmtime `GuestProfiler` (Cranelift JIT) | epoch tick sampling | `firefox-to-pprof` crate / `wasmtime-to-pprof.mjs` |
| `native`  | samply (Mach-O / ELF) | OS sampling | `samply-to-pprof.mjs` |

どれもマングル名 (`_M0FP26mizchi5bench9ackermann` の類) を pprof に流し、
共通の demangle で `mizchi::bench::ackermann` に戻す。

### wasm-gc (V8 経由)

```sh
npm run build:wasm-gc && npm run profile:wasm-gc
```

`moon build --no-strip` で wasm に関数名を保持。`runners/run-wasm-gc.mjs`
が `spectest.print_char` 等の moonbit host import を提供しつつ Node
inspector の `Profiler.start`/`Profiler.stop` で V8 CPU profile を取得、
`cpuprofile-to-pprof.mjs` で pprof protobuf に変換しつつ demangle。

### js (Node)

```sh
npm run build:js && npm run profile:js
```

moonbit の JS バックエンドはマングル名をそのまま JS 関数名として吐く
(`function _M0FP26mizchi5bench3fib(n) {...}`)。Node の inspector がそれを
そのまま CPU profile に入れるので、wasm-gc と同じ converter が使える。

### wasm (wasmtime + GuestProfiler)

```sh
npm run build:wasm && npm run profile:wasm
```

wasmtime CLI の `--profile=guest` 相当を Rust API で組む。Cranelift JIT
で wasm をフルスピード実行しつつ、別スレッドで `engine.increment_epoch()`
を周期的に呼び、`epoch_deadline_callback` 内で `GuestProfiler::sample`
が回る。`firefox-to-pprof` crate で Firefox JSON を pprof + gzip に変換。
host import は `moonbit-wasm-host` crate が提供。

### native (samply 経由)

```sh
npm run build:native && npm run profile:native
```

samply で OS サンプリングプロファイル (Firefox Profiler 形式) を取得。
`--unstable-presymbolicate` で `.syms.json` サイドカーに OS シンボル情報、
`samply-to-pprof.mjs` で pprof に変換 (インライン展開含む)。

## ライブラリとして使う

Rust と npm の 2 系統で外部プロジェクトから取り込み可能。

### Rust

```toml
[dependencies]
moonbit-demangle      = "0.1"
firefox-to-pprof      = "0.1"  # 汎用: samply / wasmtime の JSON を pprof に
wasmtime-guest-pprof  = "0.1"  # 汎用: wasmtime app に組み込む
moonbit-wasm-host     = "0.1"  # moonbit wasm の host import を 1 行で登録
```

```rust
use moonbit_demangle::demangle;
assert_eq!(demangle("_M0FP26mizchi5bench9ackermann"), "mizchi::bench::ackermann");
```

### JavaScript

```js
// 汎用
import { convert } from "@mizchi/pprof-tools/cpuprofile-to-pprof";
import { writePprofFromFirefox } from "@mizchi/pprof-tools/firefox-to-pprof";

// MoonBit 用
import { demangle } from "@mizchi/pprof-tools/moonbit/demangle";
import {
  moonbitWasmImports,
  autoStubMissing,
} from "@mizchi/pprof-tools/moonbit/wasm-host-imports";
```

## 汎用 wasm に転用する

ライブラリ部分は MoonBit 専用ではない。Rust / AssemblyScript / Zig 等の
wasm を pprof でプロファイルしたい場合:

**Rust (wasmtime + Cranelift JIT で実行)**:

```rust
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_guest_pprof::{ProfileSession, ProfilerHost, ProfilerHostExt as _, TakeProfileSession};

// MoonBit 専用のものは外す:
// - `moonbit-demangle` の代わりに自前の demangler (or 恒等関数)
// - `moonbit-wasm-host` を使わず、自分のアプリの host import を登録
//
// 残り (wasmtime-guest-pprof + firefox-to-pprof) はそのまま使える。
```

`firefox-to-pprof::Builder::demangle_with()` で symbol decoder を差し替え可:

```rust
let bytes = firefox_to_pprof::Builder::new(&profile, frames, samples)
    .demangle_with(|s| my_demangle(s))   // ← MoonBit 以外でも OK
    .encode()?;
```

**Node / V8 .cpuprofile**:

```js
import { convert } from "@mizchi/pprof-tools/cpuprofile-to-pprof";
const { encoded } = convert(cpuprofile, { demangle: (s) => s });  // 恒等関数で moonbit demangle を無効化
```

**pprof-summary の mem-mgmt 分類**:

`moon-pprof summary --mem-pattern <regex>` (または `$PPROF_SUMMARY_MEM_PATTERN`)
で、`moonbit_drop_object` 等の moonbit 固有 regex を任意の runtime
プリミティブ名に差し替えられる。

## レイアウト

```
Cargo.toml                              ← Rust workspace
package.json                            ← npm workspace (workspaces: packages/*)

crates/                                 公開ライブラリ (Rust)
├── moonbit-demangle/                   mangled symbol → readable name
├── moonbit-wasm-host/                  moonbit wasm の host import (spectest / WASI)
├── firefox-to-pprof/                   Firefox Profiler JSON → pprof (汎用)
└── wasmtime-guest-pprof/               wasmtime GuestProfiler 駆動 + pprof (汎用)

packages/                               公開ライブラリ (npm)
└── pprof-tools/                        @mizchi/pprof-tools (汎用 + moonbit subpath)

runners/                                CLI / binary
├── moon-pprof/                         Rust。統合 CLI
├── http-baseline-server/               Rust (axum + tokio)。k6 比較の baseline
├── patched-toolchain                   bash。~/.moon snapshot / patch / rebundle
├── patched-mooncakes                   bash。.mooncakes/ snapshot / patch / restore
├── run-wasm-gc.mjs / run-js.mjs        Node V8 inspector ラッパー
├── cpuprofile-to-pprof.mjs             V8 .cpuprofile → pprof
├── samply-to-pprof.mjs                 samply → pprof
└── wasmtime-to-pprof.mjs               wasmtime guest JSON → pprof

bench/                                  MoonBit ベンチ workload (ackermann / fib / mandel)
bench-async/                            moonbitlang/async 検証用 (coroutine / HTTP server)
bench-x/                                moonbitlang/x 検証用 (uuid / base64 / encoding / ...)
notes/                                  調査ログ + upstream 向け PR 素材
```

## セットアップ詳細

```sh
nix develop
```

[moonbit-overlay](https://github.com/moonbit-community/moonbit-overlay)
経由で `moon` が、その他 Node.js / Rust / wasmtime / samply / wabt /
protobuf / graphviz が入る (`go` は visualization 用の `go tool pprof`
のためだけに残してある。リポジトリ内に Go コードはなし)。

```sh
mkdir -p .bin

# Rust workspace を release ビルド + .bin にコピー
cargo build --workspace --release
cp target/release/moon-pprof target/release/http-baseline-server .bin/

# bash スクリプトを .bin に
cp runners/patched-toolchain runners/patched-mooncakes .bin/
chmod +x .bin/patched-toolchain .bin/patched-mooncakes

# npm workspace は symlink 解決のみ
npm install
```

## ベンチコード

`bench/bench.mbt` に CPU バウンドな workload を 3 つ。`bench/cmd/main/main.mbt`
がそれを呼ぶ:

- `ackermann(3, 10)` — 深い再帰
- `fib(32)` — 古典的再帰
- `mandel_sum(160, 500)` — 二重ループ＋浮動小数点

全バックエンド共通のコード。

`bench-async/` (moonbitlang/async 検証) と `bench-x/` (moonbitlang/x 検証)
にもそれぞれ複数の workload を置いてある。詳細は
[`notes/async_investigation.md`](notes/async_investigation.md) と
[`notes/x_investigation.md`](notes/x_investigation.md) 参照。

## 調査ログ / upstream 向け patches

`notes/` 配下にプロファイルから導いた patch 実験と upstream PR 用素材。

### `moonbitlang/core`

| 文書 | 内容 |
|---|---|
| [`notes/data_structures_comparison.md`](notes/data_structures_comparison.md) | 14 workload × 4 backend のクロス測定 (refcount 仮説の検証) |
| [`notes/patch_experiments.md`](notes/patch_experiments.md) | 10 個のパッチ実験 (7 採用 / 1 議論先行 / 2 不採用) |
| [`notes/pr_numbers.md`](notes/pr_numbers.md) | `--no-profile` で取った各 PR 単独の clean 数値 |
| [`notes/pr_plan.md`](notes/pr_plan.md) | 既存 upstream PR/Issue との重複チェック + 提出計画 |
| [`notes/pr-drafts/`](notes/pr-drafts/) | moonbitlang/core 向け PR 素材 (4 PR + 1 Issue) |

### `moonbitlang/async`

| 文書 | 内容 |
|---|---|
| [`notes/async_investigation.md`](notes/async_investigation.md) | callgrind 経由のプロファイル + 2 パッチ |
| [`notes/async_http_server_profile.md`](notes/async_http_server_profile.md) | k6 + callgrind で HTTP server を計測 |
| [`notes/async_backend_comparison.md`](notes/async_backend_comparison.md) | 4 バックエンド比較 |
| [`notes/async-pr-drafts/`](notes/async-pr-drafts/) | moonbitlang/async 向け PR 素材 (1 PR) |

### `moonbitlang/x`

| 文書 | 内容 |
|---|---|
| [`notes/x_investigation.md`](notes/x_investigation.md) | プロファイル + 6 パッチ |
| [`notes/x_cross_backend.md`](notes/x_cross_backend.md) | パッチを native / wasm-gc / js で交差検証 |
| [`notes/x-pr-drafts/`](notes/x-pr-drafts/) | moonbitlang/x 向け PR 素材 (6 PR) |

### このリポジトリ自身のロードマップ

| 文書 | 内容 |
|---|---|
| [`notes/pprof_mbt_roadmap.md`](notes/pprof_mbt_roadmap.md) | v1 ロードマップ (core 調査直後) |
| [`notes/pprof_mbt_roadmap_v2.md`](notes/pprof_mbt_roadmap_v2.md) | v2 (async + x 調査を経た時点での更新) |

## 既知の制約 / TODO

- **メモリプロファイルは未対応** (CPU のみ)。
- **demangle はヒューリスティック**。impl / method / generic 修飾子
  (`_M0I…`, `_M0M…`, `GsE`/`GuE` 接尾辞) は `core::` プレフィックスを
  落とすなど不完全。
- **llvm バックエンド** (`moon build --target=llvm`) は MoonBit 側の
  ビルドエラーで未検証。
- **Linux で native プロファイル** (samply 相当) は環境次第。`perf` 経由の
  pprof 変換 (`perf-to-pprof` crate) は
  [`notes/pprof_mbt_roadmap_v2.md`](notes/pprof_mbt_roadmap_v2.md) で TODO。

## License

Apache-2.0
