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
//! This crate provides those stdio imports plus the common exception, time,
//! string, string-array, and environment imports used by MoonBit test and
//! benchmark artifacts. The byte / code-unit buffers and guest-local
//! args/environment overlay live in caller-provided state via the
//! [`MoonbitStdio`] trait so the user keeps full control of the `Store<T>`
//! shape.
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
//! let mut store = Store::new(&engine, MyState { stdio: MoonbitStdioState::default() });
//! let mut linker: Linker<MyState> = Linker::new(&engine);
//! moonbit_wasm_host::register(&mut linker)?;
//! moonbit_wasm_host::register_store_imports(&mut linker, &mut store)?;
//! # Ok(()) }
//! ```
//!
//! For profiling MoonBit wasm with `wasmtime-guest-pprof`, combine this
//! with that crate's `ProfilerHost` trait on the same state struct.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use wasmtime::{
    bail, format_err, AsContextMut, Caller, Extern, ExternRef, FuncType, Linker,
    Result as WasmtimeResult, Rooted, Tag, TagType,
};

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
    /// Guest-visible command line arguments returned by
    /// `__moonbit_fs_unstable.args_get`. Defaults to an empty argv.
    pub args: Vec<String>,
    /// Guest-visible environment variable overrides. `Some(value)` means the
    /// key is set for the guest; `None` means the key is hidden from the guest.
    pub env_overrides: BTreeMap<String, Option<String>>,
}

enum MoonbitHostValue {
    Instant(Instant),
    String(String),
    StringWriter(Mutex<String>),
    StringReader(Mutex<StringReader>),
    StringArray(Arc<Vec<String>>),
    StringArrayReader(Mutex<StringArrayReader>),
}

struct StringReader {
    chars: Vec<u16>,
    pos: usize,
}

struct StringArrayReader {
    values: Arc<Vec<String>>,
    pos: usize,
}

const FFI_END_OF_STRING_ARRAY: &str = "ffi_end_of_/string_array";

/// Trait the wasmtime host state must implement so [`register`] can
/// reach its [`MoonbitStdioState`] from inside the host functions.
pub trait MoonbitStdio {
    /// Return a mutable reference to the line/byte buffers carried by
    /// this state.
    fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState;
}

/// Register store-owned MoonBit imports on `linker`.
///
/// Wasm exception tags are nominal runtime objects in Wasmtime, so they need
/// a `Store` at registration time. Call this in addition to [`register`] before
/// instantiating a MoonBit wasm module that imports `exception.tag`.
pub fn register_store_imports<T: MoonbitStdio + 'static>(
    linker: &mut Linker<T>,
    mut store: impl AsContextMut<Data = T>,
) -> Result<()> {
    let mut cx = store.as_context_mut();
    let ty = TagType::new(FuncType::new(cx.engine(), [], []));
    let tag = Tag::new(&mut cx, &ty)?;
    linker.define(&cx, "exception", "tag", tag)?;
    Ok(())
}

/// Register MoonBit-compatible host functions on `linker`:
///
/// * `spectest.print_char(i32)` — UTF-16 code units, flushes on `\n`
///   to `stdout`.
/// * `wasi_snapshot_preview1.fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> i32`
///   — minimal stub that walks the iovs, buffers per-line, writes
///   the byte count back, and flushes to `stdout` (fd=1) or `stderr`
///   (fd=2) on `\n`. Returns 0 on success, WASI-style errno on
///   malformed input (8 = BADF, 28 = EINVAL).
/// * `exception.throw()` — traps when MoonBit takes an exception path.
/// * `__moonbit_time_unstable.instant_now/instant_elapsed_as_secs_f64`
///   — monotonic timing for benchmark helpers.
/// * `__moonbit_time_unstable.now` — milliseconds since Unix epoch.
/// * `__moonbit_fs_unstable` string/string-array/env helpers used by core's
///   env package.
///
/// The state type `T` must implement [`MoonbitStdio`] so the closures
/// can borrow the line buffers.
pub fn register<T: MoonbitStdio + 'static>(linker: &mut Linker<T>) -> Result<()> {
    linker.func_wrap("exception", "throw", || -> WasmtimeResult<()> {
        bail!("MoonBit exception::throw")
    })?;
    linker.func_wrap(
        "__moonbit_sys_unstable",
        "exit",
        |_caller: Caller<'_, T>, code: i32| -> WasmtimeResult<()> {
            bail!("MoonBit sys.exit({})", code)
        },
    )?;
    linker.func_wrap(
        "__moonbit_time_unstable",
        "instant_now",
        |mut caller: Caller<'_, T>| -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let r = ExternRef::new(&mut caller, MoonbitHostValue::Instant(Instant::now()))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_time_unstable",
        "instant_elapsed_as_secs_f64",
        |caller: Caller<'_, T>, h: Option<Rooted<ExternRef>>| -> WasmtimeResult<f64> {
            with_host_value(&caller, h, |v| match v {
                MoonbitHostValue::Instant(t) => Ok(t.elapsed().as_secs_f64()),
                _ => bail!("instant_elapsed_as_secs_f64: wrong handle type"),
            })
        },
    )?;
    linker.func_wrap("__moonbit_time_unstable", "now", || -> i64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        millis.min(i64::MAX as u128) as i64
    })?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "begin_create_string",
        |mut caller: Caller<'_, T>| -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let r = ExternRef::new(
                &mut caller,
                MoonbitHostValue::StringWriter(Mutex::new(String::new())),
            )?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "string_append_char",
        |caller: Caller<'_, T>, h: Option<Rooted<ExternRef>>, ch: i32| -> WasmtimeResult<()> {
            with_host_value(&caller, h, |v| match v {
                MoonbitHostValue::StringWriter(writer) => {
                    let mut writer = writer.lock().expect("string writer lock poisoned");
                    let cu = (ch as u32 & 0xffff) as u16;
                    if let Some(ch) = char::from_u32(cu as u32) {
                        writer.push(ch);
                    } else {
                        writer.push('\u{fffd}');
                    }
                    Ok(())
                }
                _ => bail!("string_append_char: wrong handle type"),
            })
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "finish_create_string",
        |mut caller: Caller<'_, T>,
         h: Option<Rooted<ExternRef>>|
         -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let s = read_string(&caller, h)?;
            let r = ExternRef::new(&mut caller, MoonbitHostValue::String(s))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "begin_read_string",
        |mut caller: Caller<'_, T>,
         h: Option<Rooted<ExternRef>>|
         -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let chars: Vec<u16> = read_string(&caller, h)?.encode_utf16().collect();
            let r = ExternRef::new(
                &mut caller,
                MoonbitHostValue::StringReader(Mutex::new(StringReader { chars, pos: 0 })),
            )?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "string_read_char",
        |caller: Caller<'_, T>, h: Option<Rooted<ExternRef>>| -> WasmtimeResult<i32> {
            with_host_value(&caller, h, |v| match v {
                MoonbitHostValue::StringReader(reader) => {
                    let mut reader = reader.lock().expect("string reader lock poisoned");
                    if reader.pos >= reader.chars.len() {
                        Ok(-1)
                    } else {
                        let ch = reader.chars[reader.pos] as i32;
                        reader.pos += 1;
                        Ok(ch)
                    }
                }
                _ => bail!("string_read_char: wrong handle type"),
            })
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "finish_read_string",
        |_caller: Caller<'_, T>, _h: Option<Rooted<ExternRef>>| {},
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "begin_read_string_array",
        |mut caller: Caller<'_, T>,
         h: Option<Rooted<ExternRef>>|
         -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let values = read_string_array(&caller, h)?;
            let r = ExternRef::new(
                &mut caller,
                MoonbitHostValue::StringArrayReader(Mutex::new(StringArrayReader {
                    values,
                    pos: 0,
                })),
            )?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "string_array_read_string",
        |mut caller: Caller<'_, T>,
         h: Option<Rooted<ExternRef>>|
         -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let s = with_host_value(&caller, h, |v| match v {
                MoonbitHostValue::StringArrayReader(reader) => {
                    let mut reader = reader.lock().expect("string array reader lock poisoned");
                    if reader.pos >= reader.values.len() {
                        Ok(FFI_END_OF_STRING_ARRAY.to_string())
                    } else {
                        let s = reader.values[reader.pos].clone();
                        reader.pos += 1;
                        Ok(s)
                    }
                }
                _ => bail!("string_array_read_string: wrong handle type"),
            })?;
            let r = ExternRef::new(&mut caller, MoonbitHostValue::String(s))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "finish_read_string_array",
        |_caller: Caller<'_, T>, _h: Option<Rooted<ExternRef>>| {},
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "args_get",
        |mut caller: Caller<'_, T>| -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let args = Arc::new(caller.data_mut().moonbit_stdio().args.clone());
            let r = ExternRef::new(&mut caller, MoonbitHostValue::StringArray(args))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "current_dir",
        |mut caller: Caller<'_, T>| -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let cwd = std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let r = ExternRef::new(&mut caller, MoonbitHostValue::String(cwd))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "get_env_var",
        |mut caller: Caller<'_, T>,
         key: Option<Rooted<ExternRef>>|
         -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let key = read_string(&caller, key)?;
            let value = guest_env_var(caller.data_mut().moonbit_stdio(), &key).unwrap_or_default();
            let r = ExternRef::new(&mut caller, MoonbitHostValue::String(value))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "get_env_var_exists",
        |mut caller: Caller<'_, T>, key: Option<Rooted<ExternRef>>| -> WasmtimeResult<i32> {
            let key = read_string(&caller, key)?;
            Ok(
                if guest_env_var(caller.data_mut().moonbit_stdio(), &key).is_some() {
                    1
                } else {
                    0
                },
            )
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "get_env_vars",
        |mut caller: Caller<'_, T>| -> WasmtimeResult<Option<Rooted<ExternRef>>> {
            let values = Arc::new(guest_env_vars(caller.data_mut().moonbit_stdio()));
            let r = ExternRef::new(&mut caller, MoonbitHostValue::StringArray(values))?;
            Ok(Some(r))
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "set_env_var",
        |mut caller: Caller<'_, T>,
         key: Option<Rooted<ExternRef>>,
         value: Option<Rooted<ExternRef>>|
         -> WasmtimeResult<()> {
            let key = read_string(&caller, key)?;
            let value = read_string(&caller, value)?;
            caller
                .data_mut()
                .moonbit_stdio()
                .env_overrides
                .insert(key, Some(value));
            Ok(())
        },
    )?;
    linker.func_wrap(
        "__moonbit_fs_unstable",
        "unset_env_var",
        |mut caller: Caller<'_, T>, key: Option<Rooted<ExternRef>>| -> WasmtimeResult<()> {
            let key = read_string(&caller, key)?;
            caller
                .data_mut()
                .moonbit_stdio()
                .env_overrides
                .insert(key, None);
            Ok(())
        },
    )?;
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
                let buf_ptr =
                    u32::from_le_bytes(data[entry..entry + 4].try_into().unwrap()) as usize;
                let buf_len =
                    u32::from_le_bytes(data[entry + 4..entry + 8].try_into().unwrap()) as usize;
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

fn read_string<T>(caller: &Caller<'_, T>, h: Option<Rooted<ExternRef>>) -> WasmtimeResult<String> {
    let h = h.ok_or_else(|| format_err!("null externref"))?;
    let any = h
        .data(caller)?
        .ok_or_else(|| format_err!("externref data missing"))?;
    if let Some(s) = any.downcast_ref::<String>() {
        return Ok(s.clone());
    }
    let value = any
        .downcast_ref::<MoonbitHostValue>()
        .ok_or_else(|| format_err!("externref not MoonbitHostValue"))?;
    match value {
        MoonbitHostValue::String(s) => Ok(s.clone()),
        MoonbitHostValue::StringWriter(writer) => {
            Ok(writer.lock().expect("string writer lock poisoned").clone())
        }
        _ => bail!("expected MoonBit string handle"),
    }
}

fn read_string_array<T>(
    caller: &Caller<'_, T>,
    h: Option<Rooted<ExternRef>>,
) -> WasmtimeResult<Arc<Vec<String>>> {
    let h = h.ok_or_else(|| format_err!("null externref"))?;
    let any = h
        .data(caller)?
        .ok_or_else(|| format_err!("externref data missing"))?;
    let value = any
        .downcast_ref::<MoonbitHostValue>()
        .ok_or_else(|| format_err!("externref not MoonbitHostValue"))?;
    match value {
        MoonbitHostValue::StringArray(values) => Ok(values.clone()),
        _ => bail!("expected MoonBit string array handle"),
    }
}

fn with_host_value<T, R>(
    caller: &Caller<'_, T>,
    h: Option<Rooted<ExternRef>>,
    f: impl FnOnce(&MoonbitHostValue) -> WasmtimeResult<R>,
) -> WasmtimeResult<R> {
    let h = h.ok_or_else(|| format_err!("null externref"))?;
    let any = h
        .data(caller)?
        .ok_or_else(|| format_err!("externref data missing"))?;
    let value = any
        .downcast_ref::<MoonbitHostValue>()
        .ok_or_else(|| format_err!("externref not MoonbitHostValue"))?;
    f(value)
}

fn guest_env_var(state: &MoonbitStdioState, key: &str) -> Option<String> {
    match state.env_overrides.get(key) {
        Some(Some(value)) => Some(value.clone()),
        Some(None) => None,
        None => std::env::var(key).ok(),
    }
}

fn guest_env_vars(state: &MoonbitStdioState) -> Vec<String> {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    for (key, value) in &state.env_overrides {
        match value {
            Some(value) => {
                env.insert(key.clone(), value.clone());
            }
            None => {
                env.remove(key);
            }
        }
    }
    env.into_iter()
        .flat_map(|(key, value)| [key, value])
        .collect()
}
