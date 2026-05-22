# pprof-mbt

[MoonBit](https://www.moonbitlang.com/) を wasm-gc / wasm / js / native の
4 バックエンドでビルドし、各実行を [pprof](https://github.com/google/pprof)
形式でプロファイルする実験プロジェクト。

中身は実験プロジェクト + 3 言語のライブラリ群:

- **Rust crates** — `moonbit-demangle`, `firefox-to-pprof`, `wasmtime-guest-pprof`
- **Go module** — `github.com/mizchi/pprof-mbt/go/demangle`
- **npm package** — `moonbit-pprof` (subpath exports: `./demangle`, `./cpuprofile-to-pprof`, `./firefox-to-pprof`)

これらは MoonBit を扱う他のツールから個別に再利用できる形にしてある。

## レイアウト

```
Cargo.toml                              ← Rust workspace
go.work                                 ← Go workspace
package.json                            ← npm workspace

crates/                                 公開ライブラリ (Rust)
├── moonbit-demangle/                   mangled symbol → readable name
├── firefox-to-pprof/                   Firefox Profiler JSON → pprof
└── wasmtime-guest-pprof/               wasmtime GuestProfiler 駆動 + pprof

go/                                     公開ライブラリ (Go)
└── demangle/                           Symbol(name) string

packages/                               公開ライブラリ (npm)
└── moonbit-pprof/                      demangle / cpuprofile / firefox subpath exports

runners/                                CLI / binary
├── wasmtime-runner/                    Rust。3 crate を使う thin main.rs
├── wzprof-runner/                      Go (legacy 比較用)
├── run-wasm-gc.mjs / run-js.mjs        Node V8 inspector
├── cpuprofile-to-pprof.mjs
├── samply-to-pprof.mjs
└── wasmtime-to-pprof.mjs

bench/                                  MoonBit ベンチ workload (ackermann / fib / mandel)
```

## セットアップ

```sh
nix develop
```

[moonbit-overlay](https://github.com/moonbit-community/moonbit-overlay)
経由で `moon`、その他 Node.js / Go / Rust / wasmtime / samply / wabt /
protobuf / graphviz が入る。

各言語の runner / binary をビルド:

```sh
mkdir -p .bin

# Rust: workspace 一括ビルド + .bin にコピー
cargo build --workspace --release
cp target/release/wasmtime-runner .bin/

# Go: 2 binary (legacy + 補助 CLI)
( cd runners/wzprof-runner && go build -buildvcs=false -o ../../.bin/wzprof-runner . )
( cd runners/wzprof-runner && go build -buildvcs=false -o ../../.bin/pprof-demangle ./cmd/demangle )

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

## 各バックエンドでのプロファイル

### wasm-gc (V8 経由)

```sh
npm run build:wasm-gc && npm run profile:wasm-gc
go tool pprof -http :8000 wasm-gc.pb.gz
```

- `moon build --no-strip` で wasm に関数名 (`_M0FP26mizchi5bench9ackermann` 等)
  を保持。
- `runners/run-wasm-gc.mjs` が `spectest.print_char` import を提供して
  wasm を Node で実行、inspector の `Profiler.start`/`Profiler.stop` で
  V8 CPU profile (.cpuprofile) を取得。
- `runners/cpuprofile-to-pprof.mjs` (= `moonbit-pprof/cpuprofile-to-pprof`)
  が .cpuprofile → pprof protobuf に変換しつつ demangle。

### js (Node)

```sh
npm run build:js && npm run profile:js
```

moonbit の JS バックエンドはマングル名をそのまま JS 関数名として吐く
(`function _M0FP26mizchi5bench3fib(n) {...}`)。Node の inspector がそれを
そのまま CPU profile に入れるので、wasm-gc と同じ converter が使える。

### wasm (推奨: wasmtime + GuestProfiler)

```sh
npm run build:wasm && npm run profile:wasm
```

- wasmtime CLI の `--profile=guest` 相当を Rust API で組む。Cranelift JIT
  で wasm をフルスピード実行しつつ、別スレッドで `engine.increment_epoch()`
  を周期的に呼び、`epoch_deadline_callback` 内で `GuestProfiler::sample`
  が回る。
- `spectest.print_char` host import は `Linker::func_wrap` で実装。
- `GuestProfiler::finish` の出力 (Firefox Profiler JSON) を in-memory で
  受け取り、`firefox-to-pprof` crate で pprof + gzip を直接書き出す。
- ackermann(3, 10) は wasmtime のデフォルト wasm stack (512 KiB) を超える
  ので、`max_wasm_stack(8 MiB)` + `async_stack_size(16 MiB)` を上げてある。

### wasm (legacy: wzprof) ※非推奨

```sh
npm run profile:wasm:wzprof
```

最初のプロトタイプ。wazero インタプリタは Cranelift JIT より 3000× 以上遅く、
同じ workload で 14 分かかる上、per-call listener なので mandel_sum 等が
profile から消える。実用には wasmtime-runner を使う。

### native (samply 経由)

```sh
npm run build:native && npm run profile:native
```

- moonbit の native バックエンドは C にトランスパイルしてから clang で
  ビルド。シンボル名は wasm と同じマングル形式
  (`__M0FP26mizchi5bench9ackermann`)。
- samply で macOS / Linux のサンプリングプロファイルを取得 (Firefox
  Profiler 形式)。`--unstable-presymbolicate` で `.syms.json` サイドカーに
  シンボル情報を出す。
- `samply-to-pprof.mjs` がそれを読んで pprof に変換、インライン展開も処理。

## まとめ

| backend | profile source | サンプル方式 | pprof 化 |
|---------|---------------|---------|---------|
| wasm-gc | Node inspector (V8) | V8 sampling | `cpuprofile-to-pprof.mjs` |
| js      | Node inspector (V8) | V8 sampling | `cpuprofile-to-pprof.mjs` |
| wasm    | wasmtime GuestProfiler (Cranelift JIT) | epoch tick sampling | `firefox-to-pprof` crate / `wasmtime-to-pprof.mjs` |
| wasm (legacy) | wzprof (wazero interp) | per-call listener | wzprof 直接 + `pprof-demangle` |
| native  | samply (Mach-O / ELF) | OS sampling | `samply-to-pprof.mjs` |

どれもマングル名 (`_M0FP26mizchi5bench9ackermann` の類) を pprof に流せ、
共通の demangle 実装で `mizchi::bench::ackermann` に戻せる。

## ライブラリとして使う

3 言語ともプロジェクト外から取り込み可能:

### Rust

```toml
# Cargo.toml
[dependencies]
moonbit-demangle = "0.1"
firefox-to-pprof = "0.1"        # samply / wasmtime の JSON を pprof に
wasmtime-guest-pprof = "0.1"    # wasmtime app に直接組み込む
```

```rust
use moonbit_demangle::demangle;
assert_eq!(demangle("_M0FP26mizchi5bench9ackermann"), "mizchi::bench::ackermann");
```

### Go

```go
// go.mod
require github.com/mizchi/pprof-mbt/go/demangle v0.1.0

import "github.com/mizchi/pprof-mbt/go/demangle"
demangle.Symbol("_M0FP26mizchi5bench9ackermann")
// → "mizchi::bench::ackermann"
```

### JavaScript

```js
// package.json: "dependencies": { "moonbit-pprof": "^0.1" }
import { demangle } from "moonbit-pprof/demangle";
import { convert } from "moonbit-pprof/cpuprofile-to-pprof";
import { writePprofFromFirefox } from "moonbit-pprof/firefox-to-pprof";
```

## 制約 / 既知の TODO

- メモリプロファイルは未対応 (CPU のみ)。wzprof は `-memprofile` フラグを
  持っているが、本リポジトリの runner では未配線。
- demangle はヒューリスティック。impl / method / generic 修飾子
  (`_M0I…`, `_M0M…`, `GsE`/`GuE` 接尾辞) は `core::` プレフィックスを
  落とすなど不完全。
- llvm バックエンド (`moon build --target=llvm`) はビルドエラーで未検証。

## License

Apache-2.0
