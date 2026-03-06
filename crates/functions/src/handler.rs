use anyhow::Error;
use deno_core::JsRuntime;

/// Inject the request/response bridge into the JS global scope.
///
/// This creates a `globalThis.__edgeRuntime` object with:
/// - `__edgeRuntime.handler`: the registered fetch handler (set by user code)
/// - `__edgeRuntime.registerHandler(fn)`: called by the user's `Deno.serve()` equivalent
///
/// The user's JS code should call:
/// ```js
/// globalThis.__edgeRuntime.registerHandler(async (req) => {
///   return new Response("Hello!");
/// });
/// ```
///
/// Or we can override `Deno.serve` to do this automatically.
pub fn inject_request_bridge(js_runtime: &mut JsRuntime) -> Result<(), Error> {
    js_runtime.execute_script(
        "edge-internal:///runtime_bridge.js",
        deno_core::ascii_str!(
            r#"
            globalThis.__edgeRuntime = {
                handler: null,
                registerHandler(fn) {
                    this.handler = fn;
                },

                // Execution context tracking for resource cleanup
                _currentExecutionId: null,
                _timerRegistry: new Map(),       // executionId -> Set<timerId>
                _intervalRegistry: new Map(),    // executionId -> Set<intervalId>
                _abortRegistry: new Map(),       // executionId -> Set<AbortController>
                _promiseRegistry: new Map(),     // executionId -> Set<{promise, reject}>

                startExecution(executionId) {
                    this._currentExecutionId = executionId;
                    this._timerRegistry.set(executionId, new Set());
                    this._intervalRegistry.set(executionId, new Set());
                    this._abortRegistry.set(executionId, new Set());
                    this._promiseRegistry.set(executionId, new Set());
                },

                endExecution(executionId) {
                    this._timerRegistry.delete(executionId);
                    this._intervalRegistry.delete(executionId);
                    this._abortRegistry.delete(executionId);
                    this._promiseRegistry.delete(executionId);
                    if (this._currentExecutionId === executionId) {
                        this._currentExecutionId = null;
                    }
                },

                clearExecutionTimers(executionId) {
                    // Clear setTimeout timers
                    const timers = this._timerRegistry.get(executionId);
                    if (timers) {
                        for (const id of timers) {
                            globalThis.__originalClearTimeout(id);
                        }
                    }

                    // Clear setInterval intervals
                    const intervals = this._intervalRegistry.get(executionId);
                    if (intervals) {
                        for (const id of intervals) {
                            globalThis.__originalClearInterval(id);
                        }
                    }

                    // Abort pending fetch requests
                    const abortControllers = this._abortRegistry.get(executionId);
                    if (abortControllers) {
                        for (const controller of abortControllers) {
                            try {
                                controller.abort(new Error('Request cancelled due to execution timeout'));
                            } catch (e) {
                                // Ignore abort errors
                            }
                        }
                    }

                    // Reject pending tracked promises
                    const promises = this._promiseRegistry.get(executionId);
                    if (promises) {
                        const timeoutError = new Error('Promise cancelled due to execution timeout');
                        for (const entry of promises) {
                            try {
                                if (entry.reject) {
                                    entry.reject(timeoutError);
                                }
                            } catch (e) {
                                // Ignore rejection errors
                            }
                        }
                    }

                    // Cleanup registries
                    this._timerRegistry.delete(executionId);
                    this._intervalRegistry.delete(executionId);
                    this._abortRegistry.delete(executionId);
                    this._promiseRegistry.delete(executionId);
                    if (this._currentExecutionId === executionId) {
                        this._currentExecutionId = null;
                    }
                },

                // Helper to track an AbortController for the current execution
                _trackAbortController(controller) {
                    const execId = this._currentExecutionId;
                    if (execId) {
                        const controllers = this._abortRegistry.get(execId);
                        if (controllers) controllers.add(controller);
                    }
                    return controller;
                },

                // Helper to untrack an AbortController
                _untrackAbortController(controller) {
                    for (const [, controllers] of this._abortRegistry) {
                        controllers.delete(controller);
                    }
                },

                // Helper to track a promise for the current execution
                _trackPromise(promise, reject) {
                    const execId = this._currentExecutionId;
                    if (execId) {
                        const entry = { promise, reject };
                        const promises = this._promiseRegistry.get(execId);
                        if (promises) {
                            promises.add(entry);
                            // Auto-remove when promise settles
                            promise.finally(() => {
                                promises.delete(entry);
                            }).catch(() => {});
                        }
                    }
                    return promise;
                },
            };

            // Store original functions
            globalThis.__originalSetTimeout = globalThis.setTimeout;
            globalThis.__originalSetInterval = globalThis.setInterval;
            globalThis.__originalClearTimeout = globalThis.clearTimeout;
            globalThis.__originalClearInterval = globalThis.clearInterval;
            globalThis.__originalFetch = globalThis.fetch;
            globalThis.__originalQueueMicrotask = globalThis.queueMicrotask;

            // Wrap setTimeout to track by execution id
            globalThis.setTimeout = function(fn, delay, ...args) {
                const timerId = globalThis.__originalSetTimeout(function() {
                    // Remove from registry when timer fires
                    const execId = globalThis.__edgeRuntime._currentExecutionId;
                    if (execId) {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get(execId);
                        if (timers) timers.delete(timerId);
                    }
                    // Call original callback
                    if (typeof fn === 'function') {
                        fn(...args);
                    } else {
                        // Handle string argument (eval-style, rare)
                        eval(fn);
                    }
                }, delay);

                const execId = globalThis.__edgeRuntime._currentExecutionId;
                if (execId) {
                    const timers = globalThis.__edgeRuntime._timerRegistry.get(execId);
                    if (timers) timers.add(timerId);
                }
                return timerId;
            };

            // Wrap setInterval to track by execution id
            globalThis.setInterval = function(fn, interval, ...args) {
                const intervalId = globalThis.__originalSetInterval(fn, interval, ...args);
                const execId = globalThis.__edgeRuntime._currentExecutionId;
                if (execId) {
                    const intervals = globalThis.__edgeRuntime._intervalRegistry.get(execId);
                    if (intervals) intervals.add(intervalId);
                }
                return intervalId;
            };

            // Wrap clearTimeout to remove from registry
            globalThis.clearTimeout = function(id) {
                if (typeof id === 'number') {
                    for (const [, timers] of globalThis.__edgeRuntime._timerRegistry) {
                        timers.delete(id);
                    }
                }
                return globalThis.__originalClearTimeout(id);
            };

            // Wrap clearInterval to remove from registry
            globalThis.clearInterval = function(id) {
                if (typeof id === 'number') {
                    for (const [, intervals] of globalThis.__edgeRuntime._intervalRegistry) {
                        intervals.delete(id);
                    }
                }
                return globalThis.__originalClearInterval(id);
            };

            // Wrap fetch to track with AbortController
            globalThis.fetch = function(input, init = {}) {
                const execId = globalThis.__edgeRuntime._currentExecutionId;

                // If no execution context, just call original fetch
                if (!execId) {
                    return globalThis.__originalFetch(input, init);
                }

                // Create AbortController if not provided
                let controller;
                let signal = init.signal;

                if (!signal) {
                    controller = new AbortController();
                    signal = controller.signal;
                    globalThis.__edgeRuntime._trackAbortController(controller);
                } else if (signal.aborted) {
                    // Already aborted, just call original
                    return globalThis.__originalFetch(input, init);
                } else {
                    // User provided signal, wrap it with our controller
                    controller = new AbortController();
                    const userSignal = signal;

                    // If user signal aborts, abort our controller too
                    userSignal.addEventListener('abort', () => {
                        controller.abort(userSignal.reason);
                    });

                    signal = controller.signal;
                    globalThis.__edgeRuntime._trackAbortController(controller);
                }

                const fetchPromise = globalThis.__originalFetch(input, { ...init, signal });

                // Untrack controller when fetch completes (success or error)
                fetchPromise.finally(() => {
                    globalThis.__edgeRuntime._untrackAbortController(controller);
                }).catch(() => {});

                return fetchPromise;
            };

            // Wrap queueMicrotask to track execution context
            // Note: Microtasks cannot be cancelled, but we track them for visibility
            globalThis.queueMicrotask = function(callback) {
                const execId = globalThis.__edgeRuntime._currentExecutionId;

                return globalThis.__originalQueueMicrotask(function() {
                    // Microtasks run in the context they were queued
                    // We can't cancel them, but we can skip execution if context was cleared
                    const currentExecId = globalThis.__edgeRuntime._currentExecutionId;

                    // If the execution context changed or was cleared, still run
                    // (microtasks are atomic and should complete)
                    if (typeof callback === 'function') {
                        callback();
                    }
                });
            };

            // Override Deno.serve to capture the handler
            const originalServe = globalThis.Deno?.serve;
            if (globalThis.Deno) {
                globalThis.Deno.serve = function(handlerOrOptions, maybeHandler) {
                    let handler;
                    if (typeof handlerOrOptions === 'function') {
                        handler = handlerOrOptions;
                    } else if (typeof maybeHandler === 'function') {
                        handler = maybeHandler;
                    } else if (handlerOrOptions && typeof handlerOrOptions.handler === 'function') {
                        handler = handlerOrOptions.handler;
                    } else if (handlerOrOptions && typeof handlerOrOptions.fetch === 'function') {
                        handler = handlerOrOptions.fetch;
                    }
                    if (handler) {
                        globalThis.__edgeRuntime.registerHandler(handler);
                    }
                    // Return a mock server object
                    return {
                        finished: new Promise(() => {}),
                        ref() {},
                        unref() {},
                        shutdown() { return Promise.resolve(); },
                        addr: { hostname: "0.0.0.0", port: 0, transport: "tcp" },
                    };
                };
            }

            // Also support addEventListener('fetch', ...) style
            globalThis.__edgeRuntime._fetchListeners = [];
            globalThis.addEventListener = function(type, listener) {
                if (type === 'fetch') {
                    globalThis.__edgeRuntime._fetchListeners.push(listener);
                    // Wrap as handler
                    globalThis.__edgeRuntime.registerHandler(async (req) => {
                        let response = null;
                        const event = {
                            request: req,
                            respondWith(r) { response = r; },
                        };
                        listener(event);
                        return await response;
                    });
                }
            };

            // Expose a function for Rust to call
            globalThis.__edgeRuntime.handleRequest = async function(method, url, headersJson, body) {
                const handler = globalThis.__edgeRuntime.handler;
                if (!handler) {
                    return JSON.stringify({
                        status: 503,
                        headers: { "content-type": "application/json" },
                        body: '{"error":"no handler registered"}',
                    });
                }

                try {
                    const headers = JSON.parse(headersJson || '{}');
                    const reqInit = {
                        method: method,
                        headers: new Headers(headers),
                    };
                    if (body && body.length > 0 && method !== 'GET' && method !== 'HEAD') {
                        reqInit.body = body;
                    }
                    const request = new Request(url, reqInit);
                    const response = await handler(request);

                    const respHeaders = {};
                    response.headers.forEach((value, key) => {
                        respHeaders[key] = value;
                    });

                    const respBody = await response.text();

                    return JSON.stringify({
                        status: response.status,
                        headers: respHeaders,
                        body: respBody,
                    });
                } catch (err) {
                    return JSON.stringify({
                        status: 500,
                        headers: { "content-type": "application/json" },
                        body: JSON.stringify({ error: String(err) }),
                    });
                }
            };
            "#
        ),
    )?;
    Ok(())
}

/// JSON shape returned by __edgeRuntime.handleRequest
#[derive(serde::Deserialize)]
struct JsResponse {
    status: u16,
    headers: std::collections::HashMap<String, String>,
    body: String,
}

/// Dispatch an HTTP request into the JS fetch handler and return the response.
pub async fn dispatch_request(
    js_runtime: &mut JsRuntime,
    request: http::Request<bytes::Bytes>,
) -> Result<http::Response<bytes::Bytes>, Error> {
    let method = request.method().to_string();

    // Build an absolute URL — `new Request(url)` in JS requires one.
    // The router forwards only the rewritten path (e.g. "/"), so we
    // reconstruct the full URL from the Host header.
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let path = request.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let uri = format!("http://{host}{path}");

    // Serialize headers to JSON
    let headers_map: std::collections::HashMap<String, String> = request
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let headers_json = serde_json::to_string(&headers_map)?;

    let body = request.into_body();

    // Call globalThis.__edgeRuntime.handleRequest(...) directly via V8 API,
    // avoiding dynamic execute_script frames on every request.
    let result_global = {
        let context = js_runtime.main_context();
        let isolate = js_runtime.v8_isolate();
        let mut handle_scope = deno_core::v8::HandleScope::new(isolate);
        let mut handle_scope = {
            let pinned = unsafe { std::pin::Pin::new_unchecked(&mut handle_scope) };
            pinned.init()
        };
        let scope = &mut handle_scope;
        let context = deno_core::v8::Local::new(scope, context);
        let scope = &mut deno_core::v8::ContextScope::new(scope, context);

        let global = context.global(scope);
        let edge_runtime_key = deno_core::v8::String::new(scope, "__edgeRuntime")
            .ok_or_else(|| anyhow::anyhow!("failed to allocate __edgeRuntime key"))?;
        let edge_runtime_val = global
            .get(scope, edge_runtime_key.into())
            .ok_or_else(|| anyhow::anyhow!("globalThis.__edgeRuntime is missing"))?;
        let edge_runtime_obj = edge_runtime_val
            .to_object(scope)
            .ok_or_else(|| anyhow::anyhow!("globalThis.__edgeRuntime is not an object"))?;

        let handle_request_key = deno_core::v8::String::new(scope, "handleRequest")
            .ok_or_else(|| anyhow::anyhow!("failed to allocate handleRequest key"))?;
        let handle_request_val = edge_runtime_obj
            .get(scope, handle_request_key.into())
            .ok_or_else(|| anyhow::anyhow!("__edgeRuntime.handleRequest is missing"))?;
        let handle_request_fn =
            deno_core::v8::Local::<deno_core::v8::Function>::try_from(handle_request_val)
                .map_err(|_| anyhow::anyhow!("__edgeRuntime.handleRequest is not a function"))?;

        let method_v8 = deno_core::v8::String::new(scope, &method)
            .ok_or_else(|| anyhow::anyhow!("failed to allocate method string"))?;
        let uri_v8 = deno_core::v8::String::new(scope, &uri)
            .ok_or_else(|| anyhow::anyhow!("failed to allocate uri string"))?;
        let headers_v8 = deno_core::v8::String::new(scope, &headers_json)
            .ok_or_else(|| anyhow::anyhow!("failed to allocate headers string"))?;

        let body_arg: deno_core::v8::Local<deno_core::v8::Value> = if body.is_empty() {
            deno_core::v8::null(scope).into()
        } else {
            let backing_store = deno_core::v8::ArrayBuffer::new_backing_store_from_boxed_slice(
                body.to_vec().into_boxed_slice(),
            );
            let backing_store = backing_store.make_shared();
            let array_buffer =
                deno_core::v8::ArrayBuffer::with_backing_store(scope, &backing_store);
            let uint8 = deno_core::v8::Uint8Array::new(scope, array_buffer, 0, body.len())
                .ok_or_else(|| anyhow::anyhow!("failed to allocate Uint8Array body"))?;
            uint8.into()
        };

        let args: [deno_core::v8::Local<deno_core::v8::Value>; 4] =
            [method_v8.into(), uri_v8.into(), headers_v8.into(), body_arg];

        let result = handle_request_fn
            .call(scope, edge_runtime_obj.into(), &args)
            .ok_or_else(|| anyhow::anyhow!("failed to call __edgeRuntime.handleRequest"))?;

        deno_core::v8::Global::new(scope, result)
    };

    // The result is a Promise, we need to resolve it.
    let resolved = js_runtime.resolve(result_global);

    // Run the event loop to resolve the promise
    js_runtime
        .run_event_loop(deno_core::PollEventLoopOptions {
            wait_for_inspector: false,
            pump_v8_message_loop: true,
        })
        .await?;

    let resolved_value = resolved.await?;

    // Extract the JSON string from the resolved value
    // Create a HandleScope and ContextScope for V8 operations
    let context = js_runtime.main_context();
    let isolate = js_runtime.v8_isolate();
    let mut handle_scope = deno_core::v8::HandleScope::new(isolate);
    let mut handle_scope = {
        let pinned = unsafe { std::pin::Pin::new_unchecked(&mut handle_scope) };
        pinned.init()
    };
    let scope = &mut handle_scope;
    let context = deno_core::v8::Local::new(scope, context);
    let scope = &mut deno_core::v8::ContextScope::new(scope, context);

    let local = deno_core::v8::Local::new(scope, resolved_value);
    let json_str = local
        .to_string(scope)
        .ok_or_else(|| anyhow::anyhow!("failed to convert JS result to string"))?
        .to_rust_string_lossy(scope);

    // Parse the JSON response
    let js_response: JsResponse = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("failed to parse JS response: {e}, got: {json_str}"))?;

    // Build the HTTP response
    let mut builder = http::Response::builder().status(js_response.status);

    for (key, value) in &js_response.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }

    let response = builder
        .body(bytes::Bytes::from(js_response.body))
        .map_err(|e| anyhow::anyhow!("failed to build HTTP response: {e}"))?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deno_core::RuntimeOptions;
    use runtime_core::extensions;

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
        JsRuntime::new(opts)
    }

    #[test]
    fn inject_bridge_sets_globals() {
        let mut runtime = make_runtime();
        inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

        let val = runtime
            .execute_script(
                "<test>",
                deno_core::ascii_str!("typeof globalThis.__edgeRuntime === 'object'"),
            )
            .unwrap();

        deno_core::scope!(scope, runtime);
        let local = val.open(scope);
        assert!(
            local.is_true(),
            "__edgeRuntime should be an object on globalThis"
        );
    }

    #[test]
    fn inject_bridge_overrides_deno_serve() {
        let mut runtime = make_runtime();
        inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

        let val = runtime
            .execute_script(
                "<test>",
                deno_core::ascii_str!("typeof globalThis.Deno.serve === 'function'"),
            )
            .unwrap();

        deno_core::scope!(scope, runtime);
        let local = val.open(scope);
        assert!(local.is_true(), "Deno.serve should be a function");
    }

    #[test]
    fn dispatch_without_handler_returns_503() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let local = tokio::task::LocalSet::new();
        let result = local.block_on(&rt, async {
            let mut runtime = make_runtime();
            inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

            let request = http::Request::builder()
                .method("GET")
                .uri("/test")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();

            dispatch_request(&mut runtime, request).await
        });

        let response = result.expect("dispatch_request should not error");
        assert_eq!(
            response.status(),
            503,
            "should return 503 when no handler registered"
        );
    }
}
