// Convert wasmtime's GuestProfiler JSON (Firefox processed-profile format)
// into pprof. Function names are already resolved in funcTable, so frame
// resolution is a direct index lookup. The rest of the pprof construction
// is shared with samply-to-pprof.mjs via runners/lib/firefox-to-pprof.mjs.

import { readFileSync } from "node:fs";
import { writePprofFromFirefox } from "@mizchi/pprof-tools/firefox-to-pprof";

const [, , inPath = "wasmtime-guest.json", outPath = "wasmtime-guest.pb.gz"] = process.argv;
const profile = JSON.parse(readFileSync(inPath, "utf8"));

writePprofFromFirefox(
  {
    profile,
    label: "wasmtime→pprof",
    resolveFrame(thread, frameIdx) {
      const { frameTable, funcTable, stringArray } = thread;
      const funcIdx = frameTable.func[frameIdx];
      const name = stringArray[funcTable.name[funcIdx]] || "(anonymous)";
      const fileSlot = funcTable.fileName?.[funcIdx];
      const file = fileSlot != null && fileSlot >= 0 ? stringArray[fileSlot] || "" : "";
      const addr = frameTable.address?.[frameIdx];
      return [
        {
          name,
          file,
          line: frameTable.line?.[frameIdx] ?? 0,
          address: addr != null && addr >= 0 ? addr : 0,
          mappingIndex: 0,
        },
      ];
    },
    resolveSample(thread, i) {
      // wasmtime records the real elapsed time between samples in `timeDeltas`
      // (ms). Fall back to the nominal interval if it's missing.
      const intervalMs = profile.meta?.interval ?? 1;
      const dt = thread.samples.timeDeltas?.[i] ?? intervalMs;
      const count = thread.samples.weight?.[i] ?? 1;
      return {
        stack: thread.samples.stack[i],
        count,
        ns: Math.round(dt * 1_000_000),
      };
    },
  },
  outPath,
);
