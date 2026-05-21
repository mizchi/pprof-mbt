// Host shim for moonbit's wasm-gc output.
// Mirrors the `moonrun` host: provides `spectest.print_char` for println.
// Optionally collects a V8 CPU profile via the inspector and writes
// `wasm-gc.cpuprofile` so it can be loaded into Chrome DevTools or
// converted to pprof.

import { readFileSync, writeFileSync } from "node:fs";
import { Session } from "node:inspector/promises";
import { argv } from "node:process";

const wasmPath = argv[2] ?? "bench/_build/wasm-gc/release/build/cmd/main/main.wasm";
const profileOut = argv[3] ?? "wasm-gc.cpuprofile";
const iterations = Number(argv[4] ?? 1);

const bytes = readFileSync(wasmPath);
const utf8 = new TextDecoder("utf-16le"); // moonbit emits UTF-16 code units one byte at a time, two-byte pairs

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

const session = new Session();
session.connect();
await session.post("Profiler.enable");
await session.post("Profiler.start");

const t0 = performance.now();
for (let i = 0; i < iterations; i++) {
  instance.exports._start();
}
const elapsed = performance.now() - t0;

const { profile } = await session.post("Profiler.stop");
writeFileSync(profileOut, JSON.stringify(profile));
session.disconnect();

console.error(`[wasm-gc] ${iterations} iter in ${elapsed.toFixed(1)} ms → ${profileOut}`);
