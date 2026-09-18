use super::*;
use std::sync::Once;

static INIT: Once = Once::new();
fn init_v8() {
    INIT.call_once(|| {
        deno_core::JsRuntime::init_platform(None);
    });
}

#[test]
fn get_extensions_returns_expected_count() {
    let exts = get_extensions();
    // 15 extensions by default (no edge_assert in production profile)
    assert_eq!(exts.len(), 15, "expected 15 extensions, got {}", exts.len());
}

#[test]
fn get_extensions_with_assert_returns_expected_count() {
    let base_exts = get_extensions();
    let exts = get_extensions_with_edge_assert(true);
    // Profile with edge_assert should always add exactly one extension.
    assert_eq!(
        exts.len(),
        base_exts.len() + 1,
        "expected assert profile to add one extension (base={}, assert={})",
        base_exts.len(),
        exts.len()
    );
}

#[test]
fn set_extension_transpiler_configures_opts() {
    let mut opts = RuntimeOptions::default();
    assert!(opts.extension_transpiler.is_none());
    set_extension_transpiler(&mut opts);
    assert!(opts.extension_transpiler.is_some());
}

#[test]
fn runtime_boots_with_extensions() {
    init_v8();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Tokio runtime");
    let _guard = runtime.enter();

    let mut options = RuntimeOptions {
        extensions: get_extensions(),
        ..Default::default()
    };
    set_extension_transpiler(&mut options);
    let _runtime = deno_core::JsRuntime::new(options);
}
