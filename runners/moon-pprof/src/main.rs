//! moon-pprof: unified CLI for MoonBit profiling.
//!
//! Subcommands:
//!   * `profile <wasm>` — run wasm under wasmtime + GuestProfiler, emit pprof
//!   * `summary <file>` — print self-time / mem-mgmt rollup
//!   * `summary --diff <a> <b>` — diff two profiles at function granularity
//!   * `bench` — drive cross-backend benches (baseline ↔ patched)
//!
//! Replaces three previous binaries (wasmtime-runner, pprof-summary,
//! bench-runner). The implementation of each subcommand lives in its
//! own module under this crate.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod cmd_bench;
mod cmd_cpuprofile2pprof;
mod cmd_firefox2pprof;
mod cmd_heapprofile2pprof;
mod cmd_memprofile;
mod cmd_memprofile_native;
mod cmd_perf2pprof;
mod cmd_profile;
mod cmd_summary;

#[derive(Parser, Debug)]
#[command(name = "moon-pprof", about = "Profile MoonBit code across native / wasm-gc / wasm / js backends.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a wasm binary under wasmtime + GuestProfiler and emit gzip'd pprof.
    Profile(cmd_profile::Args),
    /// Print top-N self-time and mem-mgmt rollup for a pprof file, or diff two.
    Summary(cmd_summary::Args),
    /// Drive a set of MoonBit benches across backends and emit a markdown delta table.
    Bench(cmd_bench::Args),
    /// Convert a Node V8 `.cpuprofile` into gzip'd pprof.
    Cpuprofile2pprof(cmd_cpuprofile2pprof::Args),
    /// Convert a Firefox Profiler JSON (samply / wasmtime) into gzip'd pprof.
    Firefox2pprof(cmd_firefox2pprof::Args),
    /// Convert a Node V8 `.heapprofile` (sampling allocations) into gzip'd pprof.
    Heapprofile2pprof(cmd_heapprofile2pprof::Args),
    /// Capture an allocation profile of a MoonBit wasm by instrumenting moonbit.malloc.
    Memprofile(cmd_memprofile::Args),
    /// Capture an allocation profile of a MoonBit native binary by patching its generated <cmd>.c and relinking with a hook.
    MemprofileNative(cmd_memprofile_native::Args),
    /// Convert Linux `perf script` textual output into gzip'd pprof.
    Perf2pprof(cmd_perf2pprof::Args),
}

fn main() -> ExitCode {
    // Default SIGPIPE handling on Unix is "ignore", which makes println!
    // panic when a downstream consumer (like `head`) closes the pipe.
    // Restore the inherit-from-shell default so the process exits silently.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // bench-runner historically accepted single-dash long flags (`-runs 3`)
    // from its Go ancestry. Preserve that for `moon-pprof bench` by
    // pre-normalizing the args before clap sees them.
    let argv = normalize_argv(std::env::args().collect());
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            e.print().ok();
            return ExitCode::from(2);
        }
    };

    let result = match cli.command {
        Command::Profile(a) => cmd_profile::run(a),
        Command::Summary(a) => cmd_summary::run(a),
        Command::Bench(a) => cmd_bench::run(a),
        Command::Cpuprofile2pprof(a) => cmd_cpuprofile2pprof::run(a),
        Command::Firefox2pprof(a) => cmd_firefox2pprof::run(a),
        Command::Heapprofile2pprof(a) => cmd_heapprofile2pprof::run(a),
        Command::Memprofile(a) => cmd_memprofile::run(a),
        Command::MemprofileNative(a) => cmd_memprofile_native::run(a),
        Command::Perf2pprof(a) => cmd_perf2pprof::run(a),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("moon-pprof: {:#}", e);
            ExitCode::FAILURE
        }
    }
}

/// Translate single-leading-dash long options (`-foo`, `-foo=bar`) to
/// double-dash so clap accepts them. Single-char short flags (`-h`) stay.
fn normalize_argv(argv: Vec<String>) -> Vec<String> {
    argv.into_iter()
        .map(|a| {
            if a.starts_with("--") || !a.starts_with('-') {
                return a;
            }
            let inner = &a[1..];
            if inner.len() <= 1 {
                return a;
            }
            format!("--{}", inner)
        })
        .collect()
}
