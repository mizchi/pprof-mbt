// Rewrites function names in a pprof file through the moonbit demangler.
// Useful for outputs (like wzprof's) that bake the raw mangled symbols
// into the .pb.gz directly, since `go tool pprof` doesn't know moonbit.

import { readFileSync, writeFileSync } from "node:fs";
import { gunzipSync, gzipSync } from "node:zlib";
import { Profile } from "pprof-format";

function demangle(name) {
  if (!name) return name;
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

const [, , inPath, outPath = inPath.replace(/(\.pb\.gz|\.pb)$/, ".demangled$1")] = process.argv;
if (!inPath) {
  console.error("usage: pprof-demangle.mjs <profile.pb.gz> [out.pb.gz]");
  process.exit(1);
}

const raw = readFileSync(inPath);
const decoded = inPath.endsWith(".gz") ? gunzipSync(raw) : raw;
const prof = Profile.decode(decoded);

const strs = prof.stringTable;
const oldToNew = new Map();
function intern(s) {
  if (oldToNew.has(s)) return oldToNew.get(s);
  const idx = strs.dedup(s);
  oldToNew.set(s, idx);
  return idx;
}

let rewritten = 0;
for (const fn of prof.function) {
  const orig = strs.strings[Number(fn.name)];
  const pretty = demangle(orig);
  if (pretty !== orig) {
    fn.name = intern(pretty);
    if (!fn.systemName || Number(fn.systemName) === 0) {
      fn.systemName = intern(orig);
    }
    rewritten++;
  }
}

writeFileSync(outPath, gzipSync(prof.encode()));
console.error(`[demangle] rewrote ${rewritten}/${prof.function.length} functions → ${outPath}`);
