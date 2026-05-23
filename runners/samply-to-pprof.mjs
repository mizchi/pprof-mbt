// Convert samply's Firefox-Profiler JSON (+ .syms.json sidecar) into pprof.
//
// samply stores raw RVAs in frameTable.address and ships symbol info in a
// separate .syms.json sidecar. For each address we binary-search the lib's
// symbol_table for the enclosing function, and expand inline frames so a
// hot pc inside `main` is attributed to mandel_point / mandel_sum / main
// rather than just main. The rest of the pprof construction is in
// runners/lib/firefox-to-pprof.mjs.

import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { writePprofFromFirefox } from "@mizchi/pprof-tools/firefox-to-pprof";

const [, , inPath = "native-samply.json.gz", symsPath = inPath.replace(/\.gz$/, "") + ".syms.json", outPath = "native.pb.gz"] =
  process.argv;

const raw = readFileSync(inPath);
const profile = JSON.parse(
  (inPath.endsWith(".gz") ? gunzipSync(raw) : raw).toString("utf8"),
);
const syms = JSON.parse(readFileSync(symsPath, "utf8"));

// Index syms by debugName so we can dispatch on the lib that owns a frame.
const symsByDebugName = new Map();
for (const lib of syms.data) {
  const table = lib.symbol_table.slice().sort((a, b) => a.rva - b.rva);
  symsByDebugName.set(lib.debug_name, { table, stringTable: syms.string_table });
}

function lookupSymbol(libDebugName, rva) {
  const entry = symsByDebugName.get(libDebugName);
  if (!entry) return null;
  const { table, stringTable: st } = entry;
  // Binary search for the largest table[i].rva <= rva.
  let lo = 0, hi = table.length - 1, best = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (table[mid].rva <= rva) {
      best = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  if (best < 0) return null;
  const sym = table[best];
  if (rva >= sym.rva + sym.size) return null;
  // samply's `frames` lists outer→inner; reverse so the leaf is first
  // (pprof's Location.line[] expects leaf first).
  return sym.frames
    ? sym.frames.slice().reverse().map((f) => ({
        name: st[f.function] ?? st[sym.symbol],
        file: f.file !== undefined ? st[f.file] : "",
        line: f.line ?? 0,
      }))
    : [{ name: st[sym.symbol], file: "", line: 0 }];
}

writePprofFromFirefox(
  {
    profile,
    label: "samply→pprof",
    resolveFrame(thread, frameIdx) {
      const { frameTable, funcTable, resourceTable } = thread;
      const addr = frameTable.address[frameIdx];
      const funcId = frameTable.func[frameIdx];
      const res = funcTable.resource[funcId];
      const libIndex = res >= 0 ? resourceTable.lib[res] ?? 0 : 0;
      const lib = profile.libs?.[libIndex];
      const frames = lib ? lookupSymbol(lib.debugName, addr) : null;
      if (frames) {
        return frames.map((f) => ({ ...f, address: addr, mappingIndex: libIndex }));
      }
      return [
        {
          name: `${lib?.name ?? "??"}+0x${(addr ?? 0).toString(16)}`,
          file: "",
          line: 0,
          address: addr,
          mappingIndex: libIndex,
        },
      ];
    },
    resolveSample(thread, i) {
      // samply runs at a fixed rate (default 1 kHz) — meta.interval is in ms.
      const intervalMs = profile.meta?.interval ?? 1;
      const count = thread.samples.weight?.[i] ?? 1;
      return {
        stack: thread.samples.stack[i],
        count,
        ns: count * Math.round(intervalMs * 1_000_000),
      };
    },
  },
  outPath,
);
