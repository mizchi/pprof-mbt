// Convert Node V8 .cpuprofile (CPUProfile) to pprof CPU profile.
// Input format: Inspector.Profiler.Profile — nodes + samples + timeDeltas (μs).
// Output: gzip'd pprof protobuf. Open with `go tool pprof profile.pb.gz`.

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

const [, , inPath = "wasm-gc.cpuprofile", outPath = "wasm-gc.pb.gz"] = process.argv;
const cp = JSON.parse(readFileSync(inPath, "utf8"));

const stringTable = new StringTable();
const sampleTypes = [
  new ValueType({ type: stringTable.dedup("samples"), unit: stringTable.dedup("count") }),
  new ValueType({ type: stringTable.dedup("cpu"), unit: stringTable.dedup("nanoseconds") }),
];
const periodType = new ValueType({
  type: stringTable.dedup("cpu"),
  unit: stringTable.dedup("nanoseconds"),
});

const mapping = new Mapping({ id: 1n });
const functions = [];
const locations = [];
const funcIdByKey = new Map();
const locIdByNode = new Map();

// Decode moonbit's wasm symbol mangling well enough to be human-readable.
// Example: `_M0FP26mizchi5bench9ackermann` → `mizchi::bench::ackermann`.
// Each segment is `<decimal length><identifier of that length>`; segments are
// concatenated with optional structural markers (P/B/C, a count digit, etc).
// We parse from the right end and at each digit-run try every possible split,
// taking the suffix of the digit-run as the length — that lets us recognise
// the leading `2` of `26mizchi` as a separate count marker instead of part
// of the length.
function demangle(name) {
  if (!name?.startsWith("_M")) return name;
  const stripped = name.replace(/G[A-Za-z]+E$/, ""); // drop trailing generic marker like GsE/GuE
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

function getFunctionId(cf) {
  const raw = cf.functionName || "(anonymous)";
  const pretty = demangle(raw);
  const key = `${raw}|${cf.url || ""}|${cf.scriptId || ""}`;
  let id = funcIdByKey.get(key);
  if (id !== undefined) return id;
  id = BigInt(functions.length + 1);
  const fn = new PFunction({
    id,
    name: stringTable.dedup(pretty),
    systemName: stringTable.dedup(raw),
    filename: stringTable.dedup(cf.url || ""),
    startLine: cf.lineNumber >= 0 ? cf.lineNumber + 1 : 0,
  });
  functions.push(fn);
  funcIdByKey.set(key, id);
  return id;
}

function getLocationId(node) {
  let id = locIdByNode.get(node.id);
  if (id !== undefined) return id;
  id = BigInt(locations.length + 1);
  const funcId = getFunctionId(node.callFrame);
  locations.push(
    new Location({
      id,
      mappingId: mapping.id,
      line: [
        new Line({
          functionId: funcId,
          line: node.callFrame.lineNumber >= 0 ? node.callFrame.lineNumber + 1 : 0,
        }),
      ],
    }),
  );
  locIdByNode.set(node.id, id);
  return id;
}

const byId = new Map(cp.nodes.map((n) => [n.id, n]));
const parent = new Map();
for (const n of cp.nodes) {
  for (const c of n.children ?? []) parent.set(c, n.id);
}

function stackOf(nodeId) {
  const stack = [];
  let cur = nodeId;
  while (cur !== undefined) {
    const node = byId.get(cur);
    if (!node) break;
    stack.push(getLocationId(node));
    cur = parent.get(cur);
  }
  return stack;
}

const samples = [];
const samplesByNode = new Map();
const microsByNode = new Map();
for (let i = 0; i < cp.samples.length; i++) {
  const nid = cp.samples[i];
  const dt = cp.timeDeltas[i] ?? 0;
  samplesByNode.set(nid, (samplesByNode.get(nid) ?? 0) + 1);
  microsByNode.set(nid, (microsByNode.get(nid) ?? 0) + dt);
}

for (const [nid, count] of samplesByNode) {
  const us = microsByNode.get(nid) ?? 0;
  samples.push(
    new Sample({
      locationId: stackOf(nid),
      value: [count, us * 1000], // μs → ns
    }),
  );
}

const totalUs = cp.timeDeltas.reduce((a, b) => a + b, 0);
const profile = new Profile({
  sampleType: sampleTypes,
  sample: samples,
  mapping: [mapping],
  location: locations,
  function: functions,
  stringTable,
  timeNanos: BigInt(cp.startTime) * 1000n,
  durationNanos: BigInt((cp.endTime - cp.startTime) * 1000),
  periodType,
  period: Math.max(1, Math.round(totalUs / Math.max(1, cp.samples.length)) * 1000),
});

writeFileSync(outPath, gzipSync(profile.encode()));
console.error(
  `[pprof] ${samples.length} samples, ${functions.length} funcs, ${locations.length} locs → ${outPath}`,
);
