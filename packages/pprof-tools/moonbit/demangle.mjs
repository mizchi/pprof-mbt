// Decode moonbit's symbol mangling into a readable `a::b::c` path.
//
// Examples
//   `_M0FP26mizchi5bench9ackermann` → `mizchi::bench::ackermann`
//   `__M0FP26mizchi5bench11mandel__sum` → `mizchi::bench::mandel__sum`
//   `M0FP017____moonbit__main` → `____moonbit__main`
//
// The scheme starts each segment with its byte length. Counts and namespace
// markers (like the leading `2` of `26mizchi`) are not length-prefixed, so a
// naive forward scanner misreads them as part of the next length. We parse
// backwards: pin the segment's end to the current position, scan all valid
// lengths from longest to shortest, and at each length try every possible
// split of the preceding digit run. This is enough to recover the user-facing
// package + function name on the vast majority of moonbit symbols we see in
// CPU profiles. Trait/impl prefixes (`_M0I…`, `_M0M…`, `PC…`, `PB…`) and
// generic markers (`GsE`/`GuE`) get stripped or partially decoded — good
// enough to read, not a faithful reconstruction.

export function demangle(name) {
  if (!name) return name;
  // Mach-O carries a leading underscore (`_M0F…`); samply's inline-frame
  // `function` strings strip it (`M0F…`). Normalise to the inner form.
  const match = name.match(/^_*(M0[A-Z].*)$/);
  if (!match) return name;
  const inner = match[1].replace(/G[A-Za-z]+E$/, ""); // drop trailing generic marker
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
