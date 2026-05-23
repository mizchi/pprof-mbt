// Host shim for moonbit's wasm-gc output.
// Mirrors the `moonrun` host: provides `spectest.print_char` for println.
// By default also collects a V8 CPU profile via the inspector. Pass
// `--no-profile` to skip the profiler — useful for clean wall-time
// measurements without the inspector overhead.

import { readFileSync, writeFileSync } from "node:fs";
import { Session } from "node:inspector/promises";
import { argv } from "node:process";

// Strip --no-profile from argv first so positional indices stay stable.
const positional = argv.slice(2).filter((a) => a !== "--no-profile");
const noProfile = argv.includes("--no-profile");

const wasmPath = positional[0] ?? "bench/_build/wasm-gc/release/build/cmd/main/main.wasm";
const profileOut = positional[1] ?? "wasm-gc.cpuprofile";
const iterations = Number(positional[2] ?? 1);

const bytes = readFileSync(wasmPath);

let charBuf = [];
const imports = {
  spectest: {
    // moonrun emits UTF-16 code units; print_char is called once per code unit.
    print_char: (code) => {
      if (code === 10) {
        const text = String.fromCharCode(...charBuf);
        process.stdout.write(text + "\n");
        charBuf = [];
      } else {
        charBuf.push(code);
      }
    },
  },
  // Used by moonbitlang/x's @path / @fs etc. The set of names is small
  // and stable; we only need to satisfy the linker.
  __moonbit_sys_unstable: {
    is_windows: () => (process.platform === "win32" ? 1 : 0),
  },
};

// Resolve any imports we don't already cover with a noop stub of the
// right arity, so the bench can link even if the module pulls in new
// FFI shims we haven't seen yet. Logs the stub list to stderr so the
// reader knows which calls are returning fake values.
const mod = await WebAssembly.compile(bytes);
const stubbed = [];
for (const imp of WebAssembly.Module.imports(mod)) {
  if (imports[imp.module]?.[imp.name] !== undefined) continue;
  imports[imp.module] ??= {};
  imports[imp.module][imp.name] = () => 0;
  stubbed.push(`${imp.module}.${imp.name}`);
}
if (stubbed.length > 0) {
  console.error(`[wasm-gc] stubbed imports: ${stubbed.join(", ")}`);
}

const instance = await WebAssembly.instantiate(mod, imports);

let session = null;
if (!noProfile) {
  session = new Session();
  session.connect();
  await session.post("Profiler.enable");
  await session.post("Profiler.start");
}

const t0 = performance.now();
for (let i = 0; i < iterations; i++) {
  instance.exports._start();
}
const elapsed = performance.now() - t0;

if (noProfile) {
  console.error(`[wasm-gc] ${iterations} iter in ${elapsed.toFixed(1)} ms (no profile)`);
} else {
  const { profile } = await session.post("Profiler.stop");
  writeFileSync(profileOut, JSON.stringify(profile));
  session.disconnect();
  console.error(`[wasm-gc] ${iterations} iter in ${elapsed.toFixed(1)} ms → ${profileOut}`);
}
