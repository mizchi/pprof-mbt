# moonbit-demangle (Go)

Decode [MoonBit](https://www.moonbitlang.com/) symbol mangling into a
readable `package::module::function` path.

```go
import "github.com/mizchi/pprof-mbt/go/demangle"

demangle.Symbol("_M0FP26mizchi5bench9ackermann") // -> "mizchi::bench::ackermann"
demangle.Symbol("main")                          // -> "main" (passthrough)
```

Companion implementations live in:

- [`crates/moonbit-demangle`](../../crates/moonbit-demangle) — Rust
- [`packages/moonbit-pprof`](../../packages/moonbit-pprof) — JS (`./demangle` export)

All three speak the same backward-scanning algorithm, so output is
consistent across languages.

## License

Apache-2.0
