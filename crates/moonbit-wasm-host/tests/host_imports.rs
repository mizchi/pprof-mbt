use anyhow::Result;
use moonbit_wasm_host::{MoonbitStdio, MoonbitStdioState};
use wasmtime::{Config, Engine, ExternRef, Linker, Module, Rooted, Store};

struct State {
    stdio: MoonbitStdioState,
}

impl MoonbitStdio for State {
    fn moonbit_stdio(&mut self) -> &mut MoonbitStdioState {
        &mut self.stdio
    }
}

fn engine() -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_reference_types(true);
    config.wasm_function_references(true);
    config.wasm_gc(true);
    config.wasm_exceptions(true);
    Ok(Engine::new(&config)?)
}

fn instantiate(engine: &Engine, wat: &str) -> Result<(Store<State>, wasmtime::Instance)> {
    instantiate_with_stdio(engine, wat, MoonbitStdioState::default())
}

fn instantiate_with_stdio(
    engine: &Engine,
    wat: &str,
    stdio: MoonbitStdioState,
) -> Result<(Store<State>, wasmtime::Instance)> {
    let wasm = wat::parse_str(wat)?;
    let module = Module::new(engine, wasm)?;
    let mut store = Store::new(engine, State { stdio });
    let mut linker = Linker::new(engine);
    moonbit_wasm_host::register(&mut linker)?;
    moonbit_wasm_host::register_store_imports(&mut linker, &mut store)?;
    let instance = linker.instantiate(&mut store, &module)?;
    Ok((store, instance))
}

#[test]
fn registers_moonbit_exception_imports() -> Result<()> {
    let engine = engine()?;
    let (mut store, instance) = instantiate(
        &engine,
        r#"
        (module
          (type $exception (func))
          (import "exception" "tag" (tag $tag (type $exception)))
          (import "exception" "throw" (func $throw))
          (func (export "run")
            call $throw))
        "#,
    )?;

    let run = instance.get_typed_func::<(), ()>(&mut store, "run")?;
    let err = run.call(&mut store, ()).unwrap_err();
    let text = format!("{err:?}");
    assert!(text.contains("MoonBit exception::throw"), "{text}");
    Ok(())
}

#[test]
fn registers_moonbit_time_imports() -> Result<()> {
    let engine = engine()?;
    let (mut store, instance) = instantiate(
        &engine,
        r#"
        (module
          (import "__moonbit_time_unstable" "instant_now"
            (func $instant_now (result externref)))
          (import "__moonbit_time_unstable" "instant_elapsed_as_secs_f64"
            (func $elapsed (param externref) (result f64)))
          (func (export "elapsed") (result f64)
            call $instant_now
            call $elapsed))
        "#,
    )?;

    let elapsed = instance
        .get_typed_func::<(), f64>(&mut store, "elapsed")?
        .call(&mut store, ())?;
    assert!(elapsed >= 0.0);
    Ok(())
}

#[test]
fn registers_moonbit_string_reader_imports() -> Result<()> {
    let engine = engine()?;
    let (mut store, instance) = instantiate(
        &engine,
        r#"
        (module
          (import "__moonbit_fs_unstable" "begin_read_string"
            (func $begin_read_string (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "string_read_char"
            (func $string_read_char (param externref) (result i32)))
          (import "__moonbit_fs_unstable" "finish_read_string"
            (func $finish_read_string (param externref)))
          (func (export "first_char") (param externref) (result i32)
            (local $reader externref)
            local.get 0
            call $begin_read_string
            local.tee $reader
            call $string_read_char
            local.get $reader
            call $finish_read_string))
        "#,
    )?;
    let input: Rooted<ExternRef> = ExternRef::new(&mut store, String::from("abc"))?;
    let ch = instance
        .get_typed_func::<Option<Rooted<ExternRef>>, i32>(&mut store, "first_char")?
        .call(&mut store, Some(input))?;
    assert_eq!(ch, b'a' as i32);
    Ok(())
}

#[test]
fn registers_moonbit_string_create_imports() -> Result<()> {
    let engine = engine()?;
    let (mut store, instance) = instantiate(
        &engine,
        r#"
        (module
          (import "__moonbit_fs_unstable" "begin_create_string"
            (func $begin_create_string (result externref)))
          (import "__moonbit_fs_unstable" "string_append_char"
            (func $string_append_char (param externref i32)))
          (import "__moonbit_fs_unstable" "finish_create_string"
            (func $finish_create_string (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "begin_read_string"
            (func $begin_read_string (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "string_read_char"
            (func $string_read_char (param externref) (result i32)))
          (func (export "roundtrip_first_char") (result i32)
            (local $writer externref)
            call $begin_create_string
            local.tee $writer
            i32.const 122
            call $string_append_char
            local.get $writer
            call $finish_create_string
            call $begin_read_string
            call $string_read_char))
        "#,
    )?;
    let ch = instance
        .get_typed_func::<(), i32>(&mut store, "roundtrip_first_char")?
        .call(&mut store, ())?;
    assert_eq!(ch, b'z' as i32);
    Ok(())
}

#[test]
fn registers_moonbit_args_and_env_imports() -> Result<()> {
    let engine = engine()?;
    let stdio = MoonbitStdioState {
        args: vec!["alpha".to_string()],
        ..MoonbitStdioState::default()
    };
    let (mut store, instance) = instantiate_with_stdio(
        &engine,
        r#"
        (module
          (import "__moonbit_fs_unstable" "args_get"
            (func $args_get (result externref)))
          (import "__moonbit_fs_unstable" "begin_read_string_array"
            (func $begin_read_string_array (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "string_array_read_string"
            (func $string_array_read_string (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "finish_read_string_array"
            (func $finish_read_string_array (param externref)))
          (import "__moonbit_fs_unstable" "begin_read_string"
            (func $begin_read_string (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "string_read_char"
            (func $string_read_char (param externref) (result i32)))
          (import "__moonbit_fs_unstable" "get_env_var_exists"
            (func $get_env_var_exists (param externref) (result i32)))
          (import "__moonbit_fs_unstable" "get_env_var"
            (func $get_env_var (param externref) (result externref)))
          (import "__moonbit_fs_unstable" "get_env_vars"
            (func $get_env_vars (result externref)))
          (import "__moonbit_fs_unstable" "set_env_var"
            (func $set_env_var (param externref externref)))
          (import "__moonbit_fs_unstable" "unset_env_var"
            (func $unset_env_var (param externref)))
          (import "__moonbit_fs_unstable" "current_dir"
            (func $current_dir (result externref)))
          (func (export "first_arg_first_char") (result i32)
            call $args_get
            call $begin_read_string_array
            call $string_array_read_string
            call $begin_read_string
            call $string_read_char)
          (func (export "set_exists") (param externref externref) (result i32)
            local.get 0
            local.get 1
            call $set_env_var
            local.get 0
            call $get_env_var_exists)
          (func (export "get_value_first_char") (param externref) (result i32)
            local.get 0
            call $get_env_var
            call $begin_read_string
            call $string_read_char)
          (func (export "unset_exists") (param externref) (result i32)
            local.get 0
            call $unset_env_var
            local.get 0
            call $get_env_var_exists)
          (func (export "smoke")
            call $get_env_vars
            drop
            call $current_dir
            drop))
        "#,
        stdio,
    )?;

    let first_arg = instance
        .get_typed_func::<(), i32>(&mut store, "first_arg_first_char")?
        .call(&mut store, ())?;
    assert_eq!(first_arg, b'a' as i32);

    let key: Rooted<ExternRef> = ExternRef::new(&mut store, String::from("__MOON_PPROF_TEST"))?;
    let value: Rooted<ExternRef> = ExternRef::new(&mut store, String::from("value"))?;
    let set_exists = instance
        .get_typed_func::<(Option<Rooted<ExternRef>>, Option<Rooted<ExternRef>>), i32>(
            &mut store,
            "set_exists",
        )?
        .call(&mut store, (Some(key), Some(value)))?;
    assert_eq!(set_exists, 1);

    let key: Rooted<ExternRef> = ExternRef::new(&mut store, String::from("__MOON_PPROF_TEST"))?;
    let first_value_char = instance
        .get_typed_func::<Option<Rooted<ExternRef>>, i32>(&mut store, "get_value_first_char")?
        .call(&mut store, Some(key))?;
    assert_eq!(first_value_char, b'v' as i32);

    let key: Rooted<ExternRef> = ExternRef::new(&mut store, String::from("__MOON_PPROF_TEST"))?;
    let unset_exists = instance
        .get_typed_func::<Option<Rooted<ExternRef>>, i32>(&mut store, "unset_exists")?
        .call(&mut store, Some(key))?;
    assert_eq!(unset_exists, 0);

    instance
        .get_typed_func::<(), ()>(&mut store, "smoke")?
        .call(&mut store, ())?;
    Ok(())
}

#[test]
fn registers_moonbit_now_and_exit_imports() -> Result<()> {
    let engine = engine()?;
    let (mut store, instance) = instantiate(
        &engine,
        r#"
        (module
          (import "__moonbit_time_unstable" "now"
            (func $now (result i64)))
          (import "__moonbit_sys_unstable" "exit"
            (func $exit (param i32)))
          (func (export "now") (result i64)
            call $now)
          (func (export "exit")
            i32.const 7
            call $exit))
        "#,
    )?;

    let now = instance
        .get_typed_func::<(), i64>(&mut store, "now")?
        .call(&mut store, ())?;
    assert!(now > 0);

    let err = instance
        .get_typed_func::<(), ()>(&mut store, "exit")?
        .call(&mut store, ())
        .unwrap_err();
    let text = format!("{err:?}");
    assert!(text.contains("MoonBit sys.exit(7)"), "{text}");
    Ok(())
}
