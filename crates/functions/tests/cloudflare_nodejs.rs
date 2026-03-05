use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;

// This module tests Node.js APIs and their Web Standard alternatives in Cloudflare Workers context
// Reference: https://developers.cloudflare.com/workers/runtime-apis/

static INIT: std::sync::Once = std::sync::Once::new();

fn init_v8() {
    INIT.call_once(|| {
        deno_core::JsRuntime::init_platform(None, false);
    });
}

fn make_runtime() -> JsRuntime {
    init_v8();
    let mut opts = RuntimeOptions {
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);
    let mut runtime = JsRuntime::new(opts);

    {
        let mut op_state = runtime.op_state();
        op_state.borrow_mut().put(Permissions);
    }

    runtime
}

fn assert_js_true(js: &str, desc: &str) {
    // Ensure we run within a Tokio runtime context for deno_core operations
    if tokio::runtime::Handle::try_current().is_err() {
        // Use current_thread runtime to match deno_fetch expectations (EventSource)
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(async {
            assert_js_true_async(js, desc).await;
        });
    } else {
        // Already in async context, run directly
        let mut runtime = make_runtime();
        let result = runtime.execute_script("<test>", js.to_string());
        match result {
            Err(e) => panic!("[{desc}] JS execution error: {e}"),
            Ok(val) => {
                let scope = &mut runtime.handle_scope();
                let local = deno_core::v8::Local::new(scope, val);
                assert!(local.is_true(), "[{desc}] expected true, got false");
            }
        }
    }
}

async fn assert_js_true_async(js: &str, desc: &str) {
    let mut runtime = make_runtime();
    let result = runtime.execute_script("<test>", js.to_string());
    match result {
        Err(e) => panic!("[{desc}] JS execution error: {e}"),
        Ok(val) => {
            let scope = &mut runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, val);
            assert!(local.is_true(), "[{desc}] expected true, got false");
        }
    }
}

// ── require() and CommonJS ────────────────────────────────────────

// NOTE: require() and CommonJS are NOT available in current deno-edge-runtime
// Reason: deno_node extension is not loaded
//
// Solution: Use ES modules (import/export) instead
// The runtime uses standard JavaScript modules

#[test]
fn nodejs_require_not_available() {
    assert_js_true(
        "typeof require === 'undefined'",
        "require() correctly not available (ES modules only)",
    );
}

#[test]
fn nodejs_module_exports_not_available() {
    assert_js_true(
        "typeof module === 'undefined' || typeof module.exports === 'undefined'",
        "module.exports correctly not available",
    );
}

// ── Events: EventEmitter vs EventTarget ────────────────────────────

// NOTE: Node.js EventEmitter is NOT available
// Alternative: Use Web Standard EventTarget API

#[test]
fn nodejs_events_alternative_event_target() {
    assert_js_true(
        "(() => {
            // EventTarget is Web Standard (available)
            const emitter = new EventTarget();

            let eventFired = false;
            emitter.addEventListener('customEvent', () => {
                eventFired = true;
            });

            emitter.dispatchEvent(new Event('customEvent'));

            return eventFired === true;
        })()",
        "EventTarget as EventEmitter alternative",
    );
}

#[test]
fn nodejs_events_emitter_pattern() {
    assert_js_true(
        "(() => {
            // Implement EventEmitter-like pattern with EventTarget
            class SimpleEmitter extends EventTarget {
                emit(event, data) {
                    this.dispatchEvent(new CustomEvent(event, { detail: data }));
                }

                on(event, handler) {
                    this.addEventListener(event, (e) => handler(e.detail));
                }
            }

            const emitter = new SimpleEmitter();
            let received = null;

            emitter.on('message', (data) => {
                received = data;
            });

            emitter.emit('message', 'hello');

            return received === 'hello';
        })()",
        "EventEmitter pattern via EventTarget",
    );
}

// ── Stream: Web Streams API instead of Node.js Stream ──────────────

// NOTE: Node.js Stream module is NOT available
// Alternative: Use Web Streams API (ReadableStream, WritableStream, TransformStream)

#[test]
fn nodejs_stream_alternative_readable() {
    assert_js_true(
        "(() => {
            const hasReadableStream = typeof ReadableStream === 'function';
            const hasWritableStream = typeof WritableStream === 'function';
            const hasTransformStream = typeof TransformStream === 'function';

            return hasReadableStream && hasWritableStream && hasTransformStream;
        })()",
        "Web Streams API available (Node.js Stream alternative)",
    );
}

#[test]
fn nodejs_stream_example_transform() {
    assert_js_true(
        "(() => {
            // Transform stream pattern (like Node.js Transform)
            const transformStream = new TransformStream({
                transform(chunk, controller) {
                    // Transform the chunk
                    const transformed = new TextEncoder().encode(
                        new TextDecoder().decode(chunk).toUpperCase()
                    );
                    controller.enqueue(transformed);
                }
            });

            return typeof transformStream.readable === 'object' &&
                   typeof transformStream.writable === 'object';
        })()",
        "TransformStream for stream transformations",
    );
}

// ── Path: URL API instead of Node.js path module ────────────────

// NOTE: Node.js path module is NOT available
// Alternative: Use Web Standard URL API

#[test]
fn nodejs_path_alternative_url_api() {
    assert_js_true(
        "(() => {
            const url = new URL('file:///home/user/documents/file.txt');

            // Extract components like path module
            const pathname = url.pathname;  // /home/user/documents/file.txt
            const filename = pathname.split('/').pop();  // file.txt
            const dirname = pathname.substring(0, pathname.lastIndexOf('/'));  // /home/user/documents

            return filename === 'file.txt' && dirname === '/home/user/documents';
        })()",
        "URL API for path-like operations",
    );
}

#[test]
fn nodejs_path_functions_as_utilities() {
    assert_js_true(
        r#"(() => {
            // Helper functions to replace path module
            const path = {
                basename: (p) => p.split('/').pop(),
                dirname: (p) => p.substring(0, p.lastIndexOf('/')),
                extname: (p) => {
                    const lastDot = p.lastIndexOf('.');
                    return lastDot > 0 ? p.substring(lastDot) : '';
                },
                join: (...parts) => parts.join('/').replace(/\\/g, '/')
            };

            return path.basename('/a/b/c.txt') === 'c.txt' &&
                   path.extname('/a/b/c.txt') === '.txt';
        })()"#,
        "Path-like utility functions",
    );
}

// ── Crypto: Web Crypto vs Node.js crypto ──────────────────────────

// NOTE: Node.js crypto module is NOT available
// But Web Crypto API IS available, which covers most use cases

#[test]
fn nodejs_crypto_alternative_web_crypto() {
    assert_js_true(
        "(() => {
            // Web Crypto is available
            const hasCrypto = typeof crypto === 'object' &&
                            typeof crypto.subtle === 'object' &&
                            typeof crypto.getRandomValues === 'function';

            return hasCrypto;
        })()",
        "Web Crypto API available (Node.js crypto alternative)",
    );
}

#[test]
fn nodejs_crypto_random_values() {
    assert_js_true(
        "(() => {
            // crypto.getRandomValues() is available
            const buffer = new Uint8Array(16);
            crypto.getRandomValues(buffer);

            // Check that values are not all zeros
            return buffer.some(v => v !== 0);
        })()",
        "crypto.getRandomValues() works",
    );
}

#[test]
fn nodejs_crypto_random_uuid() {
    assert_js_true(
        "(() => {
            // crypto.randomUUID() is available
            const uuid = crypto.randomUUID();
            return typeof uuid === 'string' && uuid.includes('-');
        })()",
        "crypto.randomUUID() works",
    );
}

// ── Util: No direct replacement, use built-in functions ────────────

// NOTE: Node.js util module is NOT available
// But JavaScript built-ins provide most functionality

#[test]
fn nodejs_util_types_alternative() {
    assert_js_true(
        "(() => {
            // util.types alternatives using in-built JavaScript
            const types = {
                isArray: Array.isArray,
                isDate: (v) => v instanceof Date,
                isError: (v) => v instanceof Error,
                isFunction: (v) => typeof v === 'function',
                isObject: (v) => v !== null && typeof v === 'object',
                isString: (v) => typeof v === 'string',
                isNumber: (v) => typeof v === 'number'
            };

            return Array.isArray([1, 2, 3]) && types.isString('hello');
        })()",
        "Type checking alternatives",
    );
}

#[test]
fn nodejs_util_promisify_alternative() {
    assert_js_true(
        "(() => {
            // util.promisify alternative
            const promisify = (fn) => (...args) => {
                return new Promise((resolve, reject) => {
                    fn(...args, (err, result) => {
                        if(err) reject(err);
                        else resolve(result);
                    });
                });
            };

            return typeof promisify === 'function';
        })()",
        "util.promisify() pattern",
    );
}

// ── Assert: Assertion patterns without assert module ────────────────

// NOTE: Node.js assert module is NOT available
// But assertion patterns can be implemented directly

#[test]
fn nodejs_assert_alternative() {
    assert_js_true(
        "(() => {
            // Simple assert function
            const assert = (condition, message) => {
                if(!condition) throw new Error(message);
            };

            try {
                assert(1 + 1 === 2, 'Math works');
                return true;
            } catch(e) {
                return false;
            }
        })()",
        "Simple assertion pattern",
    );
}

// ── Summary: Node.js vs Web/Deno APIs ──────────────────────────────

// NOT Available in deno-edge-runtime:
// ✗ require() / module.exports (use import/export)
// ✗ Node.js EventEmitter (use EventTarget)
// ✗ Node.js Stream (use Web Streams API)
// ✗ Node.js path module (use URL API + string utilities)
// ✗ Node.js crypto module (use Web Crypto API)
// ✗ Node.js util module (use built-in functions)
// ✗ Node.js assert module (use custom assertions)
// ✗ Node.js fs module (use Deno APIs or external services)
// ✗ Node.js http/https module (use Fetch API)
//
// Available Equivalents:
// ✓ Web Crypto API (crypto.subtle, crypto.getRandomValues, crypto.randomUUID)
// ✓ Web Streams API (ReadableStream, WritableStream, TransformStream)
// ✓ EventTarget (event-driven programming)
// ✓ URL API (path-like operations)
// ✓ TextEncoder/TextDecoder (string encoding)
// ✓ Fetch API (HTTP requests)
// ✓ Timers (setTimeout, setInterval)
// ✓ JSON (serialization)
// ✓ ArrayBuffer, TypedArrays (binary data)
