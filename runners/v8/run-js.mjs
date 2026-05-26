// Run the JS backend output under Node with the inspector CPU profiler.
// Generates a .cpuprofile that the same converter can turn into pprof —
// the mangled symbol scheme is identical to the wasm-gc backend.
//
// Pass `--no-profile` to skip the inspector entirely for clean wall-time
// measurements.

import { Session } from "node:inspector/promises";
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve as resolvePath } from "node:path";
import { pathToFileURL } from "node:url";
import vm from "node:vm";
import { argv } from "node:process";

const positional = argv.slice(2).filter((a) => a !== "--no-profile");
const noProfile = argv.includes("--no-profile");

const jsPath = positional[0] ?? "bench/_build/js/release/build/cmd/main/main.js";
const profileOut = positional[1] ?? "js.cpuprofile";
const iterations = Number(positional[2] ?? 1);

// The moonbit js backend emits two flavors of code:
//   1) plain code (no `require()` calls) — works under dynamic import
//   2) CommonJS-style `const p = require("process")` for modules like @path
//      that need host platform info. ESM dynamic import rejects these.
//
// Detect (2) by scanning for a `require(` call in the source, and switch
// to the slower vm.Script path only when needed. The vm path runs in a
// separate context which V8 can't optimize as aggressively (~5-15x
// slower wall time), so we keep dynamic import as the default.
const source = readFileSync(jsPath, "utf8");
const usesRequire = /\brequire\s*\(/.test(source);

let runOnce;
if (usesRequire) {
  // createRequire needs an absolute file URL or absolute path — the
  // moonbit-emitted require() calls resolve relative to the .js file's
  // location, so anchor here rather than to the runner.
  const require = createRequire(pathToFileURL(resolvePath(jsPath)));
  const sandbox = {
    require,
    console,
    process,
    Buffer,
    TextEncoder,
    TextDecoder,
    performance,
  };
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  const script = new vm.Script(source, { filename: jsPath });
  console.error(`[js] using vm.Script fallback (file calls require()) — wall time will be slower`);
  runOnce = () => script.runInContext(sandbox);
} else {
  runOnce = (i) => import(`${jsPath}?i=${i}`); // cache-bust so the script runs each time
}

let session = null;
if (!noProfile) {
  session = new Session();
  session.connect();
  await session.post("Profiler.enable");
  await session.post("Profiler.start");
}

const t0 = performance.now();
for (let i = 0; i < iterations; i++) {
  await runOnce(i);
}
const elapsed = performance.now() - t0;

if (noProfile) {
  console.error(`[js] ${iterations} iter in ${elapsed.toFixed(1)} ms (no profile)`);
} else {
  const { profile } = await session.post("Profiler.stop");
  writeFileSync(profileOut, JSON.stringify(profile));
  session.disconnect();
  console.error(`[js] ${iterations} iter in ${elapsed.toFixed(1)} ms → ${profileOut}`);
}
