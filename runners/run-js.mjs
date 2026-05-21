// Run the JS backend output under Node with the inspector CPU profiler.
// Generates a .cpuprofile that the same converter can turn into pprof —
// the mangled symbol scheme is identical to the wasm-gc backend.

import { Session } from "node:inspector/promises";
import { writeFileSync } from "node:fs";
import { argv } from "node:process";

const jsPath = argv[2] ?? "bench/_build/js/release/build/cmd/main/main.js";
const profileOut = argv[3] ?? "js.cpuprofile";
const iterations = Number(argv[4] ?? 1);

const session = new Session();
session.connect();
await session.post("Profiler.enable");
await session.post("Profiler.start");

const t0 = performance.now();
for (let i = 0; i < iterations; i++) {
  await import(`${jsPath}?i=${i}`); // cache-bust so the script runs each time
}
const elapsed = performance.now() - t0;

const { profile } = await session.post("Profiler.stop");
writeFileSync(profileOut, JSON.stringify(profile));
session.disconnect();

console.error(`[js] ${iterations} iter in ${elapsed.toFixed(1)} ms → ${profileOut}`);
