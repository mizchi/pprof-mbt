# moon-pprof

> English version: [README.md](README.md).

[MoonBit](https://www.moonbitlang.com/) を `native` / `wasm-gc` / `wasm` / `js`
の 4 バックエンドでビルドし、各実行を [pprof](https://github.com/google/pprof)
形式に正規化するプロファイリング道具一式。

## インストール

> 最短経路は [`docs/quick-start.md`](docs/quick-start.md) — CLI を入れて、
> 同梱の sample wasm を profile して、 summary を読むまでが 1 分で終わる。
> pprof / Chrome trace / Speedscope / folded stack の変換表は
> [`docs/profile-formats.md`](docs/profile-formats.md) を参照。

CLI 1 本だけ欲しい場合 (任意の wasm を `profile` / `summary` / `cpuprofile2pprof`
/ `firefox2pprof` する):

```sh
# cargo (crates.io から、要: rustc 1.80+ と protoc が PATH に)
cargo install moon-pprof --locked

# nix (上記のビルド時 deps は flake に閉じ込め済み)
nix run github:mizchi/moon-pprof -- --help
nix profile install github:mizchi/moon-pprof           # 永続インストール
```

`main` を追いたいときは cargo 行を
`cargo install --git https://github.com/mizchi/moon-pprof moon-pprof --locked`
に差し替える。

`moon-pprof bench` を使うときだけは外部に `moon` / `node` / `samply` が要ります
(下のクイックスタートを参照、 もしくは `nix develop` でまとめて入る)。

## 中身

**MoonBit 用の既製品**:

- `moon-pprof` CLI 1 本で `profile / summary / bench` と各種 converter を実行
- MoonBit のシンボルマングルを demangle し、4 バックエンドのプロファイルを
  同じ pprof スキーマに揃える
- 改善 PR 作成のための **baseline ↔ patched 比較ワークフロー** (`patched-toolchain`
  / `patched-mooncakes` / `moon-pprof bench`)

ですが、**内部ライブラリは MoonBit 非依存にしてある**。 `firefox-to-pprof` /
`cpuprofile-to-pprof` / `chrome-trace-to-pprof` / `pprof-to-chrome-trace` /
`pprof-stack-formats` / `wasmtime-guest-pprof` の Rust crate 群は
AssemblyScript / Rust / Zig などの wasm にも転用可。
[→ 詳細](#汎用-wasm-に転用する)

## クイックスタート (リポジトリで開発する場合)

`nix develop` の中では
[moonbit-overlay](https://github.com/moonbit-community/moonbit-overlay)
経由で `moon` が、その他 Node.js / Rust / wasmtime / samply / wabt /
protobuf / graphviz が入る (`go` は visualization 用の `go tool pprof`
のためだけに残してある。リポジトリ内に Go コードはなし)。

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
| `moon-pprof profile --wasm-gc <wasm>` | wasm-gc を同じ wasmtime 経路でプロファイル (V8 経由ではなく Cranelift baseline) |
| `moon-pprof profile --no-profile <wasm>` | profile を切ってクリーンな wasm wall-time 計測 |
| `moon-pprof summary <file>` | self-time / mem-mgmt rollup を Top-N 表示 |
| `moon-pprof summary --diff <a> <b>` | 関数毎の改善 / 退行 / 出現 / 消失を Top-N で表示 |
| `moon-pprof bench` | 複数 workload × 複数 backend で baseline ↔ patched 自動切替 → markdown 表 |
| `moon-pprof cpuprofile2pprof <in> <out>` | V8 .cpuprofile → pprof gzip |
| `moon-pprof chrometrace2pprof <in> <out>` | Chrome trace-event JSON 内の V8 `Profile` / `ProfileChunk` → pprof gzip (`--profile-index` で複数 stream を選択) |
| `moon-pprof pprof2chrometrace <in.pb.gz> <out.json>` | pprof CPU profile → synthetic Chrome trace-event JSON。`--expand-samples` で round-trip 時の `samples/count` を保持 |
| `moon-pprof pprof2folded <in.pb.gz> <out.folded>` | pprof CPU profile → folded stacks (`root;child;leaf value`) |
| `moon-pprof folded2pprof <in.folded> <out.pb.gz>` | folded stacks → pprof gzip。既定は `delay/microseconds` で、off-CPU / blocking profiler の folded 出力を pprof に寄せる用途向け |
| `moon-pprof pprof2speedscope <in.pb.gz> <out.json>` | pprof CPU profile → Speedscope sampled JSON |
| `moon-pprof speedscope2pprof <in.json> <out.pb.gz>` | Speedscope sampled JSON → pprof gzip |
| `moon-pprof heapprofile2pprof <in> <out>` | V8 .heapprofile → allocation pprof gzip |
| `moon-pprof memprofile <wasm>` | wasm / wasm-gc allocation pprof。`--trace-out <trace.json>` で Chrome trace allocation timeline も出力 |
| `moon-pprof memprofile-native <exe>` | native allocation pprof。`--retained` で retained heap (`inuse_objects` / `inuse_space`) |
| `moon-pprof firefox2pprof <in> <out>` | Firefox Profiler JSON → pprof |
| `moon-pprof perf2pprof <perf-script.txt>` | Linux `perf script` テキスト → pprof gzip |

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
| `node runners/v8/run-wasm-gc.mjs <wasm>` | wasm-gc を Node V8 で実行 + .cpuprofile 出力 (`--no-profile` で wall time)。 default 経路 (`moon-pprof profile`) で wasmtime に乗らない数値を取りたい時の比較用 |
| `node runners/v8/run-js.mjs <js>` | js バックエンドを Node V8 で実行 (V8 必須) |
| `moon-pprof cpuprofile2pprof <in> <out>` | V8 .cpuprofile → pprof gzip (旧 `cpuprofile-to-pprof.mjs` の後継、 Rust 移植) |
| `moon-pprof chrometrace2pprof <in> <out>` | Chrome trace-event JSON → pprof gzip。V8 CPU profiler の `Profile` / `ProfileChunk` stream を読む |
| `moon-pprof pprof2chrometrace <in.pb.gz> <out.json>` | pprof → Chrome trace-event JSON。実タイムラインではなく synthetic V8 CPU profile として出力 |
| `moon-pprof pprof2folded <in.pb.gz> <out.folded>` | pprof → folded stacks |
| `moon-pprof folded2pprof <in.folded> <out.pb.gz>` | folded stacks → pprof gzip |
| `moon-pprof pprof2speedscope <in.pb.gz> <out.json>` | pprof → Speedscope JSON |
| `moon-pprof speedscope2pprof <in.json> <out.pb.gz>` | Speedscope sampled JSON → pprof gzip |
| `moon-pprof firefox2pprof <in> <out>` | Firefox Profiler JSON → pprof (`--source samply --syms <sidecar>` で samply の RVA + 旧 `samply-to-pprof.mjs` 相当、 default `--source wasmtime-guest` で 旧 `wasmtime-to-pprof.mjs` 相当) |

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
cp -r /tmp/moon-pprof-mooncakes/bench-x /tmp/moon-pprof-mooncakes/bench-x.patched
( cd /tmp/moon-pprof-mooncakes/bench-x.patched/moonbitlang/x \
  && patch -p1 < $(pwd)/notes/x-pr-drafts/04-uuid-tostring-inplace/patch.diff )

.bin/moon-pprof bench \
  --bench-dir bench-x \
  --backends native,wasm-gc,js \
  --workloads uuid_parse \
  --mooncakes-baseline /tmp/moon-pprof-mooncakes/bench-x \
  --mooncakes-patched /tmp/moon-pprof-mooncakes/bench-x.patched \
  --runs 3
```

→ markdown 表に native -64% / wasm-gc -45% / js -39% が出る。

## バックエンド別プロファイル

| backend | profile source | サンプル方式 | pprof 化 |
|---------|---------------|---------|---------|
| `wasm-gc` (default) | wasmtime `GuestProfiler` (Cranelift) | epoch tick sampling | `firefox-to-pprof` crate |
| `wasm-gc` (`--via-v8`) | Node inspector (V8) | V8 sampling | `v8/cpuprofile-to-pprof.mjs` |
| `js`      | Node inspector (V8) | V8 sampling | `v8/cpuprofile-to-pprof.mjs` |
| `wasm`    | wasmtime `GuestProfiler` (Cranelift JIT) | epoch tick sampling | `firefox-to-pprof` crate |
| `native`  | samply (Mach-O / ELF) | OS sampling | `firefox-to-pprof::samply` + `firefox-to-pprof` crate |

どれもマングル名 (`_M0FP26mizchi5bench9ackermann` の類) を pprof に流し、
共通の demangle で `mizchi::bench::ackermann` に戻す。

### wasm-gc (wasmtime, default)

```sh
npm run build:wasm-gc && npm run profile:wasm-gc
```

`moon build --no-strip --target=wasm-gc` で関数名を保持した wasm-gc を出し、
`moon-pprof profile --wasm-gc` が wasmtime engine (`Config::wasm_gc(true)`
+ `wasm_function_references(true)` + `wasm_reference_types(true)`) で
ロード → GuestProfiler で epoch tick サンプリング → pprof gzip に変換。
`moonbit-wasm-host` crate が `spectest.print_char` / `wasi fd_write`
host import を 1 行で登録する。

V8 inline cache が乗った状態の wall time を測りたい場合は V8 経路も
残してある:

```sh
npm run profile:wasm-gc:v8   # 旧 Node V8 inspector 経路 (比較用)
# あるいは:
.bin/moon-pprof bench --backends wasm-gc --wasm-gc-via-v8 ...
```

wasmtime (Cranelift baseline) と V8 (inline cache) では同じ wasm-gc
バイナリでも自己時間の分布が変わる。 hot path のトポロジ (どの関数が
重いか) はほぼ一致するが、絶対値や比率は別物として扱う。

なお wasm-gc バックエンドの alloc は wasm の GC 命令 (`struct.new`
等) で行われるため、`--mem-pattern` の mem-mgmt 分類は反応しない。
GC オーバーヘッドを追いたい場合は別途 GC 命令ベースの計測が要る。

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
`--unstable-presymbolicate` で `.syms.json` サイドカーに OS シンボル情報を出し、
`moon-pprof firefox2pprof --source samply --syms <sidecar>` で pprof に変換
(インライン展開含む)。 RVA → enclosing symbol の binary search は
`firefox-to-pprof::samply::SamplySymsResolver` が担当。

## ライブラリとして使う

Rust と npm の 2 系統で外部プロジェクトから取り込み可能。

### Rust

10 crate すべて crates.io にある。必要なものだけ抜く:

```toml
[dependencies]
moonbit-demangle      = "0.1"
firefox-to-pprof      = "0.1"  # 汎用: samply / wasmtime の JSON を pprof に
cpuprofile-to-pprof   = "0.1"  # 汎用: V8 .cpuprofile を pprof に
chrome-trace-to-pprof = "0.1"  # 汎用: Chrome trace-event の V8 ProfileChunk を pprof に
pprof-to-chrome-trace = "0.1"  # 汎用: pprof を synthetic Chrome trace-event V8 ProfileChunk に
pprof-stack-formats   = "0.1"  # 汎用: pprof ↔ Speedscope, pprof ↔ folded stacks
heapprofile-to-pprof  = "0.1"  # 汎用: V8 .heapprofile を pprof に
perf-to-pprof         = "0.1"  # 汎用: Linux `perf script` のテキストを pprof に
wasmtime-guest-pprof  = "0.1"  # 汎用: wasmtime app に組み込む
moonbit-wasm-host     = "0.1"  # moonbit wasm の host import を 1 行で登録
```

```rust
use moonbit_demangle::demangle;
assert_eq!(demangle("_M0FP26mizchi5bench9ackermann"), "mizchi::bench::ackermann");
```

### JavaScript

```js
import {
  moonbitWasmImports,
  autoStubMissing,
} from "@mizchi/moonbit-wasm-host";
```

> pprof 変換系 (`cpuprofile-to-pprof` / `chrome-trace-to-pprof` /
> `pprof-to-chrome-trace` / `pprof-stack-formats` / `firefox-to-pprof` / `moonbit/demangle`) は Rust crate に移管しました。 CLI からは
> `moon-pprof cpuprofile2pprof` / `moon-pprof chrometrace2pprof` /
> `moon-pprof pprof2chrometrace` / `moon-pprof pprof2speedscope` /
> `moon-pprof speedscope2pprof` / `moon-pprof pprof2folded` /
> `moon-pprof folded2pprof` / `moon-pprof firefox2pprof` で
> 呼べます。 npm 側に残るのは MoonBit wasm を Node V8 で実行する
> ときの host import (`spectest.print_char` / WASI `fd_write`) のみ。

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

CLI で済ますなら:

```sh
moon-pprof cpuprofile2pprof --no-demangle in.cpuprofile out.pb.gz
```

ライブラリとして組み込むなら `cpuprofile-to-pprof` crate:

```rust
use cpuprofile_to_pprof::{Builder, CpuProfile};
let profile: CpuProfile = serde_json::from_slice(&bytes)?;
let out = Builder::new(profile)
    .demangle_with(|s| s.to_string())  // moonbit demangle を無効化
    .encode()?;
std::fs::write("out.pb.gz", out.encoded)?;
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
├── cpuprofile-to-pprof/                V8 .cpuprofile → pprof (汎用)
├── chrome-trace-to-pprof/              Chrome trace-event V8 ProfileChunk → pprof (汎用)
├── pprof-to-chrome-trace/              pprof → synthetic Chrome trace-event V8 ProfileChunk (汎用)
├── pprof-stack-formats/                pprof ↔ Speedscope, pprof ↔ folded stacks (汎用)
├── heapprofile-to-pprof/               V8 .heapprofile → pprof (汎用)
├── perf-to-pprof/                      Linux perf script テキスト → pprof (汎用)
└── wasmtime-guest-pprof/               wasmtime GuestProfiler 駆動 + pprof (汎用)

packages/                               公開ライブラリ (npm)
└── moonbit-wasm-host/                  @mizchi/moonbit-wasm-host (Node V8 で moonbit wasm を動かすときの host import)

runners/                                CLI / binary
├── moon-pprof/                         Rust。統合 CLI
├── http-baseline-server/               Rust (axum + tokio)。k6 比較の baseline
├── patched-toolchain                   bash。~/.moon snapshot / patch / rebundle
├── patched-mooncakes                   bash。.mooncakes/ snapshot / patch / restore
├── v8/                                 Node V8 inspector 経由の経路
│   ├── run-wasm-gc.mjs                 wasm-gc を V8 で実行 (--via-v8)
│   └── run-js.mjs                      js を V8 で実行
│                                       (.cpuprofile → pprof は moon-pprof cpuprofile2pprof)
│                                       (Chrome trace JSON → pprof は moon-pprof chrometrace2pprof)
│                                       (pprof → Chrome trace JSON は moon-pprof pprof2chrometrace)
│                                       (pprof ↔ Speedscope / folded は moon-pprof pprof2speedscope / speedscope2pprof / pprof2folded / folded2pprof)
                                        (samply / wasmtime guest JSON →
                                         pprof は moon-pprof firefox2pprof)

bench/                                  MoonBit ベンチ workload (ackermann / fib / mandel)
bench-async/                            moonbitlang/async 検証用 (coroutine / HTTP server)
bench-x/                                moonbitlang/x 検証用 (uuid / base64 / encoding / ...)
notes/                                  調査ログ + upstream 向け PR 素材
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

- **メモリプロファイルは js + wasm + wasm-gc + native (macOS) 対応**。 `moon-pprof
  heapprofile2pprof` で Node V8 の sampling allocation profile を、
  `moon-pprof memprofile` で wasm / wasm-gc を walrus instrumentation +
  wasmtime backtrace 経由で pprof 化できる。`--trace-out` で allocation の
  Chrome trace timeline も出せるが、これは hook 由来の allocation activity
  であり true GC pause trace ではない。wasm-gc 側は field-sum proxy で wasmtime 実 heap
  consumption とは厳密一致しない。 大きい workload には
  `--sample-rate 100` を渡すと ~22 倍速 + top site の誤差 0.1% 以内。
  native は `moon-pprof memprofile-native <exe>` で対応 — 生成済み
  `<cmd>.c` の `moonbit_malloc_inlined` を patch し、 backtrace 取得 hook を
  link して再 cc → 走らせる。`--retained` を付けると `moonbit_free` も patch
  して process exit 時点の `inuse_objects` / `inuse_space` を出す
  (sample-rate=1 で exact、>1 は sampled estimate)。これは `mimalloc` が
  静的リンクされてて `DYLD_INSERT_LIBRARIES` では捕まえられない問題への対処。
  sample-rate=100 で ~70 倍速。macOS + Linux glibc 対応。
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
