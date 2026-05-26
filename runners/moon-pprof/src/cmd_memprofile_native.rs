//! `moon-pprof memprofile-native <path/to/main.exe>` — allocation
//! profile of a MoonBit `--target native` binary. Works on macOS
//! (Mach-O) and Linux (glibc ELF — verified in
//! `notes/linux-memprofile/`).
//!
//! ## Why this is more work than the wasm path
//!
//! MoonBit's native runtime statically links mimalloc into every
//! binary. User code's `moonbit_malloc(size)` macro-expands to
//! `moonbit_malloc_inlined(size)`, which calls `libc_malloc` →
//! `malloc`, and the linker binds that `malloc` to mimalloc's local
//! symbol (not libSystem's). DYLD interpose / LD_PRELOAD never sees
//! those calls — verified by counting events (~190 across libsystem
//! startup, zero from JSON parsing 197k chars × 50 iterations).
//!
//! So instead of interposing the allocator at run-time, we instrument
//! at compile-time: patch the generated `main.c` so the inline malloc
//! calls a `__moon_pprof_alloc_hook(size_t)` *before* the real
//! `libc_malloc`, link in our hook .o that captures `backtrace()` +
//! `dladdr()` and dumps a raw stream, then convert the stream to pprof
//! on the Rust side after the user binary exits.
//!
//! ## End-to-end flow
//!
//! 1. From the `.exe` path, walk up to the MoonBit project root
//!    (the directory containing `moon.mod.json`).
//! 2. Run `moon build --target native --release` to ensure the
//!    generated `.c` and the original `.exe` are up to date.
//! 3. Run `moon build --target native --release --dry-run` and grep
//!    for the `cc … -o …/<name>.exe` line for our target.
//! 4. Read the generated `.c`, replace the body of
//!    `moonbit_malloc_inlined` with one that calls
//!    `__moon_pprof_alloc_hook`, write it to a sibling
//!    `<name>.memprof.c`.
//! 5. Compile our bundled `native_alloc_hook.c` with the same `cc` to
//!    a sibling `.memprof_hook.o`.
//! 6. Re-run the original `cc` command with the patched `.c` swapped
//!    in for the original and the hook `.o` appended, output to
//!    `<name>.memprof.exe`.
//! 7. Run `<name>.memprof.exe` with `MOON_PPROF_RAW_OUTPUT` +
//!    `MOON_PPROF_SAMPLE_RATE`.
//! 8. Read the raw record stream, aggregate `(frames → count, bytes)`,
//!    demangle MoonBit symbols, encode pprof, gzip, write to `--out`.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use clap::Parser;
use flate2::Compression;
use flate2::write::GzEncoder;
use prost::Message;

use firefox_to_pprof::proto;

const HOOK_SOURCE: &str = include_str!("../assets/native_alloc_hook.c");
const HOOK_SYMBOL: &str = "__moon_pprof_alloc_hook";

#[derive(Parser, Debug)]
#[command(
    about = "Capture an allocation profile of a MoonBit native binary by patching the generated <cmd>.c and relinking with a backtrace-capturing hook."
)]
pub struct Args {
    /// Path to the `<cmd>.exe` produced by `moon build --target native --release`.
    pub exe: PathBuf,
    /// Output path for the gzip'd pprof.
    #[arg(long, default_value = "native-mem.pb.gz")]
    pub out: PathBuf,
    /// Capture 1/N allocations. Default 1 (every alloc). Matches the
    /// `--sample-rate` flag on the wasm `memprofile` subcommand.
    #[arg(long, default_value_t = 1)]
    pub sample_rate: u32,
    /// Pass raw mangled symbols through instead of running them through
    /// `moonbit_demangle::demangle`.
    #[arg(long)]
    pub no_demangle: bool,
    /// Send SIGTERM to the patched binary after this many seconds. Use
    /// for servers / event loops that never return on their own — the
    /// linked hook installs a SIGTERM/SIGINT handler that flushes the
    /// raw stream before `_exit(0)`. Default 0 = wait forever for the
    /// binary to exit on its own.
    #[arg(long, default_value_t = 0)]
    pub duration: u64,
    /// Extra arguments forwarded to the user binary after the env vars
    /// are set. Use `--` to separate from moon-pprof's own flags.
    #[arg(last = true)]
    pub forward: Vec<String>,
}

pub fn run(args: Args) -> Result<()> {
    let original_exe = args.exe.canonicalize().with_context(|| {
        format!("looking up native exe at {}", args.exe.display())
    })?;
    let out_abs = absolutize(&args.out)?;
    let sample_rate = args.sample_rate.max(1);

    let project = find_project_root(&original_exe)?;
    let cmd_name = derive_cmd_name(&original_exe)?;
    let generated_c = original_exe
        .with_extension("c")
        .with_file_name(format!("{cmd_name}.c"));
    if !generated_c.exists() {
        bail!(
            "expected generated C source at {} alongside the .exe",
            generated_c.display()
        );
    }

    eprintln!(
        "[moon-pprof memprofile-native] project={} cmd={}",
        project.display(),
        cmd_name,
    );

    // Step 1: make sure the build is current. Cheap if already built.
    let status = Command::new("moon")
        .current_dir(&project)
        .args(["build", "--target", "native", "--release"])
        .status()
        .context("running `moon build --target native --release`")?;
    if !status.success() {
        bail!("`moon build` failed");
    }

    // Step 2: capture cc commands.
    let dry_run = Command::new("moon")
        .current_dir(&project)
        .args(["build", "--target", "native", "--release", "--dry-run"])
        .output()
        .context("running `moon build … --dry-run`")?;
    if !dry_run.status.success() {
        bail!(
            "`moon build --dry-run` failed: {}",
            String::from_utf8_lossy(&dry_run.stderr),
        );
    }
    let dry_text = String::from_utf8_lossy(&dry_run.stdout);

    let exe_filename = original_exe
        .file_name()
        .ok_or_else(|| anyhow!("exe has no filename"))?
        .to_string_lossy()
        .into_owned();
    let cc_line = find_cc_line_for_exe(&dry_text, &exe_filename).ok_or_else(|| {
        anyhow!(
            "could not find the cc command for {exe_filename} in `moon build --dry-run` output; \
             is this binary built with `moon build --target native --release`?"
        )
    })?;

    let moon_home = env::var("MOON_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs_home().map(|h| h.join(".moon")))
        .ok_or_else(|| anyhow!("could not determine MOON_HOME (set $MOON_HOME)"))?;
    // `moon build --dry-run` uses `$MOON_TOOLCHAIN_ROOT` for paths
    // when invoked from a patched/relocated toolchain (e.g. our
    // /tmp/moonbit-patched test setup), and `$MOON_HOME` otherwise.
    // Substitute both so the relink works in either layout.
    let toolchain_root = env::var("MOON_TOOLCHAIN_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| moon_home.clone());
    let cc_argv = shell_split(&cc_line)?
        .into_iter()
        .map(|s| {
            s.replace("$MOON_TOOLCHAIN_ROOT", &toolchain_root.to_string_lossy())
                .replace("$MOON_HOME", &moon_home.to_string_lossy())
        })
        .collect::<Vec<_>>();
    if cc_argv.is_empty() {
        bail!("empty cc command parsed from dry-run line: {cc_line}");
    }

    // Step 3: patch generated .c.
    let original_c_text = fs::read_to_string(&generated_c)
        .with_context(|| format!("reading generated C at {}", generated_c.display()))?;
    let patched_c_text = patch_moonbit_malloc(&original_c_text)?;
    let patched_c_path = generated_c.with_file_name(format!("{cmd_name}.memprof.c"));
    fs::write(&patched_c_path, &patched_c_text)
        .with_context(|| format!("writing patched C to {}", patched_c_path.display()))?;

    // Step 4: extract & compile hook.c.
    let hook_c_path = patched_c_path.with_file_name(format!("{cmd_name}.memprof_hook.c"));
    let hook_o_path = patched_c_path.with_file_name(format!("{cmd_name}.memprof_hook.o"));
    fs::write(&hook_c_path, HOOK_SOURCE)
        .with_context(|| format!("writing hook source to {}", hook_c_path.display()))?;
    let cc = &cc_argv[0];
    let cc_flags = extract_compile_flags(&cc_argv);
    let hook_compile = Command::new(cc)
        .args(&cc_flags)
        .args(["-c", "-o"])
        .arg(&hook_o_path)
        .arg(&hook_c_path)
        .current_dir(&project)
        .status()
        .with_context(|| format!("compiling hook with {cc}"))?;
    if !hook_compile.success() {
        bail!("hook .c compile failed");
    }

    // Step 5: relink.
    let memprof_exe_path = original_exe.with_file_name(format!("{cmd_name}.memprof.exe"));
    let relinked_argv = rebuild_cc_argv(
        &cc_argv,
        &generated_c,
        &patched_c_path,
        &original_exe,
        &memprof_exe_path,
        &hook_o_path,
    )?;

    let relink = Command::new(&relinked_argv[0])
        .args(&relinked_argv[1..])
        .current_dir(&project)
        .status()
        .context("relinking patched binary")?;
    if !relink.success() {
        bail!("relink failed");
    }

    // Step 6: run. When --duration is set we spawn + wait so we can
    // send SIGTERM ourselves; the hook's signal handler flushes the
    // raw stream and then _exit(0)s, so the file is complete by the
    // time wait() returns.
    let raw_path = env::temp_dir().join(format!(
        "moon-pprof-native-raw.{}.bin",
        std::process::id()
    ));
    let _ = fs::remove_file(&raw_path);
    let t0 = Instant::now();
    let mut child = Command::new(&memprof_exe_path)
        .args(&args.forward)
        .env("MOON_PPROF_RAW_OUTPUT", &raw_path)
        .env("MOON_PPROF_SAMPLE_RATE", sample_rate.to_string())
        .spawn()
        .with_context(|| format!("spawning {}", memprof_exe_path.display()))?;
    let run_status = if args.duration > 0 {
        let pid = child.id();
        let secs = args.duration;
        let timer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            // SAFETY: kill(pid, SIGTERM) is async-signal-safe; pid is
            // owned by us (we haven't reaped it yet).
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        });
        let status = child.wait().context("waiting for patched binary")?;
        // Drain the timer regardless; if the binary exited before the
        // deadline the thread is still sleeping. Detach by ignoring it.
        let _ = timer.join();
        status
    } else {
        child.wait().context("waiting for patched binary")?
    };
    let elapsed = t0.elapsed();
    // With --duration the hook exits via _exit(0) → status is success.
    // Without it, if the user binary itself failed, surface that but
    // still try to parse whatever raw stream we got.
    if !run_status.success() && args.duration == 0 {
        bail!(
            "patched binary exited with {run_status}; raw at {} may be partial",
            raw_path.display(),
        );
    }
    if !raw_path.exists() {
        bail!(
            "no raw output at {} — did the hook .o get linked? \
             (check for `__moon_pprof_alloc_hook` symbol in {})",
            raw_path.display(),
            memprof_exe_path.display(),
        );
    }

    // Step 7: parse → aggregate → encode pprof.
    let raw_bytes = fs::read(&raw_path).context("reading raw alloc stream")?;
    let samples = parse_raw_stream(&raw_bytes, sample_rate)?;
    let pprof = encode_pprof(samples, args.no_demangle)?;
    if let Some(parent) = out_abs.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&out_abs, &pprof)
        .with_context(|| format!("writing pprof to {}", out_abs.display()))?;

    // Cleanup the intermediate raw stream; keep the patched .c /
    // memprof.exe around so the user can re-run or inspect.
    let _ = fs::remove_file(&raw_path);

    eprintln!(
        "[moon-pprof memprofile-native] {} in {:.2?} (sample-rate={}) → {}",
        memprof_exe_path.display(),
        elapsed,
        sample_rate,
        out_abs.display(),
    );
    Ok(())
}

// ──────────────────────── project + cc-line plumbing ────────────────────

fn find_project_root(exe: &Path) -> Result<PathBuf> {
    let mut p = exe.to_path_buf();
    while p.pop() {
        if p.join("moon.mod.json").exists() {
            return Ok(p);
        }
    }
    Err(anyhow!(
        "could not find moon.mod.json walking up from {}",
        exe.display()
    ))
}

fn derive_cmd_name(exe: &Path) -> Result<String> {
    Ok(exe
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("exe path has no stem"))?
        .to_string())
}

fn dirs_home() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn find_cc_line_for_exe(dry_text: &str, exe_filename: &str) -> Option<String> {
    // We're looking for a cc line whose `-o <path>` ends in our exe
    // filename. The dry-run output uses single-line cc commands, so
    // a substring match is sufficient.
    let needle = format!("/{exe_filename}");
    for line in dry_text.lines() {
        let trimmed = line.trim();
        if !is_cc_invocation(trimmed) {
            continue;
        }
        if trimmed.contains(&needle) || trimmed.ends_with(exe_filename) {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn is_cc_invocation(line: &str) -> bool {
    // moon emits absolute cc paths (e.g. `/usr/bin/cc …`). We also
    // accept bare `cc` / `gcc` / `clang` to keep this flexible if moon
    // changes its toolchain.
    let first = line.split_ascii_whitespace().next().unwrap_or("");
    first.ends_with("/cc")
        || first.ends_with("/gcc")
        || first.ends_with("/clang")
        || first == "cc"
        || first == "gcc"
        || first == "clang"
}

fn shell_split(line: &str) -> Result<Vec<String>> {
    // The dry-run output uses POSIX single-quote escaping for paths
    // containing `$` (e.g. `'$MOON_HOME/lib/runtime.c'`). A
    // hand-rolled splitter that handles single-quoted segments is
    // enough — moon never emits double-quoted args or backslash
    // escapes for these commands.
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                cur.push(c);
            }
        } else {
            match c {
                '\'' => in_single = true,
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(c),
            }
        }
    }
    if in_single {
        bail!("unterminated single-quote in cc command: {line}");
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// Pluck the cc flags (everything that looks like an `-I…`, `-D…`,
/// `-f…`, `-W…`, `-O…`, `-g`, `-std=…`) out of a full cc command so we
/// can pass the same flag set to our hook compile. Skips inputs, outputs,
/// link-only flags, and library names.
fn extract_compile_flags(argv: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 1; // skip cc
    while i < argv.len() {
        let a = &argv[i];
        if a == "-o" {
            i += 2;
            continue;
        }
        if a == "-c" {
            i += 1;
            continue;
        }
        if a.starts_with("-l") {
            i += 1;
            continue;
        }
        if a.starts_with("-L") {
            i += 1;
            continue;
        }
        if a.ends_with(".c")
            || a.ends_with(".o")
            || a.ends_with(".a")
            || a.ends_with(".dylib")
            || a.ends_with(".so")
        {
            i += 1;
            continue;
        }
        if a.starts_with('-') {
            out.push(a.clone());
        }
        i += 1;
    }
    out
}

fn rebuild_cc_argv(
    cc_argv: &[String],
    original_c: &Path,
    patched_c: &Path,
    original_exe: &Path,
    memprof_exe: &Path,
    hook_o: &Path,
) -> Result<Vec<String>> {
    // Substitute the original .c with the patched one and the original
    // -o target with the memprof.exe, then append the hook .o.
    let orig_c_str = original_c.to_string_lossy().into_owned();
    let orig_c_name = original_c
        .file_name()
        .ok_or_else(|| anyhow!("original .c has no filename"))?
        .to_string_lossy()
        .into_owned();
    let orig_exe_str = original_exe.to_string_lossy().into_owned();
    let orig_exe_name = original_exe
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let mut out: Vec<String> = Vec::with_capacity(cc_argv.len() + 2);
    let mut i = 0;
    while i < cc_argv.len() {
        let a = &cc_argv[i];
        if a == "-o" {
            out.push(a.clone());
            // Next arg is the original .exe path; rewrite.
            if i + 1 >= cc_argv.len() {
                bail!("cc command ends mid -o");
            }
            let exe_arg = &cc_argv[i + 1];
            if exe_arg == &orig_exe_str
                || exe_arg.ends_with(&format!("/{orig_exe_name}"))
                || exe_arg.ends_with(&orig_exe_name)
            {
                out.push(memprof_exe.to_string_lossy().into_owned());
            } else {
                out.push(exe_arg.clone());
            }
            i += 2;
            continue;
        }
        if a == &orig_c_str
            || a.ends_with(&format!("/{orig_c_name}"))
            || a.ends_with(&orig_c_name)
        {
            out.push(patched_c.to_string_lossy().into_owned());
            i += 1;
            continue;
        }
        out.push(a.clone());
        i += 1;
    }
    out.push(hook_o.to_string_lossy().into_owned());
    // Linux ELF only exports symbols listed in the dynamic table; static
    // / `extern` C functions defined in the same exe are invisible to
    // `dladdr` without `-rdynamic`. The hook also needs libdl
    // (`dladdr`) and libpthread (the mutex). macOS Mach-O exports
    // everything by default and links dl/pthread implicitly.
    if cfg!(target_os = "linux") {
        out.push("-rdynamic".to_string());
        out.push("-ldl".to_string());
        out.push("-lpthread".to_string());
    }
    Ok(out)
}

// ──────────────────────── source patcher ────────────────────────────────

fn patch_moonbit_malloc(src: &str) -> Result<String> {
    // We need to inject a call to `__moon_pprof_alloc_hook(size)`
    // *before* the libc_malloc call inside `moonbit_malloc_inlined`.
    // Approach: insert a forward declaration up top, then rewrite the
    // function body. The runtime's body is short and stable across
    // versions:
    //
    //   static void *moonbit_malloc_inlined(size_t size) {
    //     struct moonbit_object *ptr = (struct moonbit_object *)libc_malloc(
    //         sizeof(struct moonbit_object) + size);
    //     ptr->rc = 1;
    //     return ptr + 1;
    //   }
    //
    // We replace the whole function in one shot so we don't have to
    // track moonbit's exact whitespace.

    let signature = "static void *moonbit_malloc_inlined(size_t size) {";
    let start = src
        .find(signature)
        .ok_or_else(|| anyhow!("could not find `moonbit_malloc_inlined` definition in generated C"))?;
    // Walk forward to find the matching close brace at depth 0.
    let body_start = start + signature.len();
    let mut depth = 1; // we're already inside the `{`
    let mut end = body_start;
    for (i, ch) in src[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        bail!("unbalanced braces while scanning moonbit_malloc_inlined");
    }

    let replacement = format!(
        "static void *moonbit_malloc_inlined(size_t size) {{\n  \
            extern void {HOOK_SYMBOL}(size_t);\n  \
            {HOOK_SYMBOL}(size);\n  \
            struct moonbit_object *ptr = (struct moonbit_object *)libc_malloc(\n        \
                sizeof(struct moonbit_object) + size);\n  \
            ptr->rc = 1;\n  \
            return ptr + 1;\n\
        }}"
    );

    let mut out = String::with_capacity(src.len() + replacement.len());
    out.push_str(&src[..start]);
    out.push_str(&replacement);
    out.push_str(&src[end..]);

    Ok(out)
}

// ──────────────────────── raw stream parser ─────────────────────────────

#[derive(Default)]
struct SampleMap(HashMap<Vec<String>, SampleAgg>);

#[derive(Default, Clone, Copy)]
struct SampleAgg {
    count: i64,
    bytes: i64,
}

fn parse_raw_stream(buf: &[u8], sample_rate: u32) -> Result<SampleMap> {
    let mut samples = SampleMap::default();
    let mut p = 0usize;
    let scale = sample_rate.max(1) as i64;
    while p + 9 <= buf.len() {
        let size = u64::from_le_bytes(buf[p..p + 8].try_into().unwrap());
        p += 8;
        let nframes = buf[p] as usize;
        p += 1;
        let mut frames: Vec<String> = Vec::with_capacity(nframes);
        let mut ok = true;
        for _ in 0..nframes {
            if p + 2 > buf.len() {
                ok = false;
                break;
            }
            let name_len = u16::from_le_bytes(buf[p..p + 2].try_into().unwrap()) as usize;
            p += 2;
            if p + name_len > buf.len() {
                ok = false;
                break;
            }
            let name = std::str::from_utf8(&buf[p..p + name_len])
                .unwrap_or("<bad utf8>")
                .to_string();
            p += name_len;
            frames.push(name);
        }
        if !ok {
            break;
        }
        // Strip leading hook frames so the leaf is user code.
        let leaf_start = frames
            .iter()
            .position(|n| !is_internal_frame(n))
            .unwrap_or(frames.len());
        let user_frames: Vec<String> = frames.into_iter().skip(leaf_start).collect();
        if user_frames.is_empty() {
            continue;
        }
        let entry = samples.0.entry(user_frames).or_default();
        entry.count += scale;
        entry.bytes += size as i64 * scale;
    }
    Ok(samples)
}

fn is_internal_frame(name: &str) -> bool {
    let n = name.trim_start_matches('_');
    matches!(n, "malloc" | "calloc" | "realloc")
        || n.starts_with("moonbit_malloc")
        || n.starts_with("moonbit_realloc")
        || n.starts_with("moonbit_make_")
        || n.starts_with("mi_")
        || n.starts_with("moon_pprof_")
        || n == "mpprof_init"
        || n == "mpprof_fini"
}

// ──────────────────────── pprof encoder ─────────────────────────────────

fn encode_pprof(samples: SampleMap, no_demangle: bool) -> Result<Vec<u8>> {
    let mut s = StringPool::new();
    let alloc_objs = s.intern("alloc_objects");
    let count_str = s.intern("count");
    let alloc_space = s.intern("alloc_space");
    let bytes_str = s.intern("bytes");
    let drop_pat = s.intern(
        "^_*(?:malloc|calloc|realloc|moonbit_malloc.*|moonbit_realloc.*|moonbit_make_.*|mi_.*|moon_pprof_.*|mpprof_.*)$",
    );
    let mapping_filename = s.intern("");

    let mut functions: Vec<proto::Function> = Vec::new();
    let mut locations: Vec<proto::Location> = Vec::new();
    let mut name_to_loc: HashMap<String, u64> = HashMap::new();
    let mut name_to_func: HashMap<String, u64> = HashMap::new();
    let mut out_samples: Vec<proto::Sample> = Vec::with_capacity(samples.0.len());

    for (frames, agg) in samples.0 {
        let mut location_ids: Vec<u64> = Vec::with_capacity(frames.len());
        for raw in &frames {
            let display = if no_demangle {
                raw.clone()
            } else {
                demangle_for_display(raw)
            };
            let loc_id = if let Some(&id) = name_to_loc.get(&display) {
                id
            } else {
                let func_id = if let Some(&fid) = name_to_func.get(&display) {
                    fid
                } else {
                    let fid = (functions.len() + 1) as u64;
                    let name_id = s.intern(&display);
                    let system_id = s.intern(raw);
                    functions.push(proto::Function {
                        id: fid,
                        name: name_id,
                        system_name: system_id,
                        filename: 0,
                        start_line: 0,
                    });
                    name_to_func.insert(display.clone(), fid);
                    fid
                };
                let id = (locations.len() + 1) as u64;
                locations.push(proto::Location {
                    id,
                    mapping_id: 1,
                    address: 0,
                    line: vec![proto::Line {
                        function_id: func_id,
                        line: 0,
                        column: 0,
                    }],
                    is_folded: false,
                });
                name_to_loc.insert(display.clone(), id);
                id
            };
            location_ids.push(loc_id);
        }
        if location_ids.is_empty() {
            continue;
        }
        out_samples.push(proto::Sample {
            location_id: location_ids,
            value: vec![agg.count, agg.bytes],
            label: vec![],
        });
    }

    let profile = proto::Profile {
        sample_type: vec![
            proto::ValueType {
                r#type: alloc_objs,
                unit: count_str,
            },
            proto::ValueType {
                r#type: alloc_space,
                unit: bytes_str,
            },
        ],
        sample: out_samples,
        mapping: vec![proto::Mapping {
            id: 1,
            memory_start: 0,
            memory_limit: 0,
            file_offset: 0,
            filename: mapping_filename,
            build_id: 0,
            has_functions: true,
            has_filenames: false,
            has_line_numbers: false,
            has_inline_frames: false,
        }],
        location: locations,
        function: functions,
        string_table: s.strings,
        drop_frames: drop_pat,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: 0,
        period_type: Some(proto::ValueType {
            r#type: alloc_space,
            unit: bytes_str,
        }),
        period: 1,
        comment: vec![],
        default_sample_type: alloc_space,
        doc_url: 0,
    };

    let mut buf = Vec::new();
    profile.encode(&mut buf)?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&buf)?;
    Ok(gz.finish()?)
}

fn demangle_for_display(raw: &str) -> String {
    // dladdr returns symbols with the platform's C linkage prefix
    // (`_` on macOS Mach-O, none on Linux ELF). MoonBit's mangle
    // starts with `_M…`, so on macOS the dladdr name looks like
    // `__M0FP…`. `moonbit_demangle::demangle` strips leading `_`s
    // itself, so the raw name works on either platform; we don't
    // need to pre-strip.
    let pretty = moonbit_demangle::demangle(raw);
    if pretty == raw {
        raw.to_string()
    } else {
        pretty
    }
}

struct StringPool {
    strings: Vec<String>,
    index: HashMap<String, i64>,
}

impl StringPool {
    fn new() -> Self {
        Self {
            strings: vec![String::new()],
            index: HashMap::from([(String::new(), 0)]),
        }
    }
    fn intern(&mut self, s: &str) -> i64 {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = self.strings.len() as i64;
        self.strings.push(s.to_string());
        self.index.insert(s.to_string(), id);
        id
    }
}

fn absolutize(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    let mut cur = env::current_dir().context("getting cwd to absolutize --out")?;
    cur.push(p);
    Ok(cur)
}

