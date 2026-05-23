# `moonbitlang/x` 向け PR ドラフト

`bench-x/` で取った callgrind プロファイルから出たパッチを upstream
[`moonbitlang/x`](https://github.com/moonbitlang/x) に出す素材。`notes/pr-drafts/`
(core)、`notes/async-pr-drafts/` (async) と同じ形式。

## 一覧

| # | ブランチ名 | 主な効果 | 行数 |
|---|---|---|--:|
| 01 | `pr-json5-lex-number-lazy-tostring` | json5_parse -5.2% (parse_double に StringView 直渡し、error 時のみ to_string) | +9/-3 |
| 02 | `pr-base64-index-loop` | base64_encode -20.6% / base64_decode **-36.9%** (iter overhead 除去) | +33/-3 |
| 03 | `pr-encoding-utf8-code-unit-walk` | encoding_utf8 **-22.7%** (`for char in src` を code-unit walk に書き換え) | +22/-3 |
| 04 | `pr-uuid-tostring-inplace` | uuid_parse **-64.1%** (`Bytes::from_array(Array::from_fixed_array(rv))` 2 段 copy を `unsafe_reinterpret_as_bytes` 1 段に) | +5/-1 |

**`moonbitlang/core` PR-01 (bigint mul_single_limb) が moonbitlang/x の decimal_arith にも cascade**: decimal の `factorial` 系チェーンは BigInt × 1-limb の連鎖なので、別ベンチで **-72%** (170→47ms) を確認。x 側に新規パッチは要らない。

詳細な調査ログ: `notes/x_investigation.md`

## 出し方

```sh
git clone git@github.com:<your-fork>/x.git
cd x
git remote add upstream https://github.com/moonbitlang/x.git
git fetch upstream

git checkout -b pr-json5-lex-number-lazy-tostring upstream/main
git am < /path/to/pprof-mbt/notes/x-pr-drafts/01-json5-lex-number-lazy-tostring/0001-json5-lex-number-lazy-tostring.patch

moon fmt
moon test --target native -p moonbitlang/x/json5

git push -u origin pr-json5-lex-number-lazy-tostring
gh pr create --repo moonbitlang/x \
  --title "$(cat /path/to/pprof-mbt/notes/x-pr-drafts/01-json5-lex-number-lazy-tostring/title.txt)" \
  --body-file /path/to/pprof-mbt/notes/x-pr-drafts/01-json5-lex-number-lazy-tostring/body.md
```

## 試して落としたもの (記録のみ)

- **crypto/utils.mbt の `uint32` を `Byte::to_uint` 直呼び + `#inline`**:
  `u8_to_u32le` / `bytes_u8_to_u32be` が SHA-256 / MD5 で 11〜24%
  占めていたので micro-optimize を試みたが効果ゼロ。moonc がすでに
  `to_int().reinterpret_as_uint()` を `to_uint()` と同等にコンパイル
  していた。同じパターンの core Hasher `#inline` 実験 (no-op) と一致。
