// Package demangle decodes MoonBit's symbol mangling into a readable
// `package::module::function` path.
//
// MoonBit's compiler emits symbols like `_M0FP26mizchi5bench9ackermann`
// into every backend (wasm `name` section, JS function names, Mach-O / ELF
// exports). Each segment is `<decimal length><identifier of that length>`,
// separated by single-character structural markers (P, B, C, …) and
// occasional namespace-count digits. There's no published specification,
// so this package uses a heuristic: scan the suffix backwards, greedily
// match <digits><chars-of-that-length> segments, and stop when the run is
// no longer well-formed. Mirrors crates/moonbit-demangle (Rust) and
// packages/moonbit-pprof/demangle.mjs (JS).
package demangle

import (
	"regexp"
	"strconv"
	"strings"
)

var (
	manglePrefix  = regexp.MustCompile(`^_*M0[A-Z]`)
	genericSuffix = regexp.MustCompile(`G[A-Za-z]+E$`)
)

func isIdent(s string) bool {
	if len(s) == 0 {
		return false
	}
	c := s[0]
	if !(c == '_' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) {
		return false
	}
	for j := 1; j < len(s); j++ {
		c := s[j]
		if !(c == '_' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')) {
			return false
		}
	}
	return true
}

func isDigit(b byte) bool { return b >= '0' && b <= '9' }

// Symbol converts a mangled MoonBit symbol to a readable name. If the input
// doesn't look mangled, it's returned verbatim.
//
//	demangle.Symbol("_M0FP26mizchi5bench9ackermann") == "mizchi::bench::ackermann"
//	demangle.Symbol("main")                           == "main"
//
// Allocates the return string. For repeated calls on the same input,
// intern through your own cache: the algorithm is O(n²) in the symbol
// length.
func Symbol(name string) string {
	if !manglePrefix.MatchString(name) {
		return name
	}
	inner := strings.TrimLeft(name, "_")
	inner = genericSuffix.ReplaceAllString(inner, "")
	parts := []string{}
	i := len(inner)
	for guard := 0; guard < 50 && i > 0; guard++ {
		var foundChars string
		var foundI int
		hasFound := false
		maxN := i - 1
		if maxN > 64 {
			maxN = 64
		}
		for n := maxN; n >= 1; n-- {
			chars := inner[i-n : i]
			if !isIdent(chars) {
				continue
			}
			dEnd := i - n
			dStart := dEnd
			for dStart > 0 && isDigit(inner[dStart-1]) {
				dStart--
			}
			if dStart == dEnd {
				continue
			}
			target := strconv.Itoa(n)
			for ds := dStart; ds < dEnd; ds++ {
				if inner[ds:dEnd] == target {
					foundChars = chars
					foundI = ds
					hasFound = true
					break
				}
			}
			if hasFound {
				break
			}
		}
		if !hasFound {
			break
		}
		parts = append([]string{foundChars}, parts...)
		i = foundI
	}
	if len(parts) == 0 {
		return name
	}
	return strings.Join(parts, "::")
}
