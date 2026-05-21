# pprof-mbt

MoonBit を 4 つのバックエンド (wasm-gc / wasm / js / native) でビルドし、それぞれの実行を pprof 形式でプロファイルできるか試した記録。

## セットアップ

```sh
nix develop
```

[moonbit-overlay](https://github.com/moonbit-community/moonbit-overlay) 経由で `moon`、その他 Node.js / Go (pprof CLI) / samply / gperftools / wabt が入る。

`wzprof-runner` と `pprof-demangle` は初回のみビルドが必要：

```sh
mkdir -p .bin
( cd runners/wzprof-runner && go build -buildvcs=false -o ../../.bin/wzprof-runner . )
( cd runners/wzprof-runner && go build -buildvcs=false -o ../../.bin/pprof-demangle ./cmd/demangle )
npm install
```

## ベンチコード

`bench/bench.mbt` に CPU バウンドな workload を 3 つ。`bench/cmd/main/main.mbt` がそれを呼ぶ:

- `ackermann(3, 10)` — 深い再帰
- `fib(32)` — 古典的再帰
- `mandel_sum(160, 500)` — 二重ループ＋浮動小数点

全バックエンド共通のコード。

## 各バックエンドでのプロファイル

### wasm-gc (V8 経由)

```sh
( cd bench && moon build --release --no-strip --target=wasm-gc cmd/main )
node runners/run-wasm-gc.mjs bench/_build/wasm-gc/release/build/cmd/main/main.wasm wasm-gc.cpuprofile 5
node runners/cpuprofile-to-pprof.mjs wasm-gc.cpuprofile wasm-gc.pb.gz
go tool pprof -http :8000 wasm-gc.pb.gz
```

仕組み:

- `moon build --no-strip` で wasm に関数名 (`_M0FP26mizchi5bench9ackermann` 等) を保持。
- `run-wasm-gc.mjs` が `spectest.print_char` import を提供して wasm を Node で実行、Node の inspector の `Profiler.start`/`Profiler.stop` で V8 CPU profile (.cpuprofile) を取得。
- `cpuprofile-to-pprof.mjs` が `.cpuprofile` → pprof protobuf に変換しつつ、moonbit のマングル名を可読化。

サンプル出力:

```
77.84%  mizchi::bench::ackermann
11.29%  mizchi::bench::mandel__sum
 9.30%  mizchi::bench::mandel__point
 8.30%  mizchi::bench::fib
```

### js (Node)

```sh
( cd bench && moon build --release --target=js cmd/main )
node runners/run-js.mjs "$(pwd)/bench/_build/js/release/build/cmd/main/main.js" js.cpuprofile
node runners/cpuprofile-to-pprof.mjs js.cpuprofile js.pb.gz
```

JS バックエンドは moonbit の mangled シンボルをそのまま JS 関数名として吐く (`function _M0FP26mizchi5bench3fib(n) {...}`)。Node の inspector がそれを CPU profile にそのまま入れるので、同じ変換スクリプトが使える。

### wasm (MVP, wzprof)

```sh
( cd bench && moon build --release --no-strip --target=wasm cmd/main )
.bin/wzprof-runner -cpuprofile wasm.pb.gz -sample 0.05 bench/_build/wasm/release/build/cmd/main/main.wasm
.bin/pprof-demangle wasm.pb.gz wasm.demangled.pb.gz
```

仕組み:

- 公式 `wzprof` CLI は WASI モジュールを前提にしてるが、moonbit の wasm 出力は `spectest.print_char` (moonrun の慣習) しか import しない。
- `runners/wzprof-runner/` で wzprof をライブラリとして使い、その import を自前で実装し、`CPUProfiler` で pprof 出力。
- wzprof 出力はマングル名のままなので `.bin/pprof-demangle` で再書き込み。

注意: wazero インタプリタは V8/JIT より大幅に遅い (デフォルト workload で 10〜15 分かかる)。実用には workload を小さくするか sample レートを下げる。

### native (samply 経由)

```sh
( cd bench && moon build --release --target=native cmd/main )
samply record --save-only --unstable-presymbolicate -o native-samply.json.gz --no-open \
  -- bench/_build/native/release/build/cmd/main/main.exe
node runners/samply-to-pprof.mjs native-samply.json.gz native-samply.json.syms.json native.pb.gz
```

仕組み:

- moonbit の native バックエンドは C にトランスパイルしてから clang でビルド (`main.exe`)。シンボル名は wasm と同じマングル形式 (`__M0FP26mizchi5bench9ackermann`)。
- samply で macOS / Linux のサンプリングプロファイルを取得 (Firefox Profiler 形式)。`--unstable-presymbolicate` で `.syms.json` サイドカーにシンボル情報を出す。
- `samply-to-pprof.mjs` がそれを読んで pprof に変換、インライン展開も処理する。

別ルートとして gperftools の `libprofiler.dylib` を `DYLD_INSERT_LIBRARIES` で注入する方法も試したが、macOS では recorded address と mapping range がズレてシンボル解決できなかった。samply の方が安定。

## まとめ

| backend | profile source | tool chain | pprof 化 | symbol 解決 |
|---------|---------------|------------|---------|-----------|
| wasm-gc | Node inspector (V8) | `WebAssembly.instantiate` + `Profiler.start` | `cpuprofile-to-pprof.mjs` | wasm の `name` custom section + 自前 demangle |
| js      | Node inspector (V8) | dynamic `import()` + `Profiler.start` | `cpuprofile-to-pprof.mjs` | JS 関数名 (= マングル名) + 自前 demangle |
| wasm    | wzprof (wazero) | カスタム host w/ spectest import | wzprof が直接 pprof | wasm `name` section + 自前 demangle |
| native  | samply (Mach-O / ELF) | プロセス attach サンプリング | `samply-to-pprof.mjs` | samply の presymbolicate sidecar + 自前 demangle |

どのバックエンドでもマングル名 (`_M0FP26mizchi5bench9ackermann` の類) を pprof に流せ、共通の demangle ルーチンで `mizchi::bench::ackermann` に戻せた。pprof 上でホットスポット (ackermann) が同じように識別できる。

### 制約 / TODO

- メモリプロファイルは未対応 (CPU のみ)。wzprof は `-memprofile` フラグを持っているが、本リポジトリの runner では未配線。
- demangle はヒューリスティック。impl / method / generic 修飾子 (`_M0I…`, `_M0M…`, `GsE`/`GuE` 接尾辞) は core プレフィックスを落とすなど不完全。
- llvm バックエンド (`moon build --target=llvm`) はビルドエラーで未検証。
