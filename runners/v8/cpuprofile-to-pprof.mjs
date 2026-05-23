// CLI wrapper around `@mizchi/pprof-tools/cpuprofile-to-pprof`.
//
// Reads a Node V8 `.cpuprofile`, runs it through the conversion library,
// and writes a gzip'd pprof. The conversion logic itself lives in the
// reusable package — this file is purely I/O.

import { readFileSync, writeFileSync } from "node:fs";
import { convert } from "@mizchi/pprof-tools/cpuprofile-to-pprof";

const [, , inPath = "wasm-gc.cpuprofile", outPath = "wasm-gc.pb.gz"] = process.argv;
const cpuprofile = JSON.parse(readFileSync(inPath, "utf8"));
const { encoded, stats } = convert(cpuprofile);
writeFileSync(outPath, encoded);
console.error(
  `[pprof] ${stats.samples} samples, ${stats.functions} funcs, ${stats.locations} locs → ${outPath}`,
);
