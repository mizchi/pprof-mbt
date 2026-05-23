//! Wasmtime host imports for MoonBit-compiled wasm guests.
//!
//! MoonBit's `wasm` target emits one of two import flavors for `println`:
//!
//! 1. **Legacy moonrun style** — calls `spectest.print_char(i32)` once per
//!    UTF-16 code unit, with a final `\n` flushing the buffered line. This
//!    matches what `moonrun` (the official MoonBit interpreter) provides.
//!
//! 2. **Modern WASI style** — calls `wasi_snapshot_preview1.fd_write` with
//!    iovs of UTF-8 bytes. Newer toolchains default to this.
//!
//! This crate provides both imports in a single [`register`] call so a
//! wasmtime app can run a MoonBit wasm out of the box. The byte / code-unit
//! buffers live in caller-provided state via the [`MoonbitStdio`] trait so
//! the user keeps full control of the `Store<T>` shape.
//!
//! ## Example
//!
//! ```no_run
//! use wasmtime::{Engine, Linker, Module, Store};
//! use moonbit_wasm_host::{MoonbitStdio, MoonbitStdioState};
//!
//! struct MyState {
//!     stdio: MoonbitStdioState,
//! }
//! impl MoonbitStdio for MyState {
//!     fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState { &mut self.stdio }
//! }
//!
//! # fn doctest() -> anyhow::Result<()> {
//! let engine = Engine::default();
//! let mut linker: Linker<MyState> = Linker::new(&engine);
//! moonbit_wasm_host::register(&mut linker)?;
//! # Ok(()) }
//! ```
//!
//! For profiling MoonBit wasm with `wasmtime-guest-pprof`, combine this
//! with that crate's `ProfilerHost` trait on the same state struct.

use anyhow::Result;
use wasmtime::{Caller, Extern, Linker};

/// Buffers held on the host state to assemble whole lines from either of
/// the two import flavors before flushing to stdout/stderr.
///
/// Owned by the user's `Store<T>` data. Construct with [`Default::default`].
#[derive(Default)]
pub struct MoonbitStdioState {
    /// UTF-16 code units accumulated by `spectest.print_char` until '\n'.
    pub line_utf16: Vec<u16>,
    /// UTF-8 bytes accumulated by `wasi_snapshot_preview1.fd_write` until '\n'.
    pub stdout_bytes: Vec<u8>,
}

/// Trait the wasmtime host state must implement so [`register`] can
/// reach its [`MoonbitStdioState`] from inside the host functions.
pub trait MoonbitStdio {
    /// Return a mutable reference to the line/byte buffers carried by
    /// this state.
    fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState;
}

/// Register both MoonBit-compatible stdio imports on `linker`:
///
/// * `spectest.print_char(i32)` — UTF-16 code units, flushes on `\n`
///   to `stdout`.
/// * `wasi_snapshot_preview1.fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> i32`
///   — minimal stub that walks the iovs, buffers per-line, writes
///   the byte count back, and flushes to `stdout` (fd=1) or `stderr`
///   (fd=2) on `\n`. Returns 0 on success, WASI-style errno on
///   malformed input (8 = BADF, 28 = EINVAL).
///
/// The state type `T` must implement [`MoonbitStdio`] so the closures
/// can borrow the line buffers.
pub fn register<T: MoonbitStdio + 'static>(linker: &mut Linker<T>) -> Result<()> {
    linker.func_wrap(
        "spectest",
        "print_char",
        |mut caller: Caller<'_, T>, code: i32| {
            let state = caller.data_mut().moonbit_stdio();
            if code == b'\n' as i32 {
                println!("{}", String::from_utf16_lossy(&state.line_utf16));
                state.line_utf16.clear();
            } else {
                state.line_utf16.push(code as u16);
            }
        },
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, T>,
         fd: i32,
         iovs_ptr: i32,
         iovs_len: i32,
         nwritten_ptr: i32|
         -> i32 {
            let mem = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return 8, // BADF
            };
            let mut total: u32 = 0;
            let mut payload: Vec<u8> = Vec::new();
            let data = mem.data(&caller);
            for i in 0..iovs_len as usize {
                let entry = iovs_ptr as usize + i * 8;
                if entry + 8 > data.len() {
                    return 28;
                }
                let buf_ptr = u32::from_le_bytes(data[entry..entry + 4].try_into().unwrap()) as usize;
                let buf_len = u32::from_le_bytes(data[entry + 4..entry + 8].try_into().unwrap()) as usize;
                if buf_ptr + buf_len > data.len() {
                    return 28;
                }
                payload.extend_from_slice(&data[buf_ptr..buf_ptr + buf_len]);
                total += buf_len as u32;
            }
            let nwritten_bytes = total.to_le_bytes();
            let mem_mut = mem.data_mut(&mut caller);
            let np = nwritten_ptr as usize;
            if np + 4 <= mem_mut.len() {
                mem_mut[np..np + 4].copy_from_slice(&nwritten_bytes);
            }
            let state = caller.data_mut().moonbit_stdio();
            state.stdout_bytes.extend_from_slice(&payload);
            while let Some(nl) = state.stdout_bytes.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = state.stdout_bytes.drain(..=nl).collect();
                let s = String::from_utf8_lossy(&line[..line.len() - 1]);
                if fd == 2 {
                    eprintln!("{}", s);
                } else {
                    println!("{}", s);
                }
            }
            0
        },
    )?;
    Ok(())
}
