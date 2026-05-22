# pprof-mbt

MoonBit を 4 つのバックエンド (wasm-gc / wasm / js / native) でビルドし、それぞれの実行を pprof 形式でプロファイルできるか試した記録。

## セットアップ

```sh
nix develop
```

[moonbit-overlay](https://github.com/moonbit-community/moonbit-overlay) 経由で `moon`、その他 Node.js / Go (pprof CLI) / samply / gperftools / wabt が入る。

ネイティブ製の runner は初回のみビルドが必要：

```sh
mkdir -p .bin
# wasm 経路 (推奨): wasmtime + GuestProfiler
( cd runners/wasmtime-runner && cargo build --release )
cp runners/wasmtime-runner/target/release/wasmtime-runner .bin/

# wasm 経路 (legacy): wzprof (15 分かかるので参考用)
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

### wasm (推奨: wasmtime + GuestProfiler)

```sh
( cd bench && moon build --release --no-strip --target=wasm cmd/main )
( cd runners/wasmtime-runner && cargo build --release )
cp runners/wasmtime-runner/target/release/wasmtime-runner .bin/
.bin/wasmtime-runner --interval-us 1000 --out wasmtime-guest.pb.gz \
  bench/_build/wasm/release/build/cmd/main/main.wasm
# 必要なら --json-out wasmtime-guest.json で Firefox Profiler JSON も併出力
```

仕組み:

- wasmtime CLI の `--profile=guest` 相当を Rust API で組む。Cranelift JIT で wasm をフルスピード実行しつつ、別スレッドで `engine.increment_epoch()` を周期的に呼び、`epoch_deadline_callback` 内で `GuestProfiler::sample` が回る。
- `spectest.print_char` host import は `Linker::func_wrap` で実装。
- `GuestProfiler::finish` の出力 (Firefox Profiler JSON) を in-memory で受け取り、`prost` 経由で pprof protobuf に変換 + gzip して直接書き出す。Node の変換ステップは不要。
- ackermann(3,10) は wasmtime のデフォルト wasm stack (512 KiB) を超えるので、`max_wasm_stack(8 MiB)` + `async_stack_size(16 MiB)` を上げてある。

実行時間: V8/native と同等 (デフォルト workload で 250ms 程度)。

### wasm (legacy: wzprof) ※非推奨

`runners/wzprof-runner/` は最初のプロトタイプとして残してある。wazero インタプリタは Cranelift JIT より 3000x 以上遅く、同じ workload で 14 分かかる。`--sample 0.05` でサンプリング率を落とせばマシだが、結局 mandel_sum 等が profile から消えるなど精度も落ちる。実用には wasmtime-runner を使う。

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
| wasm    | wasmtime GuestProfiler (Cranelift JIT) | `epoch_deadline_callback` + `GuestProfiler::sample` | Rust 側で `prost` で直接 pprof | wasm `name` section + 自前 demangle |
| wasm (legacy) | wzprof (wazero interpreter) | カスタム host w/ spectest import | wzprof が直接 pprof | wasm `name` section + Go 製 demangler |
| native  | samply (Mach-O / ELF) | プロセス attach サンプリング | `samply-to-pprof.mjs` | samply の presymbolicate sidecar + 自前 demangle |

どのバックエンドでもマングル名 (`_M0FP26mizchi5bench9ackermann` の類) を pprof に流せ、共通の demangle ルーチンで `mizchi::bench::ackermann` に戻せた。pprof 上でホットスポット (ackermann) が同じように識別できる。

### 制約 / TODO

- メモリプロファイルは未対応 (CPU のみ)。wzprof は `-memprofile` フラグを持っているが、本リポジトリの runner では未配線。
- demangle はヒューリスティック。impl / method / generic 修飾子 (`_M0I…`, `_M0M…`, `GsE`/`GuE` 接尾辞) は core プレフィックスを落とすなど不完全。
- llvm バックエンド (`moon build --target=llvm`) はビルドエラーで未検証。
