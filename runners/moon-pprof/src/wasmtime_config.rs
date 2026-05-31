use wasmtime::Config;

pub fn enable_moonbit_wasm_features(config: &mut Config) {
    // MoonBit's wasm backends may use GC, function references, reference
    // types, and the exception-handling proposal. Keep the runner permissive
    // for both legacy wasm and wasm-gc artifacts; unused proposals are a no-op
    // for modules that do not exercise them.
    config.wasm_reference_types(true);
    config.wasm_function_references(true);
    config.wasm_gc(true);
    config.wasm_exceptions(true);
}
