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
};

const mod = await WebAssembly.compile(bytes);
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
