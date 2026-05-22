// Shared "Firefox Profiler JSON → pprof" pipeline.
//
// samply and wasmtime both speak Firefox's "processed profile" format but
// disagree on how to resolve a frame to a symbol (samply: addr lookup with
// inline chain; wasmtime: direct funcTable index) and how to weigh samples
// (samply: fixed 1 kHz; wasmtime: per-sample timeDeltas). This module owns
// the parts that don't differ — stack-table walking, pprof StringTable
// building, Function/Location interning, encode + gzip — and exposes a
// `resolveFrame`/`resolveSample` hook so callers describe just the bits
// that are format-specific.

import { writeFileSync } from "node:fs";
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
import { demangle as defaultDemangle } from "./demangle.mjs";

/**
 * @typedef {Object} ResolvedFrame
 * @property {string} name                  - The function name (will be demangled).
 * @property {string} [file]                - Source filename.
 * @property {number} [line]                - Line number at the call site.
 * @property {number} [mappingIndex]        - Index into libs[].
 * @property {number|bigint} [address]      - Address/RVA for the pprof Location.
 *
 * @typedef {Object} ResolvedSample
 * @property {number|null|undefined} stack  - Stack id (key into stackTable).
 * @property {number} count                 - Sample count.
 * @property {number} ns                    - CPU time in nanoseconds.
 */

/**
 * Build a pprof Profile from a Firefox-format profile.
 *
 * @param {Object} options
 * @param {Object} options.profile           - Parsed Firefox profile JSON.
 * @param {(thread, frameIdx) => ResolvedFrame[]} options.resolveFrame
 *   Returns the inline chain at a frame, leaf first. One entry = no inlining.
 * @param {(thread, sampleIdx) => ResolvedSample} options.resolveSample
 *   Translates one Firefox sample into a pprof sample value.
 * @param {(name: string) => string} [options.demangle] - Defaults to the
 *   moonbit demangler in ./demangle.mjs.
 * @param {string} [options.label]           - Prefix for the console summary.
 * @returns {Buffer}  gzip'd pprof protobuf.
 */
export function buildPprofFromFirefox({
  profile,
  resolveFrame,
  resolveSample,
  demangle = defaultDemangle,
  label = "pprof",
}) {
  const stringTable = new StringTable();
  const sampleTypes = [
    new ValueType({ type: stringTable.dedup("samples"), unit: stringTable.dedup("count") }),
    new ValueType({ type: stringTable.dedup("cpu"), unit: stringTable.dedup("nanoseconds") }),
  ];
  const periodType = new ValueType({
    type: stringTable.dedup("cpu"),
    unit: stringTable.dedup("nanoseconds"),
  });

  const libs = profile.libs ?? [];
  const mappings = [];
  const mappingByIndex = new Map();
  for (let i = 0; i < Math.max(libs.length, 1); i++) {
    const lib = libs[i];
    const m = new Mapping({
      id: BigInt(i + 1),
      filename: stringTable.dedup(lib?.name ?? "unknown"),
    });
    mappings.push(m);
    mappingByIndex.set(i, m.id);
  }

  const functions = [];
  const funcIdByKey = new Map();
  function internFunction({ name, file = "" }) {
    const pretty = demangle(name);
    const key = `${name}|${file}`;
    let id = funcIdByKey.get(key);
    if (id !== undefined) return id;
    id = BigInt(functions.length + 1);
    functions.push(
      new PFunction({
        id,
        name: stringTable.dedup(pretty),
        systemName: stringTable.dedup(name),
        filename: stringTable.dedup(file),
      }),
    );
    funcIdByKey.set(key, id);
    return id;
  }

  const locations = [];
  // frameLookup caches `(thread, frameIdx) → locationId` so we don't re-run
  // the (possibly expensive) frame resolver. canonicalLookup folds frames
  // with identical resolved content into a single pprof Location.
  const frameLookup = new Map();
  const canonicalLookup = new Map();
  function internLocation(thread, frameIdx) {
    const frameKey = `${thread._idx}:${frameIdx}`;
    const hit = frameLookup.get(frameKey);
    if (hit !== undefined) return hit;

    const frames = resolveFrame(thread, frameIdx);
    const mappingIndex = frames[0]?.mappingIndex ?? 0;
    const address = frames[0]?.address;
    const addrBig =
      typeof address === "bigint" ? address : address != null && address >= 0 ? BigInt(address) : 0n;
    const canonical = frames
      .map((f) => `${f.mappingIndex ?? 0}\x1f${f.address ?? 0}\x1f${f.name}\x1f${f.line ?? 0}`)
      .join("\x1e");
    const existing = canonicalLookup.get(canonical);
    if (existing !== undefined) {
      frameLookup.set(frameKey, existing);
      return existing;
    }

    const id = BigInt(locations.length + 1);
    locations.push(
      new Location({
        id,
        mappingId: mappingByIndex.get(mappingIndex) ?? 1n,
        address: addrBig,
        line: frames.map(
          (fr) =>
            new Line({
              functionId: internFunction({ name: fr.name, file: fr.file ?? "" }),
              line: fr.line ?? 0,
            }),
        ),
      }),
    );
    frameLookup.set(frameKey, id);
    canonicalLookup.set(canonical, id);
    return id;
  }

  function locsForStack(thread, stackId, cache) {
    if (stackId === null || stackId === undefined) return [];
    const hit = cache.get(stackId);
    if (hit) return hit;
    const { stackTable } = thread;
    const acc = [];
    let s = stackId;
    while (s !== null && s !== undefined) {
      acc.push(internLocation(thread, stackTable.frame[s]));
      s = stackTable.prefix[s];
    }
    cache.set(stackId, acc);
    return acc;
  }

  const samples = [];
  let totalNs = 0;
  const threads = profile.threads ?? [];
  for (let ti = 0; ti < threads.length; ti++) {
    // Tag threads so the location cache keys don't collide across them.
    const t = Object.assign(threads[ti], { _idx: ti });
    const stackCache = new Map();
    const aggregate = new Map();
    const n = t.samples?.length ?? 0;
    for (let i = 0; i < n; i++) {
      const rs = resolveSample(t, i);
      if (rs.stack === null || rs.stack === undefined) continue;
      const cur = aggregate.get(rs.stack) ?? { count: 0, ns: 0 };
      cur.count += rs.count;
      cur.ns += rs.ns;
      aggregate.set(rs.stack, cur);
    }
    for (const [stk, { count, ns }] of aggregate) {
      samples.push(
        new Sample({
          locationId: locsForStack(t, stk, stackCache),
          value: [count, ns],
        }),
      );
      totalNs += ns;
    }
  }

  const intervalMs = profile.meta?.interval ?? 1;
  const startMs = profile.meta?.startTime ?? 0;
  const pprof = new Profile({
    sampleType: sampleTypes,
    sample: samples,
    mapping: mappings,
    location: locations,
    function: functions,
    stringTable,
    periodType,
    period: Math.round(intervalMs * 1_000_000),
    timeNanos: BigInt(Math.round(startMs * 1_000_000)),
    durationNanos: BigInt(totalNs),
  });

  return { encoded: gzipSync(pprof.encode()), stats: { samples: samples.length, functions: functions.length, locations: locations.length, label } };
}

/**
 * Convenience: build the pprof and write it to disk, printing a 1-liner.
 */
export function writePprofFromFirefox(opts, outPath) {
  const { encoded, stats } = buildPprofFromFirefox(opts);
  writeFileSync(outPath, encoded);
  console.error(
    `[${stats.label}] ${stats.samples} samples, ${stats.functions} funcs, ${stats.locations} locs → ${outPath}`,
  );
}
