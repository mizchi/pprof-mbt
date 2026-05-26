{
  description = "moon-pprof: profiling MoonBit across wasm-gc/wasm/js/native backends";

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

        # CLI build. `nix run github:mizchi/moon-pprof -- profile ...`
        # and `nix profile install github:mizchi/moon-pprof` both end up
        # here. Only the `moon-pprof` binary is exposed; the workspace's
        # `http-baseline-server` is dev-only and not built.
        moon-pprof = pkgs.rustPlatform.buildRustPackage {
          pname = "moon-pprof";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.protobuf ];
          cargoBuildFlags = [ "--bin" "moon-pprof" ];
          # The bench subcommand spawns `moon` / `node` / `samply` at
          # runtime; we don't pull those in here so the binary stays
          # self-contained. Users who run `moon-pprof bench` need the
          # devShell (or those tools elsewhere on $PATH).
          doCheck = true;
          meta = with pkgs.lib; {
            description = "Unified CLI for profiling MoonBit code across wasm-gc / wasm / js / native backends";
            homepage = "https://github.com/mizchi/moon-pprof";
            license = licenses.asl20;
            mainProgram = "moon-pprof";
            platforms = platforms.unix;
          };
        };

        moonPprofApp = {
          type = "app";
          program = "${moon-pprof}/bin/moon-pprof";
        };
      in {
        packages.default = moon-pprof;
        packages.moon-pprof = moon-pprof;

        apps.default = moonPprofApp;
        apps.moon-pprof = moonPprofApp;

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
