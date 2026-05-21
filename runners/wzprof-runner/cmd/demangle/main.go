// Standalone pprof demangler — reads a pprof file, rewrites every
// function name through the moonbit demangler, writes a new file.
// Same logic as wzprof-runner but applicable to any pprof input.
package main

import (
	"fmt"
	"log"
	"os"
	"regexp"
	"strconv"
	"strings"

	"github.com/google/pprof/profile"
)

var manglePrefix = regexp.MustCompile(`^_*M0[A-Z]`)
var genericSuffix = regexp.MustCompile(`G[A-Za-z]+E$`)

func demangle(name string) string {
	if !manglePrefix.MatchString(name) {
		return name
	}
	inner := strings.TrimLeft(name, "_")
	inner = genericSuffix.ReplaceAllString(inner, "")
	parts := []string{}
	i := len(inner)
	isIdent := func(s string) bool {
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
	isDigit := func(b byte) bool { return b >= '0' && b <= '9' }
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

func main() {
	if len(os.Args) < 2 {
		log.Fatal("usage: demangle <in.pb.gz> [out.pb.gz]")
	}
	in := os.Args[1]
	out := in
	if len(os.Args) > 2 {
		out = os.Args[2]
	} else {
		out = strings.TrimSuffix(in, ".pb.gz") + ".demangled.pb.gz"
	}

	f, err := os.Open(in)
	if err != nil {
		log.Fatal(err)
	}
	p, err := profile.Parse(f)
	f.Close()
	if err != nil {
		log.Fatal(err)
	}

	rewritten := 0
	for _, fn := range p.Function {
		pretty := demangle(fn.Name)
		if pretty != fn.Name {
			if fn.SystemName == "" {
				fn.SystemName = fn.Name
			}
			fn.Name = pretty
			rewritten++
		}
	}

	w, err := os.Create(out)
	if err != nil {
		log.Fatal(err)
	}
	if err := p.Write(w); err != nil {
		log.Fatal(err)
	}
	w.Close()
	fmt.Fprintf(os.Stderr, "[demangle] rewrote %d/%d functions → %s\n", rewritten, len(p.Function), out)
}
