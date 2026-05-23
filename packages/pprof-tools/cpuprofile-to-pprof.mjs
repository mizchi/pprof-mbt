// Convert a Node V8 `.cpuprofile` (Inspector `Profiler.Profile`) into a
// gzip'd pprof protobuf.
//
// The cpuprofile schema is a tree of nodes — `nodes[i].children` lists
// child node ids — plus a `samples` array referencing those nodes and a
// `timeDeltas` array (μs) holding the elapsed time between consecutive
// samples. We invert children into a parent map so we can walk leaf →
// root, aggregate samples by leaf node, and emit pprof samples with both
// `count` and `nanoseconds` values.
//
// Function names go through a caller-supplied demangler — defaults to
// `@mizchi/pprof-tools/moonbit/demangle`. Pass an identity function (`s => s`) when
// profiling non-MoonBit code.

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
import { demangle as defaultDemangle } from "./moonbit/demangle.mjs";

/**
 * @typedef {Object} ConvertOptions
 * @property {(name: string) => string} [demangle] - Defaults to the moonbit demangler.
 * @property {string} [mappingFilename] - Sets the pprof Mapping's filename.
 */

/**
 * Convert a Node V8 cpuprofile into gzip'd pprof protobuf bytes.
 *
 * @param {Object} cpuprofile  - Parsed cpuprofile JSON.
 * @param {ConvertOptions} [options]
 * @returns {Buffer}
 */
export function convert(cpuprofile, { demangle = defaultDemangle, mappingFilename } = {}) {
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
    filename: mappingFilename ? stringTable.dedup(mappingFilename) : 0,
  });

  const functions = [];
  const locations = [];
  const funcIdByKey = new Map();
  const locIdByNode = new Map();

  function getFunctionId(cf) {
    const raw = cf.functionName || "(anonymous)";
    const pretty = demangle(raw);
    const key = `${raw}|${cf.url || ""}|${cf.scriptId || ""}`;
    let id = funcIdByKey.get(key);
    if (id !== undefined) return id;
    id = BigInt(functions.length + 1);
    functions.push(
      new PFunction({
        id,
        name: stringTable.dedup(pretty),
        systemName: stringTable.dedup(raw),
        filename: stringTable.dedup(cf.url || ""),
        startLine: cf.lineNumber >= 0 ? cf.lineNumber + 1 : 0,
      }),
    );
    funcIdByKey.set(key, id);
    return id;
  }

  function getLocationId(node) {
    const hit = locIdByNode.get(node.id);
    if (hit !== undefined) return hit;
    const id = BigInt(locations.length + 1);
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

  const byId = new Map(cpuprofile.nodes.map((n) => [n.id, n]));
  const parent = new Map();
  for (const n of cpuprofile.nodes) {
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

  const samplesByNode = new Map();
  const microsByNode = new Map();
  for (let i = 0; i < cpuprofile.samples.length; i++) {
    const nid = cpuprofile.samples[i];
    const dt = cpuprofile.timeDeltas?.[i] ?? 0;
    samplesByNode.set(nid, (samplesByNode.get(nid) ?? 0) + 1);
    microsByNode.set(nid, (microsByNode.get(nid) ?? 0) + dt);
  }

  const samples = [];
  for (const [nid, count] of samplesByNode) {
    const us = microsByNode.get(nid) ?? 0;
    samples.push(
      new Sample({
        locationId: stackOf(nid),
        value: [count, us * 1000], // μs → ns
      }),
    );
  }

  const totalUs = (cpuprofile.timeDeltas ?? []).reduce((a, b) => a + b, 0);
  const profile = new Profile({
    sampleType: sampleTypes,
    sample: samples,
    mapping: [mapping],
    location: locations,
    function: functions,
    stringTable,
    timeNanos: BigInt(cpuprofile.startTime ?? 0) * 1000n,
    durationNanos: BigInt(((cpuprofile.endTime ?? 0) - (cpuprofile.startTime ?? 0)) * 1000),
    periodType,
    period: Math.max(
      1,
      Math.round(totalUs / Math.max(1, cpuprofile.samples.length)) * 1000,
    ),
  });

  return {
    encoded: gzipSync(profile.encode()),
    stats: {
      samples: samples.length,
      functions: functions.length,
      locations: locations.length,
    },
  };
}
