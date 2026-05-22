// wzprof-runner profiles a moonbit wasm (non-gc) binary.
//
// `wzprof` the CLI assumes a WASI-style guest, but moonbit's wasm target
// imports `spectest.print_char` (moonrun's host convention). We embed
// wzprof as a library, provide that import ourselves, and write a pprof
// CPU profile.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"os"

	"github.com/google/pprof/profile"
	"github.com/stealthrocket/wzprof"
	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/api"
	"github.com/tetratelabs/wazero/experimental"

	"github.com/mizchi/pprof-mbt/go/demangle"
)

func demangleProfile(p *profile.Profile) {
	for _, fn := range p.Function {
		pretty := demangle.Symbol(fn.Name)
		if pretty != fn.Name {
			if fn.SystemName == "" || fn.SystemName == fn.Name {
				fn.SystemName = fn.Name
			}
			fn.Name = pretty
		}
	}
}

func main() {
	cpuPath := flag.String("cpuprofile", "wasm-wzprof.pb.gz", "CPU profile output path")
	memPath := flag.String("memprofile", "", "Memory profile output path (optional)")
	rate := flag.Float64("sample", 1.0, "Sampling rate (0..1)")
	iterations := flag.Int("iter", 1, "How many times to run the guest")
	flag.Parse()
	if flag.NArg() != 1 {
		log.Fatalf("usage: %s [flags] <module.wasm>", os.Args[0])
	}
	wasmPath := flag.Arg(0)
	wasmCode, err := os.ReadFile(wasmPath)
	if err != nil {
		log.Fatalf("read wasm: %v", err)
	}

	ctx := context.Background()
	p := wzprof.ProfilingFor(wasmCode)
	cpu := p.CPUProfiler()
	listeners := []experimental.FunctionListenerFactory{
		wzprof.Sample(*rate, cpu),
	}
	var mem *wzprof.MemoryProfiler
	if *memPath != "" {
		mem = p.MemoryProfiler()
		listeners = append(listeners, wzprof.Sample(*rate, mem))
	}
	ctx = context.WithValue(ctx,
		experimental.FunctionListenerFactoryKey{},
		experimental.MultiFunctionListenerFactory(listeners...),
	)

	runtime := wazero.NewRuntime(ctx)
	defer runtime.Close(ctx)

	compiled, err := runtime.CompileModule(ctx, wasmCode)
	if err != nil {
		log.Fatalf("compile: %v", err)
	}
	if err := p.Prepare(compiled); err != nil {
		log.Fatalf("prepare: %v", err)
	}

	// moonbit's wasm target emits println as UTF-16 code units, one per call
	// to spectest.print_char.
	var line []rune
	_, err = runtime.NewHostModuleBuilder("spectest").
		NewFunctionBuilder().
		WithFunc(func(ctx context.Context, code uint32) {
			if code == 10 {
				fmt.Println(string(line))
				line = line[:0]
			} else {
				line = append(line, rune(code))
			}
		}).
		Export("print_char").
		Instantiate(ctx)
	if err != nil {
		log.Fatalf("spectest module: %v", err)
	}

	cpu.StartProfile()

	for i := 0; i < *iterations; i++ {
		mod, err := runtime.InstantiateModule(ctx, compiled,
			wazero.NewModuleConfig().WithName(fmt.Sprintf("main-%d", i)))
		if err != nil {
			log.Fatalf("instantiate: %v", err)
		}
		// moonbit wasm exports _start; otherwise start function runs at instantiate
		if start := mod.ExportedFunction("_start"); start != nil {
			if _, err := start.Call(ctx); err != nil {
				log.Fatalf("call _start: %v", err)
			}
		}
		mod.Close(ctx)
	}

	cpuProf := cpu.StopProfile(*rate)
	demangleProfile(cpuProf)
	if err := wzprof.WriteProfile(*cpuPath, cpuProf); err != nil {
		log.Fatalf("write cpu profile: %v", err)
	}
	fmt.Fprintf(os.Stderr, "[wzprof] cpu profile → %s (%d samples)\n", *cpuPath, len(cpuProf.Sample))

	if mem != nil {
		memProf := mem.NewProfile(*rate)
		if err := wzprof.WriteProfile(*memPath, memProf); err != nil {
			log.Fatalf("write mem profile: %v", err)
		}
		fmt.Fprintf(os.Stderr, "[wzprof] mem profile → %s\n", *memPath)
	}
	_ = api.ValueTypeI32
}
