// pprof-summary reads a pprof and emits three views:
//   * total CPU + breakdown between MoonBit's memory-management runtime
//     functions (incref / decref / malloc / free / TLSF / get_tag /
//     make_array_header) and everything else (user code + lib code)
//   * top user functions by self time (with mem-mgmt frames hidden)
//   * top user functions by transitive time spent in mem-mgmt — i.e.
//     "which code paths allocate the most"
//
// With `--diff base.pb.gz patched.pb.gz` it instead diffs two profiles
// and shows top improvements / regressions / appearances / disappearances
// at function self-time granularity. Useful for verifying a perf patch
// landed where it was supposed to.
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
	self int64
	cum  int64
	memCum int64
}

// diffRow represents a single function's (base, patched, delta) self-time
// triple. Used by the diff subcommand only.
type diffRow struct {
	name              string
	base, patched, dx int64
}

func usage() {
	fmt.Fprintln(os.Stderr, "usage:")
	fmt.Fprintln(os.Stderr, "  pprof-summary <profile.pb.gz>")
	fmt.Fprintln(os.Stderr, "  pprof-summary --diff <base.pb.gz> <patched.pb.gz>")
	os.Exit(2)
}

func main() {
	if len(os.Args) < 2 {
		usage()
	}
	switch os.Args[1] {
	case "--diff", "-d", "diff":
		if len(os.Args) != 4 {
			usage()
		}
		if err := runDiff(os.Args[2], os.Args[3]); err != nil {
			fmt.Fprintln(os.Stderr, "diff:", err)
			os.Exit(1)
		}
	case "-h", "--help", "help":
		usage()
	default:
		if err := runSingle(os.Args[1]); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
	}
}

// runSingle preserves the original single-profile summary behaviour.
func runSingle(path string) error {
	p, _, err := loadProfile(path)
	if err != nil {
		return err
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

		seen := map[string]struct{}{}
		hadMemMgmtBelow := false
		if len(s.Location) > 0 {
			leafName := topLine(s.Location[0])
			get(leafName).self += v
			if isMemMgmt(leafName) {
				memMgmtNs += v
				hadMemMgmtBelow = true
			}
		}
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

	fmt.Printf("Profile: %s\n", path)
	fmt.Printf("Total %s: %.2f (%d samples)\n", unit, float64(totalNs)/div, len(p.Sample))
	fmt.Printf("Memory-management self time: %.2f %s (%.1f%%)\n",
		float64(memMgmtNs)/div, unit, pct(memMgmtNs, totalNs))
	fmt.Println()

	all := make([]*funcStats, 0, len(stats))
	for _, s := range stats {
		all = append(all, s)
	}

	users := filter(all, func(s *funcStats) bool { return !isMemMgmt(s.name) })

	printTop("Top user functions by self time (mem-mgmt frames hidden)",
		sortBySelf(users), unit, div, totalNs, 12, func(s *funcStats) int64 { return s.self })

	printTop("Top user functions by mem-mgmt-attributed time (callers of allocs)",
		sortByMemCum(users), unit, div, totalNs, 12, func(s *funcStats) int64 { return s.memCum })

	printTop("Top mem-mgmt primitives by self time",
		sortBySelf(filter(all, func(s *funcStats) bool { return isMemMgmt(s.name) })),
		unit, div, totalNs, 10, func(s *funcStats) int64 { return s.self })
	return nil
}

// computeSelf collects per-function self time for one profile, returning
// (name -> self ns), total ns, sample count.
func computeSelf(path string) (map[string]int64, int64, int, string, float64, error) {
	p, _, err := loadProfile(path)
	if err != nil {
		return nil, 0, 0, "", 0, err
	}
	vIdx := valueIndex(p)
	unit, div := timeUnit(p)
	self := map[string]int64{}
	var total int64
	for _, s := range p.Sample {
		v := s.Value[vIdx]
		total += v
		if len(s.Location) > 0 {
			self[topLine(s.Location[0])] += v
		}
	}
	return self, total, len(p.Sample), unit, div, nil
}

func runDiff(basePath, patchedPath string) error {
	baseSelf, baseTotal, baseN, baseUnit, baseDiv, err := computeSelf(basePath)
	if err != nil {
		return fmt.Errorf("base %s: %w", basePath, err)
	}
	patchedSelf, patchedTotal, patchedN, patchedUnit, patchedDiv, err := computeSelf(patchedPath)
	if err != nil {
		return fmt.Errorf("patched %s: %w", patchedPath, err)
	}
	if baseUnit != patchedUnit || baseDiv != patchedDiv {
		return fmt.Errorf("base / patched use different time units (%s vs %s)", baseUnit, patchedUnit)
	}
	unit, div := baseUnit, baseDiv

	fmt.Printf("Profile diff:\n")
	fmt.Printf("  base    = %s\n", basePath)
	fmt.Printf("  patched = %s\n", patchedPath)
	totalDelta := patchedTotal - baseTotal
	fmt.Printf("\nTotal %s: %.2f (%d samples) -> %.2f (%d samples) | Δ %+.2f %s (%+.1f%%)\n\n",
		unit,
		float64(baseTotal)/div, baseN,
		float64(patchedTotal)/div, patchedN,
		float64(totalDelta)/div, unit,
		pct(totalDelta, baseTotal))

	all := []diffRow{}
	keys := map[string]struct{}{}
	for k := range baseSelf {
		keys[k] = struct{}{}
	}
	for k := range patchedSelf {
		keys[k] = struct{}{}
	}
	for k := range keys {
		b := baseSelf[k]
		p := patchedSelf[k]
		all = append(all, diffRow{name: k, base: b, patched: p, dx: p - b})
	}

	improvements := filterRows(all, func(r diffRow) bool { return r.dx < 0 && r.base > 0 && r.patched > 0 })
	sort.Slice(improvements, func(i, j int) bool { return improvements[i].dx < improvements[j].dx })
	printDiffRows("Top improvements (Δself, largest decrease first)", improvements, unit, div, 15)

	regressions := filterRows(all, func(r diffRow) bool { return r.dx > 0 && r.base > 0 && r.patched > 0 })
	sort.Slice(regressions, func(i, j int) bool { return regressions[i].dx > regressions[j].dx })
	printDiffRows("Top regressions (Δself, largest increase first)", regressions, unit, div, 10)

	gone := filterRows(all, func(r diffRow) bool { return r.base > 0 && r.patched == 0 })
	sort.Slice(gone, func(i, j int) bool { return gone[i].base > gone[j].base })
	printDisappearedRows("Disappeared in patched (function only in base)", gone, unit, div, baseTotal, 10)

	novel := filterRows(all, func(r diffRow) bool { return r.base == 0 && r.patched > 0 })
	sort.Slice(novel, func(i, j int) bool { return novel[i].patched > novel[j].patched })
	printAppearedRows("New in patched (function only in patched)", novel, unit, div, 10)

	return nil
}

func filterRows(xs []diffRow, keep func(diffRow) bool) []diffRow {
	out := make([]diffRow, 0, len(xs))
	for _, x := range xs {
		if keep(x) {
			out = append(out, x)
		}
	}
	return out
}

func printDiffRows(title string, rows []diffRow, unit string, div float64, n int) {
	fmt.Println(title)
	fmt.Println(strings.Repeat("-", len(title)))
	if n > len(rows) {
		n = len(rows)
	}
	if n == 0 {
		fmt.Println("  (none)")
		fmt.Println()
		return
	}
	for _, r := range rows[:n] {
		pctChange := 0.0
		if r.base > 0 {
			pctChange = float64(r.dx) / float64(r.base) * 100
		}
		fmt.Printf("  %+9.2f %s  %+6.1f%%  %-50s (%.2f -> %.2f)\n",
			float64(r.dx)/div, unit, pctChange, r.name,
			float64(r.base)/div, float64(r.patched)/div)
	}
	fmt.Println()
}

func printDisappearedRows(title string, rows []diffRow, unit string, div float64, baseTotal int64, n int) {
	fmt.Println(title)
	fmt.Println(strings.Repeat("-", len(title)))
	if n > len(rows) {
		n = len(rows)
	}
	if n == 0 {
		fmt.Println("  (none)")
		fmt.Println()
		return
	}
	for _, r := range rows[:n] {
		fmt.Printf("  %9.2f %s  was %5.1f%% of base   %s\n",
			float64(r.base)/div, unit, pct(r.base, baseTotal), r.name)
	}
	fmt.Println()
}

func printAppearedRows(title string, rows []diffRow, unit string, div float64, n int) {
	fmt.Println(title)
	fmt.Println(strings.Repeat("-", len(title)))
	if n > len(rows) {
		n = len(rows)
	}
	if n == 0 {
		fmt.Println("  (none)")
		fmt.Println()
		return
	}
	for _, r := range rows[:n] {
		fmt.Printf("  %9.2f %s                       %s\n",
			float64(r.patched)/div, unit, r.name)
	}
	fmt.Println()
}

func loadProfile(path string) (*profile.Profile, *os.File, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, nil, fmt.Errorf("open: %w", err)
	}
	p, err := profile.Parse(f)
	f.Close()
	if err != nil {
		return nil, nil, fmt.Errorf("parse: %w", err)
	}
	return p, nil, nil
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

