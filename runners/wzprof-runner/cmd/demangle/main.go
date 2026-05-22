// Standalone pprof demangler — reads a pprof file, rewrites every
// function name through the moonbit demangler, writes a new file.
// Useful for any pprof input where moonbit symbols leaked through
// unmodified (e.g. wzprof's direct output).
package main

import (
	"fmt"
	"log"
	"os"
	"strings"

	"github.com/google/pprof/profile"

	"github.com/mizchi/pprof-mbt/go/demangle"
)

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
		pretty := demangle.Symbol(fn.Name)
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
