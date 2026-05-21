// Convert samply's Firefox-Profiler JSON (+ .syms.json sidecar) into pprof.
// Required so the native-backend profile can be inspected with the same
// `go tool pprof` tooling as the wasm-gc / js paths.
//
// Inlining: we expand the inline-frame chain reported by samply's
// presymbolicate sidecar, so a hot address inside main gets attributed
// to mandel_point / mandel_sum / main instead of just main.

import { readFileSync, writeFileSync } from "node:fs";
import { gunzipSync, gzipSync } from "node:zlib";
import {
  Profile,
  Sample,
  Location,
  Line,
  Function as PFunction,
  Mapping,
  ValueType,
  StringTable,
} from "pprof-format";

const [, , inPath = "native-samply.json.gz", symsPath = inPath.replace(/\.gz$/, "") + ".syms.json", outPath = "native.pb.gz"] =
  process.argv;

const buf = readFileSync(inPath);
const raw = inPath.endsWith(".gz") ? gunzipSync(buf) : buf;
const j = JSON.parse(raw.toString("utf8"));
const syms = JSON.parse(readFileSync(symsPath, "utf8"));

// Index syms by debugName so we can look up by lib name.
const symsByDebugName = new Map();
for (const lib of syms.data) {
  // Build a sorted symbol table for binary search on rva.
  const table = lib.symbol_table.slice().sort((a, b) => a.rva - b.rva);
  symsByDebugName.set(lib.debug_name, { table, stringTable: syms.string_table });
}

function lookupSymbol(libName, rva) {
  const entry = symsByDebugName.get(libName);
  if (!entry) return null;
  const { table, stringTable } = entry;
  // Binary search for largest table[i].rva <= rva
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
  // Build the inline chain (innermost first). samply's `frames` lists
  // outer→inner (callee at end), so we reverse for leaf-first.
  const frames = sym.frames
    ? sym.frames.slice().reverse().map((f) => ({
        name: stringTable[f.function] ?? stringTable[sym.symbol],
        file: f.file !== undefined ? stringTable[f.file] : "",
        line: f.line ?? 0,
      }))
    : [{ name: stringTable[sym.symbol], file: "", line: 0 }];
  return frames;
}

const stringTable = new StringTable();
const sampleTypes = [
  new ValueType({ type: stringTable.dedup("samples"), unit: stringTable.dedup("count") }),
  new ValueType({ type: stringTable.dedup("cpu"), unit: stringTable.dedup("nanoseconds") }),
];
const periodType = new ValueType({
  type: stringTable.dedup("cpu"),
  unit: stringTable.dedup("nanoseconds"),
});

// One mapping per lib; pprof requires at least one.
const mappings = [];
const mappingIdByLibIndex = new Map();
for (let i = 0; i < j.libs.length; i++) {
  const lib = j.libs[i];
  const m = new Mapping({
    id: BigInt(i + 1),
    filename: stringTable.dedup(lib.name),
  });
  mappings.push(m);
  mappingIdByLibIndex.set(i, m.id);
}

function demangle(name) {
  if (!name) return name;
  // Mach-O carries a leading underscore (`_M0F…`); samply's inline-frame
  // `function` strings strip it (`M0F…`). Normalise before parsing.
  const match = name.match(/^_*(M0[A-Z].*)$/);
  if (!match) return name;
  const inner = match[1];
  const stripped = inner.replace(/G[A-Za-z]+E$/, "");
  const parts = [];
  let i = stripped.length;
  for (let guard = 0; guard < 50 && i > 0; guard++) {
    let found = null;
    for (let n = Math.min(i - 1, 64); n >= 1; n--) {
      const chars = stripped.slice(i - n, i);
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(chars)) continue;
      const dEnd = i - n;
      let dStart = dEnd;
      while (dStart > 0 && /\d/.test(stripped[dStart - 1])) dStart--;
      if (dStart === dEnd) continue;
      const target = String(n);
      for (let ds = dStart; ds < dEnd; ds++) {
        if (stripped.slice(ds, dEnd) === target) {
          found = { chars, newI: ds };
          break;
        }
      }
      if (found) break;
    }
    if (!found) break;
    parts.unshift(found.chars);
    i = found.newI;
  }
  return parts.length ? parts.join("::") : name;
}

const functions = [];
const locations = [];
const funcIdByKey = new Map();

function getFunctionId(rawName, file) {
  const pretty = demangle(rawName);
  const key = `${rawName}|${file}`;
  let id = funcIdByKey.get(key);
  if (id !== undefined) return id;
  id = BigInt(functions.length + 1);
  functions.push(
    new PFunction({
      id,
      name: stringTable.dedup(pretty),
      systemName: stringTable.dedup(rawName),
      filename: stringTable.dedup(file),
    }),
  );
  funcIdByKey.set(key, id);
  return id;
}

const locIdByKey = new Map();
function getLocationId(libIndex, rva) {
  const key = `${libIndex}:${rva}`;
  let id = locIdByKey.get(key);
  if (id !== undefined) return id;
  id = BigInt(locations.length + 1);
  const lib = j.libs[libIndex];
  const frames = lib ? lookupSymbol(lib.debugName, rva) : null;
  const lines = frames
    ? frames.map(
        (fr) =>
          new Line({
            functionId: getFunctionId(fr.name, fr.file),
            line: fr.line,
          }),
      )
    : [
        new Line({
          functionId: getFunctionId(`${lib?.name ?? "??"}+0x${rva.toString(16)}`, ""),
          line: 0,
        }),
      ];
  locations.push(
    new Location({
      id,
      mappingId: mappingIdByLibIndex.get(libIndex) ?? 1n,
      address: BigInt(rva),
      line: lines,
    }),
  );
  locIdByKey.set(key, id);
  return id;
}

// Build samples: each thread's samples reference stackTable[stack] which is
// a linked list of frames (leaf first). frame.address is RVA inside frame's lib.
const samples = [];
for (const t of j.threads) {
  const { frameTable, stackTable, samples: sm, resourceTable, funcTable } = t;
  const sampleCount = sm.length;
  const stackToLocs = new Map();
  function locsForStack(stackId) {
    if (stackId === null || stackId === undefined) return [];
    if (stackToLocs.has(stackId)) return stackToLocs.get(stackId);
    const acc = [];
    let s = stackId;
    while (s !== null && s !== undefined) {
      const frameId = stackTable.frame[s];
      const addr = frameTable.address[frameId];
      const funcId = frameTable.func[frameId];
      // Resolve which lib this frame belongs to via resource → lib
      const res = funcTable.resource[funcId];
      const libIndex = res >= 0 ? resourceTable.lib[res] ?? 0 : 0;
      acc.push(getLocationId(libIndex, addr));
      s = stackTable.prefix[s];
    }
    stackToLocs.set(stackId, acc);
    return acc;
  }
  const aggregate = new Map();
  for (let i = 0; i < sampleCount; i++) {
    const stk = sm.stack[i];
    const w = sm.weight?.[i] ?? 1;
    if (stk === null || stk === undefined) continue;
    const key = stk;
    aggregate.set(key, (aggregate.get(key) ?? 0) + w);
  }
  // Sampling rate: samply's default is 1000 Hz → 1 sample = 1 ms = 1e6 ns
  const nsPerSample = 1_000_000;
  for (const [stk, count] of aggregate) {
    samples.push(
      new Sample({
        locationId: locsForStack(stk),
        value: [count, count * nsPerSample],
      }),
    );
  }
}

const profile = new Profile({
  sampleType: sampleTypes,
  sample: samples,
  mapping: mappings,
  location: locations,
  function: functions,
  stringTable,
  periodType,
  period: 1_000_000,
});

writeFileSync(outPath, gzipSync(profile.encode()));
console.error(
  `[pprof] ${samples.length} samples, ${functions.length} funcs, ${locations.length} locs → ${outPath}`,
);
