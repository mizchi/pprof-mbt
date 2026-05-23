// Host imports for running a MoonBit-compiled wasm-gc / wasm guest in
// Node (or any V8-style WebAssembly host). Mirrors the moonrun host:
//
//   - spectest.print_char(code: i32)
//       Called once per UTF-16 code unit by MoonBit's println. Flushes
//       a line to stdout on '\n'.
//
//   - __moonbit_sys_unstable.is_windows() -> i32
//       Used by moonbitlang/x's @path / @fs / @sys. Returns 1 on win32,
//       0 elsewhere.
//
// Use alongside `autoStubMissing(imports, mod)` to satisfy any other
// imports the moonbit compiler adds in the future with a noop stub of
// arity 0, so the bench at least links.

/**
 * Build a fresh moonbit host imports object. Returns an object suitable
 * to pass to `WebAssembly.instantiate(mod, ...)`.
 *
 * @param {object} [opts]
 * @param {(text: string) => void} [opts.writeLine]
 *   Where to send each flushed line. Defaults to `process.stdout.write(text + "\n")`.
 * @param {boolean} [opts.isWindows]
 *   Force the value returned by `__moonbit_sys_unstable.is_windows`.
 *   Defaults to `process.platform === "win32"`.
 */
export function moonbitWasmImports(opts = {}) {
  const writeLine =
    opts.writeLine ?? ((text) => process.stdout.write(text + "\n"));
  const isWindows = opts.isWindows ?? (process.platform === "win32");

  let charBuf = [];
  return {
    spectest: {
      print_char: (code) => {
        if (code === 10) {
          writeLine(String.fromCharCode(...charBuf));
          charBuf = [];
        } else {
          charBuf.push(code);
        }
      },
    },
    __moonbit_sys_unstable: {
      is_windows: () => (isWindows ? 1 : 0),
    },
  };
}

/**
 * For every import in `mod` that doesn't already have a binding in
 * `imports`, inject a noop function returning 0. Returns the list of
 * names that were stubbed (so the caller can log a warning).
 *
 * Generic: works for any wasm module, not moonbit-specific.
 *
 * @param {Record<string, Record<string, any>>} imports - mutated in place
 * @param {WebAssembly.Module} mod
 * @returns {string[]} stubbed names in `<module>.<name>` form
 */
export function autoStubMissing(imports, mod) {
  const stubbed = [];
  for (const imp of WebAssembly.Module.imports(mod)) {
    if (imports[imp.module]?.[imp.name] !== undefined) continue;
    imports[imp.module] ??= {};
    imports[imp.module][imp.name] = () => 0;
    stubbed.push(`${imp.module}.${imp.name}`);
  }
  return stubbed;
}
