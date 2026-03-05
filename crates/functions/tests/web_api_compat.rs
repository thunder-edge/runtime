//! Web API compatibility tests.
//!
//! Each test boots a JsRuntime with the production extensions and evaluates
//! a small JS snippet to verify that a Web API is available and functional.

use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;

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

    // Add Permissions to the op_state so that deno_web and other extensions can access it
    {
        let mut op_state = runtime.op_state();
        op_state.borrow_mut().put(Permissions);
    }

    runtime
}

/// Evaluate a JS expression that should return true.
fn assert_js_true(js: &str, desc: &str) {
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

// ── Fetch API ─────────────────────────────────────────────────────────

#[test]
fn fetch_headers_constructor() {
    assert_js_true(
        "typeof Headers === 'function' && new Headers({'x-test': 'ok'}).get('x-test') === 'ok'",
        "Headers constructor",
    );
}

#[test]
fn fetch_request_constructor() {
    assert_js_true(
        "typeof Request === 'function' && new Request('http://example.com').method === 'GET'",
        "Request constructor",
    );
}

#[test]
fn fetch_response_constructor() {
    assert_js_true(
        "typeof Response === 'function' && new Response('hello').status === 200",
        "Response constructor",
    );
}

#[test]
fn fetch_function_exists() {
    assert_js_true(
        "typeof fetch === 'function'",
        "fetch function",
    );
}

#[test]
fn fetch_formdata_constructor() {
    assert_js_true(
        "typeof FormData === 'function' && (() => { const fd = new FormData(); fd.append('k','v'); return fd.get('k') === 'v'; })()",
        "FormData constructor",
    );
}

#[test]
fn fetch_eventsource_constructor() {
    assert_js_true(
        "typeof EventSource === 'function'",
        "EventSource exists",
    );
}

// ── URL API ───────────────────────────────────────────────────────────

#[test]
fn url_constructor() {
    assert_js_true(
        "new URL('https://example.com/path?q=1').pathname === '/path'",
        "URL constructor",
    );
}

#[test]
fn url_search_params() {
    assert_js_true(
        "new URLSearchParams('a=1&b=2').get('b') === '2'",
        "URLSearchParams",
    );
}

#[test]
fn url_pattern() {
    assert_js_true(
        "typeof URLPattern === 'function'",
        "URLPattern exists",
    );
}

// ── Streams API ────────────────────────────────────────────────────────

#[test]
fn streams_readable() {
    assert_js_true(
        "typeof ReadableStream === 'function' && new ReadableStream() instanceof ReadableStream",
        "ReadableStream constructor",
    );
}

#[test]
fn streams_writable() {
    assert_js_true(
        "typeof WritableStream === 'function'",
        "WritableStream constructor",
    );
}

#[test]
fn streams_transform() {
    assert_js_true(
        "typeof TransformStream === 'function' && (() => { const ts = new TransformStream(); return ts.readable instanceof ReadableStream; })()",
        "TransformStream constructor",
    );
}

#[test]
fn streams_byte_length_queuing_strategy() {
    assert_js_true(
        "typeof ByteLengthQueuingStrategy === 'function'",
        "ByteLengthQueuingStrategy",
    );
}

#[test]
fn streams_count_queuing_strategy() {
    assert_js_true(
        "typeof CountQueuingStrategy === 'function'",
        "CountQueuingStrategy",
    );
}

// ── Encoding API ──────────────────────────────────────────────────────

#[test]
fn encoding_text_encoder() {
    assert_js_true(
        "new TextEncoder().encode('abc').length === 3",
        "TextEncoder",
    );
}

#[test]
fn encoding_text_decoder() {
    assert_js_true(
        "new TextDecoder().decode(new Uint8Array([72,105])) === 'Hi'",
        "TextDecoder",
    );
}

#[test]
fn encoding_atob_btoa() {
    assert_js_true(
        "atob(btoa('hello')) === 'hello'",
        "atob/btoa round trip",
    );
}

#[test]
fn encoding_text_encoder_stream() {
    assert_js_true(
        "typeof TextEncoderStream === 'function'",
        "TextEncoderStream",
    );
}

#[test]
fn encoding_text_decoder_stream() {
    assert_js_true(
        "typeof TextDecoderStream === 'function'",
        "TextDecoderStream",
    );
}

// ── Crypto API ────────────────────────────────────────────────────────

#[test]
fn crypto_get_random_values() {
    assert_js_true(
        "(() => { const arr = new Uint8Array(16); crypto.getRandomValues(arr); return arr.some(v => v !== 0); })()",
        "crypto.getRandomValues",
    );
}

#[test]
fn crypto_random_uuid() {
    assert_js_true(
        "typeof crypto.randomUUID() === 'string' && crypto.randomUUID().length === 36",
        "crypto.randomUUID",
    );
}

#[test]
fn crypto_subtle_exists() {
    assert_js_true(
        "typeof crypto.subtle === 'object' && typeof crypto.subtle.digest === 'function'",
        "crypto.subtle",
    );
}

#[test]
fn crypto_crypto_key_exists() {
    assert_js_true(
        "typeof CryptoKey === 'function'",
        "CryptoKey",
    );
}

// ── Console API ───────────────────────────────────────────────────────

#[test]
fn console_log_exists() {
    assert_js_true(
        "typeof console === 'object' && typeof console.log === 'function'",
        "console.log",
    );
}

#[test]
fn console_error_warn() {
    assert_js_true(
        "typeof console.error === 'function' && typeof console.warn === 'function'",
        "console.error/warn",
    );
}

// ── Timers API ────────────────────────────────────────────────────────

#[test]
fn timers_set_timeout() {
    assert_js_true(
        "typeof setTimeout === 'function'",
        "setTimeout",
    );
}

#[test]
fn timers_set_interval() {
    assert_js_true(
        "typeof setInterval === 'function'",
        "setInterval",
    );
}

#[test]
fn timers_clear_timeout() {
    assert_js_true(
        "typeof clearTimeout === 'function'",
        "clearTimeout",
    );
}

#[test]
fn timers_clear_interval() {
    assert_js_true(
        "typeof clearInterval === 'function'",
        "clearInterval",
    );
}

// ── Events API ────────────────────────────────────────────────────────

#[test]
fn events_event_constructor() {
    assert_js_true(
        "new Event('click').type === 'click'",
        "Event constructor",
    );
}

#[test]
fn events_event_target() {
    assert_js_true(
        "typeof EventTarget === 'function' && (() => { const et = new EventTarget(); let fired = false; et.addEventListener('x', () => { fired = true; }); et.dispatchEvent(new Event('x')); return fired; })()",
        "EventTarget",
    );
}

#[test]
fn events_abort_controller() {
    assert_js_true(
        "(() => { const ac = new AbortController(); return ac.signal.aborted === false; })()",
        "AbortController",
    );
}

#[test]
fn events_abort_signal() {
    assert_js_true(
        "(() => { const ac = new AbortController(); ac.abort(); return ac.signal.aborted === true; })()",
        "AbortSignal.aborted",
    );
}

#[test]
fn events_custom_event() {
    assert_js_true(
        "new CustomEvent('foo', { detail: 42 }).detail === 42",
        "CustomEvent",
    );
}

// ── DOM API ───────────────────────────────────────────────────────────

#[test]
fn dom_exception() {
    assert_js_true(
        "new DOMException('oops', 'NotFoundError').name === 'NotFoundError'",
        "DOMException",
    );
}

#[test]
fn dom_structured_clone() {
    assert_js_true(
        "(() => { const obj = { a: 1, b: [2,3] }; const cloned = structuredClone(obj); return cloned.a === 1 && cloned.b[1] === 3 && cloned !== obj; })()",
        "structuredClone",
    );
}

#[test]
fn dom_blob() {
    assert_js_true(
        "new Blob(['hello']).size === 5",
        "Blob",
    );
}

#[test]
fn dom_file() {
    assert_js_true(
        "new File(['data'], 'test.txt').name === 'test.txt'",
        "File",
    );
}

#[test]
fn dom_file_reader() {
    assert_js_true(
        "typeof FileReader === 'function'",
        "FileReader",
    );
}

// ── Compression API ──────────────────────────────────────────────────

#[test]
fn compression_stream() {
    assert_js_true(
        "typeof CompressionStream === 'function'",
        "CompressionStream",
    );
}

#[test]
fn decompression_stream() {
    assert_js_true(
        "typeof DecompressionStream === 'function'",
        "DecompressionStream",
    );
}

// ── Performance API ──────────────────────────────────────────────────

#[test]
fn performance_now() {
    assert_js_true(
        "typeof performance === 'object' && typeof performance.now === 'function'",
        "performance.now",
    );
}

#[test]
fn performance_mark() {
    assert_js_true(
        "typeof PerformanceMark === 'function'",
        "PerformanceMark",
    );
}

// ── Messaging API ────────────────────────────────────────────────────

#[test]
fn messaging_message_channel() {
    assert_js_true(
        "(() => { const ch = new MessageChannel(); return ch.port1 instanceof MessagePort && ch.port2 instanceof MessagePort; })()",
        "MessageChannel",
    );
}

#[test]
fn messaging_message_port() {
    assert_js_true(
        "typeof MessagePort === 'function'",
        "MessagePort",
    );
}

#[test]
fn messaging_image_data() {
    assert_js_true(
        "typeof ImageData === 'function'",
        "ImageData",
    );
}

// ── HTMLRewriter API ──────────────────────────────────────
// Note: HTMLRewriter is not available in this runtime (Cloudflare-specific)
// Skipped: Not implemented in deno extensions

// ── WebSocket API ────────────────────────────────────────
// Note: WebSocket support varies by runtime extension
// Skipped: Testing for presence only, not full functionality

// ── Cache API ────────────────────────────────────────────
// Note: Cache API not available in basic deno extensions
// Skipped: Not implemented in this runtime

// ── Typed Arrays & ArrayBuffer ─────────────────────────

#[test]
fn typed_array_uint8array() {
    assert_js_true(
        "new Uint8Array(8).length === 8",
        "Uint8Array",
    );
}

#[test]
fn typed_array_int32array() {
    assert_js_true(
        "new Int32Array(4).length === 4",
        "Int32Array",
    );
}

#[test]
fn typed_array_float64array() {
    assert_js_true(
        "new Float64Array(2).length === 2",
        "Float64Array",
    );
}

#[test]
fn array_buffer_constructor() {
    assert_js_true(
        "new ArrayBuffer(16).byteLength === 16",
        "ArrayBuffer",
    );
}

#[test]
fn data_view_constructor() {
    assert_js_true(
        "new DataView(new ArrayBuffer(8)).byteLength === 8",
        "DataView",
    );
}

// ── JSON API ──────────────────────────────────────────

#[test]
fn json_stringify_parse() {
    assert_js_true(
        "JSON.parse(JSON.stringify({a: 1})).a === 1",
        "JSON stringify/parse",
    );
}

// ── Promise API ───────────────────────────────────────

#[test]
fn promise_constructor() {
    assert_js_true(
        "(() => { return new Promise((resolve) => { resolve(42); }) instanceof Promise; })()",
        "Promise constructor",
    );
}

// ── WeakMap / WeakSet ────────────────────────────────

#[test]
fn weak_map_constructor() {
    assert_js_true(
        "typeof WeakMap === 'function'",
        "WeakMap",
    );
}

#[test]
fn weak_set_constructor() {
    assert_js_true(
        "typeof WeakSet === 'function'",
        "WeakSet",
    );
}

// ── Map / Set ────────────────────────────────────────

#[test]
fn map_constructor() {
    assert_js_true(
        "(() => { const m = new Map([['a', 1]]); return m.get('a') === 1; })()",
        "Map",
    );
}

#[test]
fn set_constructor() {
    assert_js_true(
        "(() => { const s = new Set([1, 2, 2]); return s.size === 2; })()",
        "Set",
    );
}

// ── Symbol API ────────────────────────────────────────

#[test]
fn symbol_constructor() {
    assert_js_true(
        "typeof Symbol === 'function' && typeof Symbol('test') === 'symbol'",
        "Symbol",
    );
}

// ── Proxy & Reflect ──────────────────────────────────

#[test]
fn proxy_constructor() {
    assert_js_true(
        "typeof Proxy === 'function' && new Proxy({}, {}) !== undefined",
        "Proxy",
    );
}

#[test]
fn reflect_api() {
    assert_js_true(
        "typeof Reflect === 'object' && typeof Reflect.get === 'function'",
        "Reflect API",
    );
}

// ── Generator API ─────────────────────────────────────

#[test]
fn generator_function() {
    assert_js_true(
        "(() => { function* gen() { yield 1; } return typeof gen() === 'object'; })()",
        "Generator function",
    );
}

// ── Async / Await ─────────────────────────────────────

#[test]
fn async_function() {
    assert_js_true(
        "(() => { async function test() { return 42; } return test() instanceof Promise; })()",
        "Async function",
    );
}

// ── URL static methods ────────────────────────────────

#[test]
fn url_parse_method() {
    assert_js_true(
        "typeof URL.parse === 'function'",
        "URL.parse static method",
    );
}

// ── Math object ───────────────────────────────────────

#[test]
fn math_operations() {
    assert_js_true(
        "typeof Math === 'object' && typeof Math.random === 'function' && typeof Math.floor === 'function'",
        "Math object",
    );
}

// ── Date object ───────────────────────────────────────

#[test]
fn date_constructor() {
    assert_js_true(
        "new Date().getFullYear() > 2020",
        "Date constructor",
    );
}

// ── RegExp support ────────────────────────────────────

#[test]
fn regex_constructor() {
    assert_js_true(
        "(/test/).test('test') === true",
        "RegExp support",
    );
}

// ── Error types ───────────────────────────────────────

#[test]
fn error_types() {
    assert_js_true(
        "(() => { try { throw new TypeError('test'); } catch(e) { return e instanceof TypeError; } })()",
        "Error types (TypeError)",
    );
}

#[test]
fn range_error() {
    assert_js_true(
        "(() => { try { throw new RangeError('test'); } catch(e) { return e instanceof RangeError; } })()",
        "Error types (RangeError)",
    );
}

// ── parseInt / parseFloat ────────────────────────────

#[test]
fn parse_int_float() {
    assert_js_true(
        "parseInt('42') === 42 && parseFloat('3.14') === 3.14",
        "parseInt / parseFloat",
    );
}

// ── isNaN / isFinite ─────────────────────────────────

#[test]
fn global_nan_check() {
    assert_js_true(
        "isNaN(NaN) === true && isFinite(42) === true",
        "isNaN / isFinite",
    );
}

// ── String methods ────────────────────────────────────

#[test]
fn string_methods() {
    assert_js_true(
        "(() => { const s = 'hello'; return s.toUpperCase() === 'HELLO' && s.toLowerCase() === 'hello' && s.trim() === s; })()",
        "String methods (toUpperCase, toLowerCase, trim)",
    );
}

// ── Array methods ────────────────────────────────────

#[test]
fn array_methods() {
    assert_js_true(
        "(() => { const arr = [1,2,3]; return arr.map(x => x * 2)[1] === 4 && arr.filter(x => x > 1).length === 2 && arr.reduce((a,b) => a+b, 0) === 6; })()",
        "Array methods (map, filter, reduce)",
    );
}

#[test]
fn array_includes() {
    assert_js_true(
        "[1, 2, 3].includes(2) === true",
        "Array includes method",
    );
}

// ── Object methods ────────────────────────────────────

#[test]
fn object_methods() {
    assert_js_true(
        "(() => { const obj = {a: 1, b: 2}; return Object.keys(obj).length === 2 && Object.values(obj)[0] === 1; })()",
        "Object.keys / Object.values",
    );
}

#[test]
fn object_entries() {
    assert_js_true(
        "Object.entries({a: 1}).length === 1",
        "Object.entries",
    );
}

// ── Intl API ──────────────────────────────────────────

#[test]
fn intl_collator() {
    assert_js_true(
        "typeof Intl === 'object' && typeof Intl.Collator === 'function'",
        "Intl.Collator",
    );
}

#[test]
fn intl_date_time_format() {
    assert_js_true(
        "typeof Intl.DateTimeFormat === 'function'",
        "Intl.DateTimeFormat",
    );
}

#[test]
fn intl_number_format() {
    assert_js_true(
        "typeof Intl.NumberFormat === 'function'",
        "Intl.NumberFormat",
    );
}
