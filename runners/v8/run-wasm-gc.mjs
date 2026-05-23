// Run a MoonBit wasm-gc binary under Node and (by default) collect a V8
// CPU profile via the inspector. Pass `--no-profile` to skip the profiler
// — useful for clean wall-time measurements without the inspector
// overhead.
//
// The host imports themselves live in `@mizchi/moonbit-wasm-host` so
// non-moonbit consumers can swap in their own.

import { readFileSync, writeFileSync } from "node:fs";
import { Session } from "node:inspector/promises";
import { argv } from "node:process";
import { moonbitWasmImports, autoStubMissing } from "@mizchi/moonbit-wasm-host";

// Strip --no-profile from argv first so positional indices stay stable.
const positional = argv.slice(2).filter((a) => a !== "--no-profile");
const noProfile = argv.includes("--no-profile");

const wasmPath = positional[0] ?? "bench/_build/wasm-gc/release/build/cmd/main/main.wasm";
const profileOut = positional[1] ?? "wasm-gc.cpuprofile";
const iterations = Number(positional[2] ?? 1);

const bytes = readFileSync(wasmPath);
const mod = await WebAssembly.compile(bytes);

const imports = moonbitWasmImports();
const stubbed = autoStubMissing(imports, mod);
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
