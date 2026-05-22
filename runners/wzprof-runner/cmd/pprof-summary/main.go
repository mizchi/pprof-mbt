// pprof-summary reads a pprof and emits three views:
//   * total CPU + breakdown between MoonBit's memory-management runtime
//     functions (incref / decref / malloc / free / TLSF / get_tag /
//     make_array_header) and everything else (user code + lib code)
//   * top user functions by self time (with mem-mgmt frames hidden)
//   * top user functions by transitive time spent in mem-mgmt — i.e.
//     "which code paths allocate the most"
//
// Useful for quickly answering "how much of this profile is just
// reference-counting and allocation?" without spinning up the pprof UI.
package main

import (
	"fmt"
	"os"
	"regexp"
	"sort"
	"strings"

	"github.com/google/pprof/profile"
)

// memMgmt matches the runtime symbols MoonBit emits for refcount and
// allocator primitives across all three backends. Tuned against
// json_parse / sorted_map_merge / regex_match outputs.
var memMgmt = regexp.MustCompile(
	`^(moonbit\.(incref|decref|gc\.malloc|gc\.free|malloc|free|make_array_header|get_tag|array_length|check_range|drop_object)|tlsf/.+|moonbit_drop_object|libc_(malloc|free)|moonbit_malloc|moonbit_decref|moonbit_incref|_(?:malloc|free)|libsystem_malloc\..*)$`,
)

func isMemMgmt(name string) bool {
	return memMgmt.MatchString(name)
}

// timeUnit reports the unit of the cum/flat values in the profile.
func timeUnit(p *profile.Profile) (string, float64) {
	// First sample_type entry expected to be "samples"; second one carries
	// nanoseconds. Falls back to nanoseconds.
	for _, st := range p.SampleType {
		if st.Type == "cpu" || st.Type == "wall" {
			switch st.Unit {
			case "nanoseconds":
				return "ms", 1e6
			case "microseconds":
				return "ms", 1e3
			case "milliseconds":
				return "ms", 1
			case "count":
				return "samples", 1
			}
		}
	}
	return "ns", 1
}

// valueIndex picks the sample value column representing time (or count
// when no time column exists).
func valueIndex(p *profile.Profile) int {
	for i, st := range p.SampleType {
		if st.Type == "cpu" || st.Type == "wall" {
			if st.Unit == "nanoseconds" || st.Unit == "microseconds" || st.Unit == "milliseconds" {
				return i
			}
		}
	}
	return 0
}

type funcStats struct {
	name string
	self int64 // attributable only to this function (leaf time)
	cum  int64 // total time including callers/callees (only meaningful via stack)
	// memCum: time spent in stacks where this function appears AND a
	// mem-mgmt frame appears below it. Approximates "this function caused
	// memory work".
	memCum int64
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: pprof-summary <profile.pb.gz>")
		os.Exit(2)
	}
	f, err := os.Open(os.Args[1])
	if err != nil {
		fmt.Fprintln(os.Stderr, "open:", err)
		os.Exit(1)
	}
	p, err := profile.Parse(f)
	f.Close()
	if err != nil {
		fmt.Fprintln(os.Stderr, "parse:", err)
		os.Exit(1)
	}
	vIdx := valueIndex(p)
	unit, div := timeUnit(p)

	stats := map[string]*funcStats{}
	get := func(name string) *funcStats {
		s, ok := stats[name]
		if !ok {
			s = &funcStats{name: name}
			stats[name] = s
		}
		return s
	}

	var totalNs, memMgmtNs int64
	for _, s := range p.Sample {
		v := s.Value[vIdx]
		totalNs += v

		// Walk the call stack leaf → root, tracking which functions
		// appear and whether a mem-mgmt frame is below.
		seen := map[string]struct{}{}
		hadMemMgmtBelow := false
		// pprof location[0] is the leaf.
		// Self time goes to the LEAF function only.
		if len(s.Location) > 0 {
			leafName := topLine(s.Location[0])
			get(leafName).self += v
			if isMemMgmt(leafName) {
				memMgmtNs += v
				hadMemMgmtBelow = true
			}
		}
		// Cumulative + memCum walk root → leaf so we know if a leaf is
		// mem-mgmt and credit ancestors accordingly.
		// Re-traverse leaf → root, then root → leaf for memCum.
		for _, loc := range s.Location {
			name := topLine(loc)
			if _, ok := seen[name]; ok {
				continue
			}
			seen[name] = struct{}{}
			get(name).cum += v
		}
		if hadMemMgmtBelow {
			for _, loc := range s.Location {
				name := topLine(loc)
				if isMemMgmt(name) {
					continue
				}
				get(name).memCum += v
			}
		}
	}

	fmt.Printf("Profile: %s\n", os.Args[1])
	fmt.Printf("Total %s: %.2f (%d samples)\n", unit, float64(totalNs)/div, len(p.Sample))
	fmt.Printf("Memory-management self time: %.2f %s (%.1f%%)\n",
		float64(memMgmtNs)/div, unit, pct(memMgmtNs, totalNs))
	fmt.Println()

	all := make([]*funcStats, 0, len(stats))
	for _, s := range stats {
		all = append(all, s)
	}

	// User functions = everything that isn't a mem-mgmt primitive.
	users := filter(all, func(s *funcStats) bool { return !isMemMgmt(s.name) })

	printTop("Top user functions by self time (mem-mgmt frames hidden)",
		sortBySelf(users), unit, div, totalNs, 12, func(s *funcStats) int64 { return s.self })

	printTop("Top user functions by mem-mgmt-attributed time (callers of allocs)",
		sortByMemCum(users), unit, div, totalNs, 12, func(s *funcStats) int64 { return s.memCum })

	printTop("Top mem-mgmt primitives by self time",
		sortBySelf(filter(all, func(s *funcStats) bool { return isMemMgmt(s.name) })),
		unit, div, totalNs, 10, func(s *funcStats) int64 { return s.self })
}

func topLine(loc *profile.Location) string {
	if len(loc.Line) == 0 || loc.Line[0].Function == nil {
		return "(unknown)"
	}
	return loc.Line[0].Function.Name
}

func pct(num, den int64) float64 {
	if den == 0 {
		return 0
	}
	return 100 * float64(num) / float64(den)
}

func filter[T any](xs []T, keep func(T) bool) []T {
	out := make([]T, 0, len(xs))
	for _, x := range xs {
		if keep(x) {
			out = append(out, x)
		}
	}
	return out
}

func sortBySelf(xs []*funcStats) []*funcStats {
	out := append([]*funcStats(nil), xs...)
	sort.Slice(out, func(i, j int) bool { return out[i].self > out[j].self })
	return out
}

func sortByMemCum(xs []*funcStats) []*funcStats {
	out := append([]*funcStats(nil), xs...)
	sort.Slice(out, func(i, j int) bool { return out[i].memCum > out[j].memCum })
	return out
}

func printTop(title string, xs []*funcStats, unit string, div float64, total int64, n int, val func(*funcStats) int64) {
	fmt.Println(title)
	fmt.Println(strings.Repeat("-", len(title)))
	if n > len(xs) {
		n = len(xs)
	}
	for _, s := range xs[:n] {
		v := val(s)
		if v == 0 {
			break
		}
		fmt.Printf("  %7.2f %s  %5.1f%%  %s\n",
			float64(v)/div, unit, pct(v, total), s.name)
	}
	fmt.Println()
}
