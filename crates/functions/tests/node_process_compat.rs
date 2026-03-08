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
        "typeof process.version === 'string' && typeof process.versions?.node === 'string' && typeof process.platform === 'string' && process.cwd() === '/bundle'",
        "process versions/platform should be available",
    );
}

#[test]
fn process_stdio_streams_are_exposed() {
    assert_js_true(
        "typeof process.stdout?.write === 'function' && typeof process.stderr?.write === 'function' && typeof process.stdin?.on === 'function' && process.stdout.isTTY === false && process.stderr.isTTY === false",
        "process stdio streams should be exposed as non-TTY streams",
    );
}

#[test]
fn process_env_is_local_and_mutable_without_host_access() {
    assert_js_true(
        "(() => {\n            const hostLikeBefore = process.env.PATH;\n            process.env.TEST_EDGE_VAR = 'ok';\n            const hasValue = process.env.TEST_EDGE_VAR === 'ok';\n            delete process.env.TEST_EDGE_VAR;\n            return hostLikeBefore === undefined && hasValue && process.env.TEST_EDGE_VAR === undefined;\n        })()",
        "process.env should work as local in-memory map",
    );
}

#[test]
fn process_chdir_is_blocked_to_prevent_host_filesystem_access() {
    assert_js_true(
        "(() => {\n            try {\n                process.chdir('/tmp');\n                return false;\n            } catch (e) {\n                return e?.code === 'ERR_HOST_ACCESS_DENIED' && String(e?.message).includes('process.chdir');\n            }\n        })()",
        "process.chdir should be blocked in sandbox",
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
        "(() => {\n            try {\n                process.exit(1);\n                return false;\n            } catch (e) {\n                return String(e && e.message).includes('[thunder] process.exit is not implemented in this runtime profile');\n            }\n        })()",
        "process.exit should throw deterministic not-implemented error",
    );
}

#[test]
fn buffer_global_supports_common_operations() {
    assert_js_true(
        "(() => {\n            const a = Buffer.from('hello', 'utf8');\n            const b = Buffer.from('68656c6c6f', 'hex');\n            const c = Buffer.concat([a, Buffer.from(' world')]);\n            const d = Buffer.alloc(3, 'A');\n            return Buffer.isBuffer(a) &&\n              a.toString('utf8') === 'hello' &&\n              b.toString('utf8') === 'hello' &&\n              c.toString('utf8') === 'hello world' &&\n              d.toString('utf8') === 'AAA' &&\n              Buffer.byteLength('hello', 'utf8') === 5;\n        })()",
        "Buffer global should support basic Node-compatible operations",
    );
}

#[test]
fn set_immediate_and_clear_immediate_are_available() {
    assert_js_true(
        "(() => {\n            if (typeof setImmediate !== 'function' || typeof clearImmediate !== 'function') return false;\n            const handle = setImmediate(() => {});\n            clearImmediate(handle);\n            return typeof handle === 'number';\n        })()",
        "setImmediate/clearImmediate should be available",
    );
}
