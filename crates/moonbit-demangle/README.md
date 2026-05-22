# moonbit-demangle

Decode [MoonBit](https://www.moonbitlang.com/) symbol mangling into a
readable `package::module::function` path.

```rust
use moonbit_demangle::demangle;

assert_eq!(
    demangle("_M0FP26mizchi5bench9ackermann"),
    "mizchi::bench::ackermann",
);
```

## What it handles

- User functions: `_M0FP26mizchi5bench9ackermann` → `mizchi::bench::ackermann`
- Double-underscore names from `_` → `__`: `mandel__sum` survives intact
- Mach-O leading underscore (`__M0F…`) and samply's stripped form (`M0F…`)
- Trailing generic markers (`GsE`, `GuE`) get dropped

## What it doesn't (yet)

Impl / method / trait decorations are partially decoded. For example
`_M0IPC13int3IntPB4Show10to__string` becomes `int::Int::Show::to_string`
instead of `core::int::Int::Show::to_string`. The structural prefix
markers (`PC`, `PB`) aren't fully parsed.

This crate is heuristic-only because MoonBit's mangling scheme isn't
published. If you find a symbol that misbehaves, please file an issue
with the input + expected output.

## License

Apache-2.0
