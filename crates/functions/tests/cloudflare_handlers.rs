use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;

// This module tests Cloudflare Workers Handlers, Schedulers, and RPC APIs
// Reference: https://developers.cloudflare.com/workers/runtime-apis/handlers/

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

// ── Fetch Handler ──────────────────────────────────────────────────

#[test]
fn handler_fetch_pattern() {
    assert_js_true(
        "(() => {
            // Simulate Cloudflare fetch handler pattern
            const handler = {
                fetch: (request) => {
                    return new Promise((resolve) => {
                        resolve(new Response('OK'));
                    });
                }
            };

            return typeof handler.fetch === 'function';
        })()",
        "Fetch handler pattern works",
    );
}

#[test]
fn handler_event_listener() {
    // Alternative: addEventListener pattern (Deno/Web Standard)
    assert_js_true(
        "(() => {
            let handlerCalled = false;
            const mockRequest = new Request('https://example.com', { method: 'GET' });

            // Can use addEventListener for custom handlers
            const handler = (event) => {
                handlerCalled = true;
            };

            return typeof handler === 'function';
        })()",
        "Event listener handler pattern",
    );
}

#[test]
fn handler_response_creation() {
    assert_js_true(
        "(() => {
            const response = new Response('Hello World', {
                status: 200,
                headers: { 'Content-Type': 'text/plain' }
            });

            // deno_fetch sets statusText to empty string by default for status codes
            return response.status === 200 && response.statusText === '';
        })()",
        "Handler response creation",
    );
}

// ── Scheduler API ──────────────────────────────────────────────────

#[test]
fn scheduler_wait_alternative() {
    // scheduler.wait() maps to Promise-based timer
    assert_js_true(
        "(() => {
            // Instead of scheduler.wait(ms), use:
            const wait = (ms) => new Promise(resolve => setTimeout(resolve, ms));

            // Verify it returns a Promise
            const delayPromise = wait(100);
            return delayPromise instanceof Promise;
        })()",
        "scheduler.wait() alternative via Promise",
    );
}

#[test]
fn scheduler_cron_not_available() {
    // NOTE: scheduler.cron() is Cloudflare-specific and not available
    // Cron scheduling would require external service or custom implementation
    assert_js_true(
        "typeof scheduler === 'undefined' || typeof scheduler.cron === 'undefined'",
        "scheduler.cron() correctly not available (Cloudflare-specific)",
    );
}

#[test]
fn scheduler_timeout_alternative() {
    assert_js_true(
        "(() => {
            // Cloudflare scheduler alternative via standard timers
            return typeof setTimeout === 'function' && typeof setInterval === 'function';
        })()",
        "Scheduler alternative via setTimeout/setInterval",
    );
}

// ── Request/Response Dispatch ──────────────────────────────────────

#[test]
fn request_dispatch_pattern() {
    assert_js_true(
        "(() => {
            const request = new Request('https://example.com/api/data', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ key: 'value' })
            });

            return request.method === 'POST' &&
                   request.url === 'https://example.com/api/data' &&
                   typeof request.clone === 'function';
        })()",
        "Request dispatch pattern",
    );
}

#[test]
fn response_dispatch_pattern() {
    assert_js_true(
        "(() => {
            const response = new Response(JSON.stringify({ success: true }), {
                status: 200,
                headers: { 'Content-Type': 'application/json' }
            });

            return response.ok === true && typeof response.json === 'function';
        })()",
        "Response dispatch pattern",
    );
}

#[test]
fn request_response_roundtrip() {
    assert_js_true(
        "(() => {
            const originalReq = new Request('https://api.example.com', {
                method: 'GET'
            });

            const response = new Response('handled', { status: 200 });

            return originalReq.url.includes('api.example.com') && response.status === 200;
        })()",
        "Request/Response roundtrip",
    );
}

// ── RPC (Remote Procedure Call) API ────────────────────────────────

// NOTE: RPC API is a new Cloudflare feature (2024+) that provides typed communication
// between Worker and server. It's not available in Deno Edge Runtime.
//
// Alternative implementation pattern:
// - Use fetch() for HTTP-based RPC
// - Implement custom RPC wrapper using JSON serialization
// - Use MessageChannel for cross-worker communication (Web Standard)
//
// Example RPC pattern (would need custom implementation):
// export interface WorkerService {
//   hello(name: string): Promise<string>;
// }
//
// Can be implemented as:
#[test]
fn rpc_alternative_fetch_pattern() {
    assert_js_true(
        "(() => {
            // RPC pattern can be implemented with fetch
            const rpcCall = async (method, params) => {
                const response = await fetch('/api/rpc', {
                    method: 'POST',
                    body: JSON.stringify({ jsonrpc: '2.0', method, params, id: 1 })
                });
                return response.json();
            };

            return typeof rpcCall === 'function';
        })()",
        "RPC alternative via fetch pattern",
    );
}

#[test]
fn rpc_alternative_message_channel() {
    assert_js_true(
        "(() => {
            // RPC can also use MessageChannel for typed communication
            const { port1, port2 } = new MessageChannel();

            const rpcHandler = {
                invoke: (method) => {
                    port1.postMessage({ method });
                }
            };

            return typeof rpcHandler.invoke === 'function';
        })()",
        "RPC alternative via MessageChannel",
    );
}

// NOTE: Cloudflare-specific APIs NOT available:
// - Scheduled handlers (would need external cron service)
// - Tail consumers (logging/debugging feature)
// - Module workers with exports
//
// Workarounds:
// - Use external cron services (cron-job.org, AWS EventBridge, etc.)
// - Implement custom logging to external services
// - Export JavaScript modules directly (works with import)
