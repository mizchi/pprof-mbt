// Package demangle decodes moonbit symbol mangling into a readable
// `a::b::c` path. Shared between wzprof-runner and the standalone
// pprof-demangle tool. See runners/lib/demangle.mjs for the JS twin.
package demangle

import (
	"regexp"
	"strconv"
	"strings"
)

var (
	manglePrefix   = regexp.MustCompile(`^_*M0[A-Z]`)
	genericSuffix  = regexp.MustCompile(`G[A-Za-z]+E$`)
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

// Symbol converts a mangled moonbit symbol to a readable name. If the input
// doesn't look mangled, it's returned verbatim.
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
