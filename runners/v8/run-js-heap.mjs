// Run the JS backend output under Node with the V8 sampling allocation
// profiler. Emits a `.heapprofile` JSON (HeapProfiler.SamplingHeapProfile)
// that `moon-pprof heapprofile2pprof` turns into pprof with
// alloc_objects / alloc_space sample types.
//
// Pass `--no-profile` to skip the inspector entirely if you just want
// to time the workload.
//
// Pass `--interval <bytes>` to override the sampling interval (V8
// default is 16 KiB).

import { Session } from "node:inspector/promises";
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import vm from "node:vm";
import { argv } from "node:process";

const flags = new Set(argv.filter((a) => a.startsWith("--")));
const noProfile = flags.has("--no-profile");
const positional = argv.slice(2).filter((a, i, all) => {
  if (!a.startsWith("--")) {
    // skip values that follow a known value-taking flag
    const prev = all[i - 1];
    if (prev === "--interval") return false;
    return true;
  }
  return false;
});

const intervalIdx = argv.indexOf("--interval");
const samplingInterval =
  intervalIdx >= 0 ? Number(argv[intervalIdx + 1]) : 16 * 1024;

const jsPath = positional[0] ?? "bench/_build/js/release/build/cmd/main/main.js";
const profileOut = positional[1] ?? "js.heapprofile";
const iterations = Number(positional[2] ?? 1);

// Mirror runners/v8/run-js.mjs: dynamic import is the fast path, but
// the moonbit js backend sometimes emits `require()` for modules like
// @path that need host platform info. Fall back to vm.Script when we
// see one.
const source = readFileSync(jsPath, "utf8");
const usesRequire = /\brequire\s*\(/.test(source);

let runOnce;
if (usesRequire) {
  const require = createRequire(jsPath);
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
  console.error(`[js-heap] using vm.Script fallback (file calls require()) — wall time will be slower`);
  runOnce = () => script.runInContext(sandbox);
} else {
  runOnce = (i) => import(`${jsPath}?i=${i}`); // cache-bust so the script runs each time
}

let session = null;
if (!noProfile) {
  session = new Session();
  session.connect();
  await session.post("HeapProfiler.enable");
  await session.post("HeapProfiler.startSampling", {
    samplingInterval,
    includeObjectsCollectedByMajorGC: true,
    includeObjectsCollectedByMinorGC: true,
  });
}

const t0 = performance.now();
for (let i = 0; i < iterations; i++) {
  await runOnce(i);
}
const elapsed = performance.now() - t0;

if (noProfile) {
  console.error(`[js-heap] ${iterations} iter in ${elapsed.toFixed(1)} ms (no profile)`);
} else {
  const { profile } = await session.post("HeapProfiler.stopSampling");
  writeFileSync(profileOut, JSON.stringify(profile));
  session.disconnect();
  console.error(
    `[js-heap] ${iterations} iter in ${elapsed.toFixed(1)} ms → ${profileOut} (interval=${samplingInterval}B)`,
  );
}
