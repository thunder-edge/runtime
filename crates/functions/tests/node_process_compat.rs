use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::create_permissions_container;

static INIT: std::sync::Once = std::sync::Once::new();

fn init_v8() {
    INIT.call_once(|| {
        deno_core::JsRuntime::init_platform(None);
    });
}

fn make_runtime() -> JsRuntime {
    init_v8();
    let mut opts = RuntimeOptions {
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);
    let runtime = JsRuntime::new(opts);

    {
        let op_state = runtime.op_state();
        op_state.borrow_mut().put(create_permissions_container());
    }

    runtime
}

fn assert_js_true(js: &str, desc: &str) {
    let mut runtime = make_runtime();
    let result = runtime.execute_script("<test>", js.to_string());
    match result {
        Err(e) => panic!("[{desc}] JS execution error: {e}"),
        Ok(val) => {
            deno_core::scope!(scope, runtime);
            let local = val.open(scope);
            assert!(local.is_true(), "[{desc}] expected true, got false");
        }
    }
}

#[test]
fn process_global_is_available() {
    assert_js_true(
        "typeof globalThis.process === 'object' && process[Symbol.toStringTag] === 'process'",
        "process global should exist",
    );
}

#[test]
fn process_versions_and_platform_are_stable() {
    assert_js_true(
        "typeof process.version === 'string' && typeof process.versions?.node === 'string' && typeof process.platform === 'string'",
        "process versions/platform should be available",
    );
}

#[test]
fn process_env_is_local_and_mutable_without_host_access() {
    assert_js_true(
        "(() => {\n            process.env.TEST_EDGE_VAR = 'ok';\n            const hasValue = process.env.TEST_EDGE_VAR === 'ok';\n            delete process.env.TEST_EDGE_VAR;\n            return hasValue && process.env.TEST_EDGE_VAR === undefined;\n        })()",
        "process.env should work as local in-memory map",
    );
}

#[test]
fn process_next_tick_works() {
    assert_js_true(
        "typeof process.nextTick === 'function' && typeof process.hrtime?.bigint === 'function'",
        "process.nextTick and process.hrtime.bigint should exist",
    );
}

#[test]
fn process_sensitive_apis_throw_deterministic_error() {
    assert_js_true(
        "(() => {\n            try {\n                process.exit(1);\n                return false;\n            } catch (e) {\n                return String(e && e.message).includes('[edge-runtime] process.exit is not implemented in this runtime profile');\n            }\n        })()",
        "process.exit should throw deterministic not-implemented error",
    );
}
