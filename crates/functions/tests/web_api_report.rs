//! Web Standards compatibility report generator.
//!
//! This test boots a JsRuntime with the production extensions and runs
//! a comprehensive set of JS checks. It then generates a markdown report
//! at `WEB_STANDARDS_REPORT.md` in the project root.

use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;
use std::fmt::Write as FmtWrite;

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

#[derive(Debug, Clone)]
struct ApiCheck {
    category: &'static str,
    api: &'static str,
    js_check: &'static str,
    /// "full", "partial", or "none"
    status: String,
}

/// Evaluate a JS expression and return its string result.
fn eval_js(runtime: &mut JsRuntime, js: &str) -> String {
    match runtime.execute_script("<check>", js.to_string()) {
        Ok(val) => {
            let scope = &mut runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, val);
            if let Some(s) = local.to_string(scope) {
                s.to_rust_string_lossy(scope)
            } else {
                "error".to_string()
            }
        }
        Err(_) => "none".to_string(),
    }
}

fn define_checks() -> Vec<ApiCheck> {
    // Each check returns a JS expression that evaluates to "full", "partial", or "none".
    vec![
        // ── Fetch API ──
        ApiCheck {
            category: "Fetch API",
            api: "Headers",
            js_check: r#"(() => { try { const h = new Headers({'x': 'y'}); return (h.get('x') === 'y' && typeof h.append === 'function' && typeof h.delete === 'function' && typeof h.entries === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "Request",
            js_check: r#"(() => { try { const r = new Request('http://example.com', {method: 'POST', headers: {'x':'y'}}); return (r.method === 'POST' && r.headers.get('x') === 'y' && typeof r.json === 'function' && typeof r.text === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "Response",
            js_check: r#"(() => { try { const r = new Response('ok', {status: 201, headers: {'x':'y'}}); return (r.status === 201 && r.headers.get('x') === 'y' && typeof r.json === 'function' && typeof r.text === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "fetch()",
            js_check: r#"(() => { return typeof fetch === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "FormData",
            js_check: r#"(() => { try { const fd = new FormData(); fd.append('k','v'); return (fd.get('k') === 'v' && typeof fd.entries === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "EventSource",
            js_check: r#"(() => { return typeof EventSource === 'function' ? 'partial' : 'none'; })()"#,
            status: String::new(),
        },

        // ── URL API ──
        ApiCheck {
            category: "URL API",
            api: "URL",
            js_check: r#"(() => { try { const u = new URL('https://example.com:8080/path?q=1#h'); return (u.hostname === 'example.com' && u.port === '8080' && u.pathname === '/path' && u.search === '?q=1' && u.hash === '#h') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "URL API",
            api: "URLSearchParams",
            js_check: r#"(() => { try { const p = new URLSearchParams('a=1&b=2&a=3'); return (p.get('b') === '2' && p.getAll('a').length === 2 && typeof p.entries === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "URL API",
            api: "URLPattern",
            js_check: r#"(() => { try { const p = new URLPattern({pathname: '/api/:id'}); const result = p.exec('http://example.com/api/123'); return (result && result.pathname.groups.id === '123') ? 'full' : 'partial'; } catch(e) { return typeof URLPattern === 'function' ? 'partial' : 'none'; } })()"#,
            status: String::new(),
        },

        // ── Streams API ──
        ApiCheck {
            category: "Streams API",
            api: "ReadableStream",
            js_check: r#"(() => { try { const rs = new ReadableStream({ start(c) { c.enqueue('data'); c.close(); } }); return (rs instanceof ReadableStream && typeof rs.getReader === 'function' && typeof rs.pipeTo === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "WritableStream",
            js_check: r#"(() => { try { const ws = new WritableStream(); return (ws instanceof WritableStream && typeof ws.getWriter === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "TransformStream",
            js_check: r#"(() => { try { const ts = new TransformStream(); return (ts.readable instanceof ReadableStream && ts.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "ByteLengthQueuingStrategy",
            js_check: r#"(() => { try { new ByteLengthQueuingStrategy({ highWaterMark: 1024 }); return 'full'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "CountQueuingStrategy",
            js_check: r#"(() => { try { new CountQueuingStrategy({ highWaterMark: 10 }); return 'full'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── Encoding API ──
        ApiCheck {
            category: "Encoding API",
            api: "TextEncoder",
            js_check: r#"(() => { try { const e = new TextEncoder(); const arr = e.encode('hello'); return (arr.length === 5 && arr instanceof Uint8Array) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "TextDecoder",
            js_check: r#"(() => { try { const d = new TextDecoder('utf-8'); const result = d.decode(new Uint8Array([72,101,108,108,111])); return result === 'Hello' ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "TextEncoderStream",
            js_check: r#"(() => { try { const s = new TextEncoderStream(); return (s.readable instanceof ReadableStream && s.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "TextDecoderStream",
            js_check: r#"(() => { try { const s = new TextDecoderStream(); return (s.readable instanceof ReadableStream && s.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "atob / btoa",
            js_check: r#"(() => { try { return (atob(btoa('test')) === 'test') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── Crypto API ──
        ApiCheck {
            category: "Crypto API",
            api: "crypto.getRandomValues()",
            js_check: r#"(() => { try { const arr = new Uint8Array(16); crypto.getRandomValues(arr); return arr.some(v => v !== 0) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Crypto API",
            api: "crypto.randomUUID()",
            js_check: r#"(() => { try { const uuid = crypto.randomUUID(); return (typeof uuid === 'string' && uuid.length === 36 && uuid.split('-').length === 5) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Crypto API",
            api: "crypto.subtle",
            js_check: r#"(() => { try { return (typeof crypto.subtle === 'object' && typeof crypto.subtle.digest === 'function' && typeof crypto.subtle.generateKey === 'function' && typeof crypto.subtle.sign === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Crypto API",
            api: "CryptoKey",
            js_check: r#"(() => { return typeof CryptoKey === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },

        // ── Console API ──
        ApiCheck {
            category: "Console API",
            api: "console.log / error / warn",
            js_check: r#"(() => { return (typeof console === 'object' && typeof console.log === 'function' && typeof console.error === 'function' && typeof console.warn === 'function' && typeof console.info === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Console API",
            api: "console.table / trace / dir",
            js_check: r#"(() => { return (typeof console.table === 'function' && typeof console.trace === 'function' && typeof console.dir === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── Timers API ──
        ApiCheck {
            category: "Timers API",
            api: "setTimeout / clearTimeout",
            js_check: r#"(() => { return (typeof setTimeout === 'function' && typeof clearTimeout === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Timers API",
            api: "setInterval / clearInterval",
            js_check: r#"(() => { return (typeof setInterval === 'function' && typeof clearInterval === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },

        // ── Events API ──
        ApiCheck {
            category: "Events API",
            api: "Event",
            js_check: r#"(() => { try { const e = new Event('test', {bubbles: true}); return (e.type === 'test' && e.bubbles === true) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "EventTarget",
            js_check: r#"(() => { try { const et = new EventTarget(); let fired = false; et.addEventListener('x', () => { fired = true; }); et.dispatchEvent(new Event('x')); return fired ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "CustomEvent",
            js_check: r#"(() => { try { const e = new CustomEvent('foo', { detail: { key: 42 } }); return (e.type === 'foo' && e.detail.key === 42) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "AbortController / AbortSignal",
            js_check: r#"(() => { try { const ac = new AbortController(); ac.abort('reason'); return (ac.signal.aborted === true && typeof ac.signal.addEventListener === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "ErrorEvent",
            js_check: r#"(() => { try { const e = new ErrorEvent('error', {message: 'oops'}); return e.message === 'oops' ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "PromiseRejectionEvent",
            js_check: r#"(() => { try { return typeof PromiseRejectionEvent === 'function' ? 'full' : 'none'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── DOM API ──
        ApiCheck {
            category: "DOM API",
            api: "DOMException",
            js_check: r#"(() => { try { const e = new DOMException('msg', 'NotFoundError'); return (e.name === 'NotFoundError' && e.message === 'msg' && e.code === 8) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "DOM API",
            api: "structuredClone",
            js_check: r#"(() => { try { const obj = {a: 1, b: [2,3], c: new Date()}; const clone = structuredClone(obj); return (clone.a === 1 && clone.b[1] === 3 && clone !== obj && clone.c instanceof Date) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── File API ──
        ApiCheck {
            category: "File API",
            api: "Blob",
            js_check: r#"(() => { try { const b = new Blob(['hello', ' world'], {type: 'text/plain'}); return (b.size === 11 && b.type === 'text/plain' && typeof b.text === 'function' && typeof b.arrayBuffer === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "File API",
            api: "File",
            js_check: r#"(() => { try { const f = new File(['data'], 'test.txt', {type: 'text/plain'}); return (f.name === 'test.txt' && f.size === 4 && f.type === 'text/plain') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "File API",
            api: "FileReader",
            js_check: r#"(() => { return typeof FileReader === 'function' ? 'partial' : 'none'; })()"#,
            status: String::new(),
        },

        // ── Compression API ──
        ApiCheck {
            category: "Compression API",
            api: "CompressionStream",
            js_check: r#"(() => { try { const cs = new CompressionStream('gzip'); return (cs.readable instanceof ReadableStream && cs.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Compression API",
            api: "DecompressionStream",
            js_check: r#"(() => { try { const ds = new DecompressionStream('gzip'); return (ds.readable instanceof ReadableStream && ds.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── Performance API ──
        ApiCheck {
            category: "Performance API",
            api: "performance.now()",
            js_check: r#"(() => { try { const t = performance.now(); return (typeof t === 'number' && t >= 0) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Performance API",
            api: "PerformanceMark / measure",
            js_check: r#"(() => { try { return (typeof PerformanceMark === 'function' && typeof PerformanceMeasure === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── Messaging API ──
        ApiCheck {
            category: "Messaging API",
            api: "MessageChannel / MessagePort",
            js_check: r#"(() => { try { const ch = new MessageChannel(); return (ch.port1 instanceof MessagePort && ch.port2 instanceof MessagePort && typeof ch.port1.postMessage === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Messaging API",
            api: "ImageData",
            js_check: r#"(() => { try { const id = new ImageData(2, 2); return (id.width === 2 && id.height === 2 && id.data.length === 16) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── HTMLRewriter API ──
        ApiCheck {
            category: "HTML Rewriter",
            api: "HTMLRewriter",
            js_check: r#"(() => { return typeof HTMLRewriter === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },

        // ── Typed Arrays ──
        ApiCheck {
            category: "Typed Arrays",
            api: "Uint8Array",
            js_check: r#"(() => { try { return (new Uint8Array(8).length === 8) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Typed Arrays",
            api: "Int32Array / Float64Array",
            js_check: r#"(() => { try { return (new Int32Array(4).length === 4 && new Float64Array(2).length === 2) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Typed Arrays",
            api: "ArrayBuffer / DataView",
            js_check: r#"(() => { try { return (new ArrayBuffer(16).byteLength === 16 && new DataView(new ArrayBuffer(8)).byteLength === 8) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── JSON API ──
        ApiCheck {
            category: "JSON API",
            api: "JSON stringify / parse",
            js_check: r#"(() => { try { return (JSON.parse(JSON.stringify({a: 1})).a === 1) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── Promise API ──
        ApiCheck {
            category: "Promise API",
            api: "Promise constructor",
            js_check: r#"(() => { try { return (new Promise(r => r(42)) instanceof Promise) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── Collections ──
        ApiCheck {
            category: "Collections",
            api: "Map / Set",
            js_check: r#"(() => { try { const m = new Map([['a', 1]]); const s = new Set([1, 2, 2]); return (m.get('a') === 1 && s.size === 2) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Collections",
            api: "WeakMap / WeakSet",
            js_check: r#"(() => { return (typeof WeakMap === 'function' && typeof WeakSet === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── Symbol API ──
        ApiCheck {
            category: "Symbol API",
            api: "Symbol",
            js_check: r#"(() => { return (typeof Symbol === 'function' && typeof Symbol('test') === 'symbol') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── Proxy & Reflect ──
        ApiCheck {
            category: "Proxy & Reflect",
            api: "Proxy",
            js_check: r#"(() => { try { return (typeof Proxy === 'function' && new Proxy({}, {}) !== undefined) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Proxy & Reflect",
            api: "Reflect API",
            js_check: r#"(() => { return (typeof Reflect === 'object' && typeof Reflect.get === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── Generator & Async ──
        ApiCheck {
            category: "Generators & Async",
            api: "Generator function",
            js_check: r#"(() => { try { function* gen() { yield 1; } return (typeof gen() === 'object') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Generators & Async",
            api: "Async function",
            js_check: r#"(() => { try { async function test() { return 42; } return (test() instanceof Promise) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },

        // ── String & Array methods ──
        ApiCheck {
            category: "String & Array Methods",
            api: "String methods",
            js_check: r#"(() => { const s = 'hello'; return (s.toUpperCase() === 'HELLO' && s.toLowerCase() === 'hello') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "String & Array Methods",
            api: "Array methods",
            js_check: r#"(() => { const arr = [1,2,3]; return (arr.map(x => x * 2)[1] === 4 && arr.filter(x => x > 1).length === 2) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "String & Array Methods",
            api: "Object methods",
            js_check: r#"(() => { const obj = {a: 1, b: 2}; return (Object.keys(obj).length === 2 && Object.values(obj)[0] === 1) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── Intl API ──
        ApiCheck {
            category: "Intl API",
            api: "Intl.Collator / DateTimeFormat / NumberFormat",
            js_check: r#"(() => { return (typeof Intl === 'object' && typeof Intl.Collator === 'function' && typeof Intl.DateTimeFormat === 'function' && typeof Intl.NumberFormat === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── URL enhancements ──
        ApiCheck {
            category: "URL API",
            api: "URL.parse static method",
            js_check: r#"(() => { return (typeof URL.parse === 'function' && URL.parse('https://example.com') !== null) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── Math & Date ──
        ApiCheck {
            category: "Built-in Objects",
            api: "Math object",
            js_check: r#"(() => { return (typeof Math === 'object' && typeof Math.random === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Built-in Objects",
            api: "Date object",
            js_check: r#"(() => { return (new Date().getFullYear() > 2020) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Built-in Objects",
            api: "RegExp support",
            js_check: r#"(() => { return ((/test/).test('test') === true) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },

        // ── NOT IMPLEMENTED ──
        ApiCheck {
            category: "WebSocket API",
            api: "WebSocket",
            js_check: r#"(() => { return typeof WebSocket === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Web Workers",
            api: "Worker",
            js_check: r#"(() => { return typeof Worker === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Cache API",
            api: "CacheStorage / Cache",
            js_check: r#"(() => { return (typeof caches === 'object' && typeof caches.open === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "WebGPU",
            api: "GPU / GPUDevice",
            js_check: r#"(() => { return typeof GPU === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Service Workers",
            api: "ServiceWorker",
            js_check: r#"(() => { return typeof ServiceWorkerGlobalScope === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Web Notifications",
            api: "Notification",
            js_check: r#"(() => { return typeof Notification === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "IndexedDB",
            api: "indexedDB",
            js_check: r#"(() => { return typeof indexedDB === 'object' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
    ]
}

#[test]
fn generate_web_standards_report() {
    let mut runtime = make_runtime();
    let mut checks = define_checks();

    // Run all checks
    for check in checks.iter_mut() {
        check.status = eval_js(&mut runtime, check.js_check);
        // Normalize unexpected values
        if !["full", "partial", "none"].contains(&check.status.as_str()) {
            check.status = "none".to_string();
        }
    }

    // Count stats
    let total = checks.len();
    let full = checks.iter().filter(|c| c.status == "full").count();
    let partial = checks.iter().filter(|c| c.status == "partial").count();
    let none = checks.iter().filter(|c| c.status == "none").count();

    // Build report
    let mut report = String::new();
    writeln!(report, "# Web Standards Compatibility Report").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "> Auto-generated by `web_api_report` test. Do not edit manually.").unwrap();
    writeln!(report).unwrap();

    // Summary
    writeln!(report, "## Summary").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Status | Count | Percentage |").unwrap();
    writeln!(report, "|--------|------:|------------|").unwrap();
    writeln!(report, "| Full | {full} | {:.0}% |", full as f64 / total as f64 * 100.0).unwrap();
    writeln!(report, "| Partial | {partial} | {:.0}% |", partial as f64 / total as f64 * 100.0).unwrap();
    writeln!(report, "| None | {none} | {:.0}% |", none as f64 / total as f64 * 100.0).unwrap();
    writeln!(report, "| **Total** | **{total}** | **100%** |").unwrap();
    writeln!(report).unwrap();

    // Detailed table
    writeln!(report, "## Detailed Results").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Category | API | Status |").unwrap();
    writeln!(report, "|----------|-----|--------|").unwrap();

    for check in &checks {
        let icon = match check.status.as_str() {
            "full" => "Full",
            "partial" => "Partial",
            "none" => "None",
            _ => "?",
        };
        writeln!(report, "| {} | {} | {} |", check.category, check.api, icon).unwrap();
    }
    writeln!(report).unwrap();

    // Category summary
    writeln!(report, "## Category Summary").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Category | Full | Partial | None | Total |").unwrap();
    writeln!(report, "|----------|-----:|--------:|-----:|------:|").unwrap();

    let categories: Vec<&str> = {
        let mut cats: Vec<&str> = checks.iter().map(|c| c.category).collect();
        cats.dedup();
        cats
    };

    // Deduplicate categories properly
    let mut unique_cats: Vec<&str> = Vec::new();
    for c in &checks {
        if !unique_cats.contains(&c.category) {
            unique_cats.push(c.category);
        }
    }

    for cat in &unique_cats {
        let cat_checks: Vec<&ApiCheck> = checks.iter().filter(|c| c.category == *cat).collect();
        let cat_full = cat_checks.iter().filter(|c| c.status == "full").count();
        let cat_partial = cat_checks.iter().filter(|c| c.status == "partial").count();
        let cat_none = cat_checks.iter().filter(|c| c.status == "none").count();
        let cat_total = cat_checks.len();
        writeln!(report, "| {cat} | {cat_full} | {cat_partial} | {cat_none} | {cat_total} |").unwrap();
    }
    writeln!(report).unwrap();

    // Extensions loaded
    writeln!(report, "## Runtime Extensions").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "The following Deno extensions are loaded:").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "- `deno_webidl` - WebIDL bindings").unwrap();
    writeln!(report, "- `deno_console` - Console API").unwrap();
    writeln!(report, "- `deno_url` - URL / URLSearchParams / URLPattern").unwrap();
    writeln!(report, "- `deno_web` - Web APIs (Events, Timers, Streams, Encoding, Blob, File, etc.)").unwrap();
    writeln!(report, "- `deno_crypto` - Web Crypto API").unwrap();
    writeln!(report, "- `deno_telemetry` - OpenTelemetry support").unwrap();
    writeln!(report, "- `deno_fetch` - Fetch API (Headers, Request, Response, fetch)").unwrap();
    writeln!(report, "- `deno_net` - TCP/TLS networking").unwrap();
    writeln!(report, "- `deno_tls` - TLS support").unwrap();
    writeln!(report, "- `edge_bootstrap` - Bootstrap module that wires everything to globalThis").unwrap();
    writeln!(report).unwrap();

    // write to file
    let report_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("WEB_STANDARDS_REPORT.md");

    std::fs::write(&report_path, &report).unwrap_or_else(|e| {
        panic!("Failed to write report to {}: {e}", report_path.display());
    });

    // Also print to stdout for CI visibility
    println!("\n{report}");
    println!("Report written to: {}", report_path.display());

    // Remove unused variable warning
    let _ = categories;
}
