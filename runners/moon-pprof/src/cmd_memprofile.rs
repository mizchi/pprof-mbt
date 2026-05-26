//! `moon-pprof memprofile <wasm>` — capture an allocation profile by
//! instrumenting the wasm and running it under wasmtime.
//!
//! How it works:
//!   1. walrus rewrites `$moonbit.malloc` to call an imported
//!      `moonbit_profile.alloc_hook(size: i32)` at the top of its body.
//!      `$moonbit.gc.malloc` internally calls `$moonbit.malloc(n+8)`,
//!      so this one hook covers both raw and refcount-managed allocs.
//!   2. wasmtime runs the rewritten module. The hook captures the
//!      current wasm call stack with `WasmBacktrace::force_capture`
//!      and accumulates (frames → (count, bytes)).
//!   3. After execution we emit a gzip'd pprof Profile with
//!      sample_type = [alloc_objects/count, alloc_space/bytes] and
//!      drop the `moonbit.malloc` / `moonbit.gc.malloc` frames so the
//!      visible leaf is the user code that requested the allocation.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow};
use clap::Parser;
use flate2::Compression;
use flate2::write::GzEncoder;
use moonbit_wasm_host::{MoonbitStdio, MoonbitStdioState};
use prost::Message;
use wasmtime::{
    AsContext, Caller, Config, Engine, FrameInfo, Linker, Module, Store, WasmBacktrace,
};

use firefox_to_pprof::proto;

const HOOK_MODULE: &str = "moonbit_profile";
const HOOK_FUNC: &str = "alloc_hook";

#[derive(Parser, Debug)]
#[command(about = "Capture an allocation profile of a MoonBit wasm by instrumenting moonbit.malloc.")]
pub struct Args {
    /// Path to the .wasm file (wasm or wasm-gc; see note on wasm-gc below).
    pub wasm: PathBuf,
    /// Output path for the gzip'd pprof.
    #[arg(long, default_value = "wasm-mem.pb.gz")]
    pub out: PathBuf,
    /// How many times to invoke `_start`.
    #[arg(long, default_value_t = 1)]
    pub iterations: usize,
    /// Pass raw mangled MoonBit names through instead of demangling.
    #[arg(long)]
    pub no_demangle: bool,
    /// Capture a stack trace only every Nth allocation. The host hook
    /// is what makes this profiler slow (`WasmBacktrace::force_capture`
    /// runs on every alloc by default); on large workloads — JSON
    /// parsing, bigint multiplication — that adds up to minutes per
    /// run. Sampling trades attribution resolution for speed: at
    /// `--sample-rate 100` the run is roughly 100x faster, and each
    /// captured stack is credited with 100 allocations × its measured
    /// size. Default is 1 (capture every allocation, fully exact).
    ///
    /// Caveat: deterministic 1/N sampling is biased when allocation
    /// sizes correlate with position in the alloc stream — e.g. a tight
    /// loop that allocates A,B,A,B,… and N=2 will only see one of the
    /// two sizes. For unbiased estimates use `--sample-rate 1` on a
    /// shorter workload, or rerun with a different starting offset.
    #[arg(long, default_value_t = 1)]
    pub sample_rate: u32,
}

pub fn run(args: Args) -> Result<()> {
    let wasm_bytes = fs::read(&args.wasm)
        .with_context(|| format!("reading wasm at {}", args.wasm.display()))?;

    // Step 1: rewrite the wasm.
    let (rewritten, report) = instrument(&wasm_bytes)
        .context("instrumenting wasm with moonbit_profile.alloc_hook")?;
    eprintln!(
        "[moon-pprof memprofile] instrumented: moonbit.malloc wrap={} gc-alloc-sites={}",
        report.moonbit_malloc_wrapped, report.gc_alloc_sites,
    );

    // Step 2: run it under wasmtime, with the hook wired to a shared
    // accumulator.
    let sample_rate = args.sample_rate.max(1);
    let samples = Arc::new(Mutex::new(SampleMap::default()));
    let mut config = Config::new();
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);
    // WasmBacktrace::force_capture needs address maps to map PCs back
    // to (func_index, offset). Without this the backtrace is empty.
    config.generate_address_map(true);
    config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
    // Match cmd_profile so deeply-recursive workloads don't overflow.
    config.async_stack_size(16 * 1024 * 1024);
    config.max_wasm_stack(8 * 1024 * 1024);
    // wasm-gc binaries land here too, even though MoonBit's wasm-gc
    // backend uses `struct.new` (which we don't instrument). Leaving
    // the proposals enabled keeps the runner usable for the wasm
    // backend's gc.malloc path (refcount header allocations) which
    // *does* go through moonbit.malloc.
    config.wasm_reference_types(true);
    config.wasm_function_references(true);
    config.wasm_gc(true);

    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, &rewritten)?;

    let mut store = Store::new(
        &engine,
        HostState {
            stdio: MoonbitStdioState::default(),
            samples: samples.clone(),
            sample_rate,
            counter: 0,
        },
    );

    let mut linker: Linker<HostState> = Linker::new(&engine);
    moonbit_wasm_host::register(&mut linker)?;
    linker.func_wrap(
        HOOK_MODULE,
        HOOK_FUNC,
        |mut caller: Caller<'_, HostState>, size: i32| {
            // Skip non-positive sizes — they shouldn't happen, but be
            // defensive: the instrumentation passes through whatever
            // `$moonbit.malloc` was called with.
            if size <= 0 {
                return;
            }
            // Bump the per-store counter and decide whether to take a
            // stack this time. Scale = 1 means "always sample".
            let scale = {
                let st = caller.data_mut();
                let n = st.counter;
                st.counter = n.wrapping_add(1);
                if st.sample_rate <= 1 {
                    1
                } else if n % st.sample_rate as u64 == 0 {
                    st.sample_rate as i64
                } else {
                    return;
                }
            };
            let bt = WasmBacktrace::force_capture(&caller.as_context());
            let frames: Vec<FrameKey> = bt
                .frames()
                .iter()
                .map(FrameKey::from_frame_info)
                .collect();
            if let Ok(mut map) = caller.data().samples.lock() {
                let entry = map.0.entry(frames).or_default();
                entry.count += scale;
                entry.bytes += size as i64 * scale;
            }
        },
    )?;

    let instance = linker.instantiate(&mut store, &module)?;
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
    let t0 = Instant::now();
    for _ in 0..args.iterations {
        start.call(&mut store, ())?;
    }
    let elapsed = t0.elapsed();

    // Drop the Store (and the closure-held Arc clone with it) so we
    // can move the accumulated samples out without an Arc dance.
    drop(store);
    let samples = std::mem::take(
        &mut *samples
            .lock()
            .map_err(|_| anyhow!("alloc hook accumulator poisoned"))?,
    );

    // Step 3: turn the accumulator into a pprof Profile.
    let pprof_bytes = encode_pprof(samples, args.no_demangle)?;
    fs::write(&args.out, &pprof_bytes)
        .with_context(|| format!("writing {}", args.out.display()))?;

    eprintln!(
        "[moon-pprof memprofile] {} iter in {:.2?} (sample-rate={}) → {}",
        args.iterations,
        elapsed,
        sample_rate,
        args.out.display(),
    );
    Ok(())
}

struct HostState {
    stdio: MoonbitStdioState,
    samples: Arc<Mutex<SampleMap>>,
    sample_rate: u32,
    counter: u64,
}

impl MoonbitStdio for HostState {
    fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState {
        &mut self.stdio
    }
}

#[derive(Default)]
struct SampleMap(HashMap<Vec<FrameKey>, SampleAgg>);

#[derive(Default, Clone, Copy)]
struct SampleAgg {
    count: i64,
    bytes: i64,
}

/// What we keep per frame across the lifetime of the run. Using
/// `(func_index, optional name)` keeps the key small while preserving
/// enough info to render the pprof later.
#[derive(Clone, Eq, Hash, PartialEq)]
struct FrameKey {
    func_index: u32,
    name: Option<String>,
}

impl FrameKey {
    fn from_frame_info(f: &FrameInfo) -> Self {
        Self {
            func_index: f.func_index(),
            name: f.func_name().map(|s| s.to_string()),
        }
    }
}

#[derive(Default, Debug, Clone, Copy)]
struct InstrumentReport {
    /// Whether `$moonbit.malloc` was wrapped (wasm non-gc path).
    moonbit_malloc_wrapped: bool,
    /// Number of wasm-gc allocation opcodes that were rewritten.
    gc_alloc_sites: usize,
}

fn instrument(wasm_bytes: &[u8]) -> Result<(Vec<u8>, InstrumentReport)> {
    let mut module = walrus::Module::from_buffer(wasm_bytes)
        .context("parsing wasm with walrus")?;

    // The hook signature is shared by both paths.
    let hook_type = module.types.add(&[walrus::ValType::I32], &[]);
    let (hook_fn, _) =
        module.add_import_func(HOOK_MODULE, HOOK_FUNC, hook_type);

    let mut report = InstrumentReport::default();

    // wasm (non-gc) path: wrap $moonbit.malloc if it exists. The same
    // function is called from $moonbit.gc.malloc(n) with arg n+8, so
    // both raw and refcount allocations land in our hook.
    if let Some(fid) = find_func_by_name(&module, "moonbit.malloc") {
        wrap_moonbit_malloc(&mut module, fid, hook_fn);
        report.moonbit_malloc_wrapped = true;
    }

    // wasm-gc path: pre-compute size for every struct/array type, then
    // walk every function body and rewrite each alloc opcode.
    let type_sizes = TypeSizes::build(&module);
    report.gc_alloc_sites = instrument_gc_allocs(&mut module, hook_fn, &type_sizes);

    if !report.moonbit_malloc_wrapped && report.gc_alloc_sites == 0 {
        return Err(anyhow!(
            "no instrumentation sites found in this wasm — not a MoonBit binary?"
        ));
    }

    Ok((module.emit_wasm(), report))
}

fn find_func_by_name(module: &walrus::Module, name: &str) -> Option<walrus::FunctionId> {
    module
        .funcs
        .iter()
        .find(|f| f.name.as_deref() == Some(name))
        .map(|f| f.id())
}

fn wrap_moonbit_malloc(
    module: &mut walrus::Module,
    target: walrus::FunctionId,
    hook_fn: walrus::FunctionId,
) {
    let local = match &mut module.funcs.get_mut(target).kind {
        walrus::FunctionKind::Local(l) => l,
        _ => unreachable!("moonbit.malloc must be a local function"),
    };
    let size_arg = local.args[0];
    let entry = local.entry_block();
    let block = local.block_mut(entry);
    let original: Vec<_> = block.instrs.drain(..).collect();
    block.instrs.push((
        walrus::ir::Instr::LocalGet(walrus::ir::LocalGet { local: size_arg }),
        Default::default(),
    ));
    block.instrs.push((
        walrus::ir::Instr::Call(walrus::ir::Call { func: hook_fn }),
        Default::default(),
    ));
    block.instrs.extend(original);
}

/// Per-type allocation size lookup. wasm-gc heap layout is engine-
/// defined; we use the wasm "logical" size (sum of field sizes for
/// structs, element size for arrays) as a proxy. Wasmtime's actual GC
/// heap consumption includes alignment + headers we can't see, so the
/// numbers are an attribution signal rather than an exact byte count.
struct TypeSizes {
    /// For each type id: `Some(struct_size)` if Struct, else None.
    struct_size: std::collections::HashMap<walrus::TypeId, i32>,
    /// For each type id: `Some(elem_size)` if Array, else None.
    array_elem_size: std::collections::HashMap<walrus::TypeId, i32>,
}

impl TypeSizes {
    fn build(module: &walrus::Module) -> Self {
        let mut struct_size = std::collections::HashMap::new();
        let mut array_elem_size = std::collections::HashMap::new();
        for ty in module.types.iter() {
            if let Some(s) = ty.as_struct() {
                let total: i32 = s.fields.iter().map(field_size).sum();
                struct_size.insert(ty.id(), total);
            } else if let Some(a) = ty.as_array() {
                array_elem_size.insert(ty.id(), field_size(&a.field));
            }
        }
        Self {
            struct_size,
            array_elem_size,
        }
    }

    fn get_struct(&self, ty: walrus::TypeId) -> Option<i32> {
        self.struct_size.get(&ty).copied()
    }

    fn get_array_elem(&self, ty: walrus::TypeId) -> Option<i32> {
        self.array_elem_size.get(&ty).copied()
    }
}

fn field_size(f: &walrus::FieldType) -> i32 {
    use walrus::StorageType;
    use walrus::ValType;
    match &f.element_type {
        StorageType::I8 => 1,
        StorageType::I16 => 2,
        StorageType::Val(v) => match v {
            ValType::I32 | ValType::F32 => 4,
            ValType::I64 | ValType::F64 => 8,
            ValType::V128 => 16,
            // wasmtime currently uses 4-byte slots for GC refs; even if
            // the engine grows to 8-byte refs later this still gives a
            // useful attribution proxy.
            ValType::Ref(_) => 4,
        },
    }
}

fn instrument_gc_allocs(
    module: &mut walrus::Module,
    hook_fn: walrus::FunctionId,
    sizes: &TypeSizes,
) -> usize {
    // Collect function ids up front so we can mutate the module inside
    // the loop without aliasing the iterator.
    let fids: Vec<walrus::FunctionId> = module
        .funcs
        .iter()
        .filter_map(|f| match &f.kind {
            walrus::FunctionKind::Local(_) => Some(f.id()),
            _ => None,
        })
        .collect();

    let mut total = 0;
    for fid in fids {
        total += instrument_gc_allocs_in_func(module, fid, hook_fn, sizes);
    }
    total
}

fn instrument_gc_allocs_in_func(
    module: &mut walrus::Module,
    fid: walrus::FunctionId,
    hook_fn: walrus::FunctionId,
    sizes: &TypeSizes,
) -> usize {
    use walrus::ir::*;

    // First pass: gather every InstrSeqId reachable from the function
    // body. We have to do this with an immutable borrow first because
    // walrus's `block_mut` takes a unique borrow of the function.
    let seq_ids = {
        let local = match &module.funcs.get(fid).kind {
            walrus::FunctionKind::Local(l) => l,
            _ => return 0,
        };
        collect_seq_ids(local)
    };

    // We need a scratch i32 local for the dynamic-size cases. Add it
    // lazily; this is cheap and only fires once per function.
    let local = match &mut module.funcs.get_mut(fid).kind {
        walrus::FunctionKind::Local(l) => l,
        _ => return 0,
    };
    // walrus stores locals on the module; we have to add via module.locals.
    // (Drop the local borrow so we can borrow module.locals next.)
    let _ = local;
    let scratch = module.locals.add(walrus::ValType::I32);

    let mut count = 0;
    for seq_id in seq_ids {
        let local = match &mut module.funcs.get_mut(fid).kind {
            walrus::FunctionKind::Local(l) => l,
            _ => unreachable!(),
        };
        let block = local.block_mut(seq_id);
        let old: Vec<_> = block.instrs.drain(..).collect();
        let mut new = Vec::with_capacity(old.len() + 4);

        for (instr, loc) in old {
            // ---- Static-size opcodes ----
            // struct.new $T / struct.new_default $T:
            //   stack: [..., fields...]  pops fields, pushes ref
            // array.new_fixed $T N:
            //   stack: [..., elem0..elemN-1]  pops N, pushes ref
            //
            // For all three, we can push `i32.const size; call hook` AFTER
            // the operands but BEFORE the alloc. Hook consumes one i32
            // and returns nothing, leaving the operands intact below it.
            let static_size = match &instr {
                Instr::StructNew(s) => sizes.get_struct(s.ty),
                Instr::StructNewDefault(s) => sizes.get_struct(s.ty),
                Instr::ArrayNewFixed(a) => {
                    sizes.get_array_elem(a.ty).map(|e| e * a.len as i32)
                }
                _ => None,
            };
            if let Some(size) = static_size {
                if size > 0 {
                    new.push((
                        Instr::Const(Const {
                            value: Value::I32(size),
                        }),
                        Default::default(),
                    ));
                    new.push((
                        Instr::Call(Call { func: hook_fn }),
                        Default::default(),
                    ));
                }
                new.push((instr, loc));
                count += 1;
                continue;
            }

            // ---- Dynamic-size opcodes ----
            // array.new $T:           stack [..., elem, len]    → ref
            // array.new_default $T:   stack [..., len]          → ref
            // array.new_data $T $D:   stack [..., offset, len]  → ref
            // array.new_elem $T $E:   stack [..., offset, len]  → ref
            //
            // In all four, the *top* of stack is the element-count and
            // the rest of the operands sit below it. We:
            //   1. local.set $scratch         ;; pop len → scratch
            //   2. i32.const <elem_size>
            //   3. local.get $scratch
            //   4. i32.mul                    ;; push len*elem_size
            //   5. call $hook                 ;; consume that
            //   6. local.get $scratch         ;; push len back
            //   7. <original instr>
            //
            // This leaves any operand that sat below `len` (elem for
            // array.new, offset for array.new_data/elem, nothing for
            // array.new_default) untouched.
            let dyn_elem_size = match &instr {
                Instr::ArrayNew(a) => sizes.get_array_elem(a.ty),
                Instr::ArrayNewDefault(a) => sizes.get_array_elem(a.ty),
                Instr::ArrayNewData(a) => sizes.get_array_elem(a.ty),
                Instr::ArrayNewElem(a) => sizes.get_array_elem(a.ty),
                _ => None,
            };
            if let Some(elem_size) = dyn_elem_size {
                if elem_size > 0 {
                    new.push((
                        Instr::LocalSet(LocalSet { local: scratch }),
                        Default::default(),
                    ));
                    new.push((
                        Instr::Const(Const {
                            value: Value::I32(elem_size),
                        }),
                        Default::default(),
                    ));
                    new.push((
                        Instr::LocalGet(LocalGet { local: scratch }),
                        Default::default(),
                    ));
                    new.push((
                        Instr::Binop(Binop {
                            op: BinaryOp::I32Mul,
                        }),
                        Default::default(),
                    ));
                    new.push((
                        Instr::Call(Call { func: hook_fn }),
                        Default::default(),
                    ));
                    new.push((
                        Instr::LocalGet(LocalGet { local: scratch }),
                        Default::default(),
                    ));
                }
                new.push((instr, loc));
                count += 1;
                continue;
            }

            new.push((instr, loc));
        }

        let local = match &mut module.funcs.get_mut(fid).kind {
            walrus::FunctionKind::Local(l) => l,
            _ => unreachable!(),
        };
        local.block_mut(seq_id).instrs = new;
    }

    count
}

fn collect_seq_ids(local: &walrus::LocalFunction) -> Vec<walrus::ir::InstrSeqId> {
    use walrus::ir::*;
    let mut out = Vec::new();
    let mut stack = vec![local.entry_block()];
    while let Some(id) = stack.pop() {
        out.push(id);
        for (instr, _) in &local.block(id).instrs {
            match instr {
                Instr::Block(b) => stack.push(b.seq),
                Instr::Loop(l) => stack.push(l.seq),
                Instr::IfElse(ie) => {
                    stack.push(ie.consequent);
                    stack.push(ie.alternative);
                }
                _ => {}
            }
        }
    }
    out
}

fn encode_pprof(samples: SampleMap, no_demangle: bool) -> Result<Vec<u8>> {
    let mut state = State::new(no_demangle);
    // Pre-intern fixed strings.
    let alloc_objs = state.intern("alloc_objects");
    let count_str = state.intern("count");
    let alloc_space = state.intern("alloc_space");
    let bytes_str = state.intern("bytes");
    let drop_pat = state.intern("^moonbit\\.(gc\\.)?malloc$");
    let mapping_filename = state.intern("");

    let mut out_samples: Vec<proto::Sample> = Vec::with_capacity(samples.0.len());
    let mut total_objects: i64 = 0;
    let mut total_bytes: i64 = 0;
    for (frames, agg) in samples.0 {
        // WasmBacktrace frames are leaf-first already, which matches
        // pprof's location_id ordering.
        let location_ids: Vec<u64> = frames
            .iter()
            .map(|f| state.intern_location(f))
            .collect();
        if location_ids.is_empty() {
            continue;
        }
        out_samples.push(proto::Sample {
            location_id: location_ids,
            value: vec![agg.count, agg.bytes],
            label: vec![],
        });
        total_objects += agg.count;
        total_bytes += agg.bytes;
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
        location: state.locations,
        function: state.functions,
        string_table: state.strings,
        // Drop the malloc frames so the user sees their own code as the
        // leaf in the flame graph / top view.
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

    let _ = (total_objects, total_bytes); // currently only logged via subcommand printer

    let mut buf = Vec::new();
    profile.encode(&mut buf)?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&buf)?;
    Ok(gz.finish()?)
}

struct State {
    strings: Vec<String>,
    string_index: HashMap<String, i64>,
    functions: Vec<proto::Function>,
    func_index: HashMap<u32, u64>,
    locations: Vec<proto::Location>,
    loc_index: HashMap<u32, u64>,
    no_demangle: bool,
}

impl State {
    fn new(no_demangle: bool) -> Self {
        Self {
            strings: vec![String::new()],
            string_index: HashMap::from([(String::new(), 0)]),
            functions: Vec::new(),
            func_index: HashMap::new(),
            locations: Vec::new(),
            loc_index: HashMap::new(),
            no_demangle,
        }
    }

    fn intern(&mut self, s: &str) -> i64 {
        if let Some(&id) = self.string_index.get(s) {
            return id;
        }
        let id = self.strings.len() as i64;
        self.strings.push(s.to_string());
        self.string_index.insert(s.to_string(), id);
        id
    }

    fn intern_function(&mut self, frame: &FrameKey) -> u64 {
        if let Some(&id) = self.func_index.get(&frame.func_index) {
            return id;
        }
        let raw = frame
            .name
            .clone()
            .unwrap_or_else(|| format!("func[{}]", frame.func_index));
        let pretty = if self.no_demangle {
            raw.clone()
        } else {
            moonbit_demangle::demangle(&raw)
        };
        let id = (self.functions.len() + 1) as u64;
        let name = self.intern(&pretty);
        let system_name = self.intern(&raw);
        self.functions.push(proto::Function {
            id,
            name,
            system_name,
            filename: 0,
            start_line: 0,
        });
        self.func_index.insert(frame.func_index, id);
        id
    }

    fn intern_location(&mut self, frame: &FrameKey) -> u64 {
        if let Some(&id) = self.loc_index.get(&frame.func_index) {
            return id;
        }
        let func_id = self.intern_function(frame);
        let id = (self.locations.len() + 1) as u64;
        self.locations.push(proto::Location {
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
        self.loc_index.insert(frame.func_index, id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_body<'a>(printed: &'a str, func_marker: &str) -> &'a str {
        // Returns the substring between the line that opens the
        // function (contains `func_marker`) and the next line that
        // closes it. Good enough for tiny synth wasm.
        let start = printed
            .find(func_marker)
            .expect("function marker not found in printed wasm");
        let after = &printed[start..];
        let end = after.find("\n  )").unwrap_or(after.len());
        &after[..end]
    }

    #[test]
    fn instrument_inserts_call_at_moonbit_malloc_head() {
        // Synth tiny wasm with a function called "moonbit.malloc" that
        // just returns its arg.
        let wat = r#"
            (module
              (func (export "_start"))
              (func $moonbit.malloc (param $n i32) (result i32)
                local.get $n))
        "#;
        let wasm = wat::parse_str(wat).unwrap();
        let (out, report) = instrument(&wasm).unwrap();
        assert!(report.moonbit_malloc_wrapped);
        // The output must parse and must contain an import for
        // moonbit_profile.alloc_hook.
        let printed = wasmprinter::print_bytes(&out).unwrap();
        eprintln!("instrumented wasm:\n{}", printed);
        assert!(printed.contains(r#"(import "moonbit_profile" "alloc_hook""#));
        // The injected hook call must appear inside $moonbit.malloc.
        // walrus may print the call as `call N` or `call $name`, so we
        // just check both the import declaration and a corresponding
        // call inside the function.
        let body = extract_body(&printed, "$moonbit.malloc");
        assert!(
            body.contains("call ") && body.lines().any(|l| l.trim_start().starts_with("call ")),
            "no call instruction in instrumented body:\n{body}"
        );
    }
}
