module github.com/mizchi/pprof-mbt/runners/wzprof-runner

go 1.22

require (
	github.com/google/pprof v0.0.0-20230406165453-00490a63f317
	github.com/mizchi/pprof-mbt/go/demangle v0.0.0-00010101000000-000000000000
	github.com/stealthrocket/wzprof v0.2.0
	github.com/tetratelabs/wazero v1.3.0
)

require (
	golang.org/x/exp v0.0.0-20230425010034-47ecfdc1ba53 // indirect
)

// Local resolution — also covered by go.work at the repo root, but the
// replace makes plain `go build` work without -workfile.
replace github.com/mizchi/pprof-mbt/go/demangle => ../../go/demangle
