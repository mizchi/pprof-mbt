// Convert wasmtime's GuestProfiler JSON (Firefox processed-profile format)
// into pprof. Unlike samply's output, function names are already resolved
// in funcTable — we just need to walk the stack table, aggregate samples,
// and run the moonbit demangler.

import { readFileSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
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

const [, , inPath = "wasmtime-guest.json", outPath = "wasmtime-guest.pb.gz"] = process.argv;
const j = JSON.parse(readFileSync(inPath, "utf8"));

function demangle(name) {
  if (!name) return name;
  const match = name.match(/^_*(M0[A-Z].*)$/);
  if (!match) return name;
  const inner = match[1].replace(/G[A-Za-z]+E$/, "");
  const parts = [];
  let i = inner.length;
  for (let guard = 0; guard < 50 && i > 0; guard++) {
    let found = null;
    for (let n = Math.min(i - 1, 64); n >= 1; n--) {
      const chars = inner.slice(i - n, i);
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(chars)) continue;
      const dEnd = i - n;
      let dStart = dEnd;
      while (dStart > 0 && /\d/.test(inner[dStart - 1])) dStart--;
      if (dStart === dEnd) continue;
      const target = String(n);
      for (let ds = dStart; ds < dEnd; ds++) {
        if (inner.slice(ds, dEnd) === target) {
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

const stringTable = new StringTable();
const sampleTypes = [
  new ValueType({ type: stringTable.dedup("samples"), unit: stringTable.dedup("count") }),
  new ValueType({ type: stringTable.dedup("cpu"), unit: stringTable.dedup("nanoseconds") }),
];
const periodType = new ValueType({
  type: stringTable.dedup("cpu"),
  unit: stringTable.dedup("nanoseconds"),
});

const mapping = new Mapping({
  id: 1n,
  filename: stringTable.dedup(j.libs?.[0]?.name ?? "wasm"),
});

const functions = [];
const locations = [];
const funcIdByKey = new Map();
const locIdByFrame = new Map();

const t = j.threads[0];
const { frameTable, funcTable, stackTable, samples, stringArray } = t;

function getFunctionId(funcIdx) {
  const key = `f:${funcIdx}`;
  let id = funcIdByKey.get(key);
  if (id !== undefined) return id;
  const rawName = stringArray[funcTable.name[funcIdx]] || "(anonymous)";
  const pretty = demangle(rawName);
  const file = funcTable.fileName?.[funcIdx];
  const fileStr = file != null && file >= 0 ? stringArray[file] || "" : "";
  id = BigInt(functions.length + 1);
  functions.push(
    new PFunction({
      id,
      name: stringTable.dedup(pretty),
      systemName: stringTable.dedup(rawName),
      filename: stringTable.dedup(fileStr),
      startLine: funcTable.lineNumber?.[funcIdx] ?? 0,
    }),
  );
  funcIdByKey.set(key, id);
  return id;
}

function getLocationId(frameIdx) {
  let id = locIdByFrame.get(frameIdx);
  if (id !== undefined) return id;
  id = BigInt(locations.length + 1);
  const funcIdx = frameTable.func[frameIdx];
  const addr = frameTable.address?.[frameIdx];
  locations.push(
    new Location({
      id,
      mappingId: mapping.id,
      address: addr != null && addr >= 0 ? BigInt(addr) : 0n,
      line: [
        new Line({
          functionId: getFunctionId(funcIdx),
          line: frameTable.line?.[frameIdx] ?? 0,
        }),
      ],
    }),
  );
  locIdByFrame.set(frameIdx, id);
  return id;
}

const stackToLocs = new Map();
function locsForStack(stackId) {
  if (stackId === null || stackId === undefined) return [];
  if (stackToLocs.has(stackId)) return stackToLocs.get(stackId);
  const acc = [];
  let s = stackId;
  while (s !== null && s !== undefined) {
    acc.push(getLocationId(stackTable.frame[s]));
    s = stackTable.prefix[s];
  }
  stackToLocs.set(stackId, acc);
  return acc;
}

const intervalMs = j.meta?.interval ?? 1;
const nsPerSample = Math.round(intervalMs * 1_000_000);

const aggregate = new Map();
for (let i = 0; i < samples.length; i++) {
  const stk = samples.stack[i];
  const w = samples.weight?.[i] ?? 1;
  const dt = samples.timeDeltas?.[i] ?? intervalMs;
  if (stk === null || stk === undefined) continue;
  const cur = aggregate.get(stk) ?? { count: 0, ns: 0 };
  cur.count += w;
  cur.ns += Math.round(dt * 1_000_000);
  aggregate.set(stk, cur);
}

const pprofSamples = [];
for (const [stk, { count, ns }] of aggregate) {
  pprofSamples.push(
    new Sample({
      locationId: locsForStack(stk),
      value: [count, ns],
    }),
  );
}

const totalNs = Array.from(aggregate.values()).reduce((acc, v) => acc + v.ns, 0);
const profile = new Profile({
  sampleType: sampleTypes,
  sample: pprofSamples,
  mapping: [mapping],
  location: locations,
  function: functions,
  stringTable,
  periodType,
  period: nsPerSample,
  timeNanos: BigInt(Math.round((j.meta?.startTime ?? 0) * 1_000_000)),
  durationNanos: BigInt(totalNs),
});

writeFileSync(outPath, gzipSync(profile.encode()));
console.error(
  `[wasmtime→pprof] ${pprofSamples.length} samples (${samples.length} raw), ${functions.length} funcs, ${locations.length} locs → ${outPath}`,
);
