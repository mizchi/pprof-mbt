{
  description = "pprof-mbt: profiling MoonBit across wasm-gc/wasm/js/native backends";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    moonbit-overlay.url = "github:moonbit-community/moonbit-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, moonbit-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ moonbit-overlay.overlays.default ];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            moonbit-bin.moonbit.latest
            nodejs_22
            wabt        # wasm2wat etc.
            binaryen    # wasm-opt
            go          # only for `go tool pprof` visualization; no Go code in-repo
            graphviz    # pprof -svg needs dot
            samply      # macOS/Linux sampling profiler → Firefox JSON, converted to pprof
            gperftools  # libprofiler.dylib (DYLD_INSERT_LIBRARIES route)
            wasmtime    # for CLI use + reference for wasmtime-runner
            cargo       # builds runners/wasmtime-runner / pprof-summary / bench-runner
            rustc
            protobuf    # protoc for prost-build (pprof emission)
          ];

          shellHook = ''
            echo "moonbit:  $(moon version 2>/dev/null || echo not found)"
            echo "node:     $(node --version)"
            echo "go pprof: $(go tool pprof -h >/dev/null 2>&1 && echo ok || echo missing)"
          '';
        };
      });
}
