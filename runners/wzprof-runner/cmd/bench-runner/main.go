// bench-runner: drive a set of moonbit benches across (baseline, patched)
// toolchains × (wasm, wasm-gc, js, native) and emit a markdown delta
// table suitable for pasting into a PR description.
//
// Usage:
//
//	bench-runner \
//	  --baseline-moon ~/.moon \
//	  --patched-moon /tmp/moonbit-patched \
//	  --bench-dir ./bench \
//	  --runner-dir ./runners \
//	  --bin-dir ./.bin \
//	  --workloads bigint_ops,hashmap_ops,... \
//	  --runs 3 \
//	  --backends wasm,wasm-gc,js,native
//
// The tool builds each workload under both toolchains, runs each
// (workload, backend, toolchain) combination `--runs` times with
// profilers disabled, and prints a markdown table with the median wall
// times and deltas.
package main

import (
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

type config struct {
	baselineMoon       string
	patchedMoon        string
	mooncakesBaseline  string
	mooncakesPatched   string
	benchDir           string
	runnerDir          string
	binDir             string
	workloads          []string
	backends           []string
	runs               int
	build              bool
}

func main() {
	var c config
	var workloadsStr, backendsStr string
	flag.StringVar(&c.baselineMoon, "baseline-moon", os.Getenv("HOME")+"/.moon", "Path to baseline moonbit toolchain root")
	flag.StringVar(&c.patchedMoon, "patched-moon", "/tmp/moonbit-patched", "Path to patched moonbit toolchain root (falls back to baseline if missing)")
	flag.StringVar(&c.mooncakesBaseline, "mooncakes-baseline", "", "Path to a .mooncakes snapshot to use as baseline (registry-dep swap mode)")
	flag.StringVar(&c.mooncakesPatched, "mooncakes-patched", "", "Path to a .mooncakes snapshot to use as patched (registry-dep swap mode)")
	flag.StringVar(&c.benchDir, "bench-dir", "./bench", "Path to bench workspace (containing cmd/<workload>)")
	flag.StringVar(&c.runnerDir, "runner-dir", "./runners", "Path to runners (run-wasm-gc.mjs etc)")
	flag.StringVar(&c.binDir, "bin-dir", "./.bin", "Path to .bin (wasmtime-runner)")
	flag.StringVar(&workloadsStr, "workloads", "", "Comma-separated workload names; defaults to every dir under bench/cmd")
	flag.StringVar(&backendsStr, "backends", "wasm,wasm-gc,js,native", "Comma-separated backends")
	flag.IntVar(&c.runs, "runs", 3, "Number of runs per (workload, backend, toolchain) cell")
	flag.BoolVar(&c.build, "build", true, "Build benches before running (set --build=false to reuse _build)")
	flag.Parse()

	c.backends = strings.Split(backendsStr, ",")
	if workloadsStr == "" {
		ws, err := listWorkloads(filepath.Join(c.benchDir, "cmd"))
		if err != nil {
			die("listing workloads: %v", err)
		}
		c.workloads = ws
	} else {
		c.workloads = strings.Split(workloadsStr, ",")
	}

	if err := run(&c); err != nil {
		die("%v", err)
	}
}

func die(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "bench-runner: "+format+"\n", args...)
	os.Exit(1)
}

func listWorkloads(cmdDir string) ([]string, error) {
	entries, err := os.ReadDir(cmdDir)
	if err != nil {
		return nil, err
	}
	var out []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		// Skip "main" since it's just the generic startup workload.
		if e.Name() == "main" {
			continue
		}
		out = append(out, e.Name())
	}
	sort.Strings(out)
	return out, nil
}

func run(c *config) error {
	// (workload, backend) -> { baseline median, patched median }
	results := make(map[string]map[string]cell)

	// If -patched-moon was left at the default and doesn't exist on disk,
	// transparently fall back to -baseline-moon. This matters when the user
	// is using the mooncakes-swap mode and doesn't have a patched toolchain.
	patchedMoon := c.patchedMoon
	if !dirExists(patchedMoon) {
		fmt.Fprintf(os.Stderr, "==> patched toolchain %s missing; using baseline for both phases\n", patchedMoon)
		patchedMoon = c.baselineMoon
	}

	mooncakesSwap := c.mooncakesBaseline != "" || c.mooncakesPatched != ""
	if mooncakesSwap {
		if c.mooncakesBaseline == "" || c.mooncakesPatched == "" {
			return fmt.Errorf("-mooncakes-baseline and -mooncakes-patched must be set together")
		}
		if !dirExists(c.mooncakesBaseline) {
			return fmt.Errorf("-mooncakes-baseline %s does not exist", c.mooncakesBaseline)
		}
		if !dirExists(c.mooncakesPatched) {
			return fmt.Errorf("-mooncakes-patched %s does not exist", c.mooncakesPatched)
		}
	}

	for _, kind := range []string{"baseline", "patched"} {
		moonRoot := c.baselineMoon
		if kind == "patched" {
			moonRoot = patchedMoon
		}
		fmt.Fprintf(os.Stderr, "==> %s toolchain: %s\n", kind, moonRoot)

		if mooncakesSwap {
			src := c.mooncakesBaseline
			if kind == "patched" {
				src = c.mooncakesPatched
			}
			if err := swapMooncakes(c.benchDir, src); err != nil {
				return fmt.Errorf("mooncakes swap (%s): %w", kind, err)
			}
			fmt.Fprintf(os.Stderr, "==> %s mooncakes: %s -> %s/.mooncakes\n", kind, src, c.benchDir)
		}

		if c.build {
			if err := buildAll(c, moonRoot); err != nil {
				return fmt.Errorf("build (%s): %w", kind, err)
			}
		}
		for _, w := range c.workloads {
			if results[w] == nil {
				results[w] = make(map[string]cell)
			}
			for _, b := range c.backends {
				times := make([]float64, 0, c.runs)
				for r := 0; r < c.runs; r++ {
					t, err := runOnce(c, w, b)
					if err != nil {
						fmt.Fprintf(os.Stderr, "  %s/%s run %d: %v\n", w, b, r+1, err)
						break
					}
					times = append(times, t)
				}
				if len(times) == 0 {
					continue
				}
				med := median(times)
				ce := results[w][b]
				if kind == "baseline" {
					ce.baseMs = med
				} else {
					ce.patchedMs = med
				}
				results[w][b] = ce
				fmt.Fprintf(os.Stderr, "  %-22s %-7s %s = %.1f ms (median of %d)\n", w, b, kind, med, len(times))
			}
		}
	}

	printMarkdown(c, results)
	return nil
}

func buildAll(c *config, moonRoot string) error {
	moonBin := filepath.Join(moonRoot, "bin", "moon")
	env := append(os.Environ(),
		"PATH="+filepath.Join(moonRoot, "bin")+":"+os.Getenv("PATH"),
		"MOON_TOOLCHAIN_ROOT="+moonRoot,
	)
	// rm -rf _build for a clean state.
	buildDir := filepath.Join(c.benchDir, "_build")
	_ = os.RemoveAll(buildDir)

	for _, w := range c.workloads {
		for _, b := range c.backends {
			args := []string{"build", "--release", "--target=" + b, "cmd/" + w}
			if b == "wasm" || b == "wasm-gc" {
				args = append([]string{"build", "--release", "--no-strip", "--target=" + b, "cmd/" + w}, nil...)
				args = args[:5]
			}
			cmd := exec.Command(moonBin, args...)
			cmd.Dir = c.benchDir
			cmd.Env = env
			if out, err := cmd.CombinedOutput(); err != nil {
				return fmt.Errorf("moon build %s/%s: %w\n%s", w, b, err, out)
			}
		}
	}
	return nil
}

var msRe = regexp.MustCompile(`([0-9]+\.[0-9]+|[0-9]+) *ms`)

func runOnce(c *config, workload, backend string) (float64, error) {
	switch backend {
	case "wasm":
		return runWasm(c, workload)
	case "wasm-gc":
		return runWasmGC(c, workload)
	case "js":
		return runJS(c, workload)
	case "native":
		return runNative(c, workload)
	default:
		return 0, fmt.Errorf("unknown backend %q", backend)
	}
}

func runWasm(c *config, w string) (float64, error) {
	bin := filepath.Join(c.binDir, "wasmtime-runner")
	path := filepath.Join(c.benchDir, "_build", "wasm", "release", "build", "cmd", w, w+".wasm")
	cmd := exec.Command(bin, "--no-profile", path)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return 0, fmt.Errorf("%w: %s", err, out)
	}
	return parseMs(string(out))
}

func runWasmGC(c *config, w string) (float64, error) {
	path := filepath.Join(c.benchDir, "_build", "wasm-gc", "release", "build", "cmd", w, w+".wasm")
	cmd := exec.Command("node", filepath.Join(c.runnerDir, "run-wasm-gc.mjs"), "--no-profile", path)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return 0, fmt.Errorf("%w: %s", err, out)
	}
	return parseMs(string(out))
}

func runJS(c *config, w string) (float64, error) {
	// run-js.mjs requires an absolute path for the dynamic import.
	abs, err := filepath.Abs(filepath.Join(c.benchDir, "_build", "js", "release", "build", "cmd", w, w+".js"))
	if err != nil {
		return 0, err
	}
	cmd := exec.Command("node", filepath.Join(c.runnerDir, "run-js.mjs"), "--no-profile", abs)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return 0, fmt.Errorf("%w: %s", err, out)
	}
	return parseMs(string(out))
}

func runNative(c *config, w string) (float64, error) {
	bin := filepath.Join(c.benchDir, "_build", "native", "release", "build", "cmd", w, w+".exe")
	start := time.Now()
	cmd := exec.Command(bin)
	if err := cmd.Run(); err != nil {
		return 0, err
	}
	return float64(time.Since(start).Microseconds()) / 1000.0, nil
}

func parseMs(s string) (float64, error) {
	m := msRe.FindStringSubmatch(s)
	if m == nil {
		return 0, fmt.Errorf("no ms value in: %s", strings.TrimSpace(s))
	}
	return strconv.ParseFloat(m[1], 64)
}

func median(xs []float64) float64 {
	c := append([]float64(nil), xs...)
	sort.Float64s(c)
	return c[len(c)/2]
}

func printMarkdown(c *config, results map[string]map[string]cell) {
	fmt.Println()
	fmt.Println("## Results")
	fmt.Println()
	// header
	fmt.Print("| workload |")
	for _, b := range c.backends {
		fmt.Printf(" %s base | %s patched | Δ |", b, b)
	}
	fmt.Println()
	fmt.Print("|---|")
	for range c.backends {
		fmt.Print("--:|--:|--:|")
	}
	fmt.Println()

	// stable workload order
	ws := make([]string, 0, len(results))
	for w := range results {
		ws = append(ws, w)
	}
	sort.Strings(ws)

	for _, w := range ws {
		fmt.Printf("| %s |", w)
		for _, b := range c.backends {
			ce := results[w][b]
			if ce.baseMs == 0 && ce.patchedMs == 0 {
				fmt.Print(" - | - | - |")
				continue
			}
			delta := ""
			if ce.baseMs > 0 {
				d := (ce.patchedMs - ce.baseMs) / ce.baseMs * 100
				delta = fmt.Sprintf("%+.1f%%", d)
			}
			fmt.Printf(" %.1f | %.1f | %s |", ce.baseMs, ce.patchedMs, delta)
		}
		fmt.Println()
	}
}

type cell struct {
	baseMs, patchedMs float64
}

func dirExists(p string) bool {
	if p == "" {
		return false
	}
	fi, err := os.Stat(p)
	return err == nil && fi.IsDir()
}

// swapMooncakes replaces benchDir/.mooncakes with a fresh copy of src.
// Used by the mooncakes-swap mode to flip between baseline and patched
// dependency states.
func swapMooncakes(benchDir, src string) error {
	dst := filepath.Join(benchDir, ".mooncakes")
	if err := os.RemoveAll(dst); err != nil {
		return fmt.Errorf("rm %s: %w", dst, err)
	}
	cmd := exec.Command("cp", "-r", src, dst)
	if out, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("cp -r %s %s: %w\n%s", src, dst, err, out)
	}
	// Make sure the new tree is user-writable in case the source was r-o.
	cmd = exec.Command("chmod", "-R", "u+w", dst)
	if out, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("chmod -R u+w %s: %w\n%s", dst, err, out)
	}
	return nil
}
