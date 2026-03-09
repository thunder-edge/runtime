use anyhow::Error;
use base64::Engine;
use deno_core::{op2, Extension, JsRuntime, OpState};
use runtime_core::isolate::{
    IsolateConfig, IsolateResponse, IsolateResponseBody, OutgoingProxyConfig,
};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::connection_manager::{global_connection_manager, AcquireError};

#[derive(Default)]
struct ResponseStreamRegistry {
    streams: std::collections::HashMap<String, mpsc::UnboundedSender<Result<bytes::Bytes, Error>>>,
}

#[op2(fast)]
fn op_edge_stream_chunk(
    state: &mut OpState,
    #[string] stream_id: String,
    #[buffer] chunk: &[u8],
) -> Result<(), deno_error::JsErrorBox> {
    let registry = state.borrow_mut::<ResponseStreamRegistry>();
    if let Some(sender) = registry.streams.get(&stream_id) {
        let _ = sender.send(Ok(bytes::Bytes::copy_from_slice(chunk)));
    }
    Ok(())
}

#[op2(fast)]
fn op_edge_stream_end(
    state: &mut OpState,
    #[string] stream_id: String,
) -> Result<(), deno_error::JsErrorBox> {
    let registry = state.borrow_mut::<ResponseStreamRegistry>();
    registry.streams.remove(&stream_id);
    Ok(())
}

#[op2(fast)]
fn op_edge_stream_error(
    state: &mut OpState,
    #[string] stream_id: String,
    #[string] message: String,
) -> Result<(), deno_error::JsErrorBox> {
    let registry = state.borrow_mut::<ResponseStreamRegistry>();
    if let Some(sender) = registry.streams.remove(&stream_id) {
        let _ = sender.send(Err(anyhow::anyhow!(message)));
    }
    Ok(())
}

#[op2(async(lazy), fast)]
#[string]
async fn op_edge_acquire_egress_lease(
    #[string] tenant: String,
    #[string] execution_id: String,
    timeout_ms: u32,
) -> Result<String, deno_error::JsErrorBox> {
    let timeout = if timeout_ms == 0 {
        None
    } else {
        Some(Duration::from_millis(timeout_ms as u64))
    };

    let lease_id = global_connection_manager()
        .acquire_lease(tenant, execution_id, timeout)
        .await
        .map_err(|err| {
            let reason = match err {
                AcquireError::Backpressure => "backpressure".to_string(),
                AcquireError::Timeout => "timeout".to_string(),
                AcquireError::Internal(message) => format!("internal error: {message}"),
            };
            deno_error::JsErrorBox::generic(format!(
                "[thunder] outbound lease acquisition failed: {reason}"
            ))
        })?;

    Ok(lease_id.to_string())
}

#[op2(fast)]
fn op_edge_release_egress_lease(
    #[string] lease_id: String,
) -> Result<bool, deno_error::JsErrorBox> {
    let parsed_id = lease_id.parse::<u64>().map_err(|e| {
        deno_error::JsErrorBox::generic(format!(
            "[thunder] invalid egress lease id '{lease_id}': {e}"
        ))
    })?;
    Ok(global_connection_manager().release_lease(parsed_id))
}

#[op2(fast)]
fn op_edge_release_execution_egress_leases(
    #[string] execution_id: String,
) -> Result<u32, deno_error::JsErrorBox> {
    let released = global_connection_manager().release_execution_leases(&execution_id);
    Ok(released as u32)
}

deno_core::extension!(
    edge_response_stream,
    ops = [
        op_edge_stream_chunk,
        op_edge_stream_end,
        op_edge_stream_error,
        op_edge_acquire_egress_lease,
        op_edge_release_egress_lease,
        op_edge_release_execution_egress_leases
    ],
);

pub fn response_stream_extension() -> Extension {
    edge_response_stream::init()
}

pub fn ensure_response_stream_registry(js_runtime: &mut JsRuntime) {
    let op_state = js_runtime.op_state();
    let mut state = op_state.borrow_mut();
    state.put(ResponseStreamRegistry::default());
}

fn register_response_stream(
    js_runtime: &mut JsRuntime,
    stream_id: String,
    sender: mpsc::UnboundedSender<Result<bytes::Bytes, Error>>,
) {
    let op_state = js_runtime.op_state();
    let mut state = op_state.borrow_mut();
    let registry = state.borrow_mut::<ResponseStreamRegistry>();
    registry.streams.insert(stream_id, sender);
}

fn unregister_response_stream(js_runtime: &mut JsRuntime, stream_id: &str) {
    let op_state = js_runtime.op_state();
    let mut state = op_state.borrow_mut();
    let registry = state.borrow_mut::<ResponseStreamRegistry>();
    registry.streams.remove(stream_id);
}

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
    inject_request_bridge_with_proxy_and_config(
        js_runtime,
        &OutgoingProxyConfig::default(),
        &IsolateConfig::default(),
    )
}

pub fn inject_request_bridge_with_proxy(
    js_runtime: &mut JsRuntime,
    outgoing_proxy: &OutgoingProxyConfig,
) -> Result<(), Error> {
    inject_request_bridge_with_proxy_and_config(
        js_runtime,
        outgoing_proxy,
        &IsolateConfig::default(),
    )
}

pub fn inject_request_bridge_with_proxy_and_config(
    js_runtime: &mut JsRuntime,
    outgoing_proxy: &OutgoingProxyConfig,
    isolate_config: &IsolateConfig,
) -> Result<(), Error> {
    let proxy_json = serde_json::to_string(outgoing_proxy)
        .map_err(|e| anyhow::anyhow!("failed to serialize outgoing proxy config: {e}"))?;
    let set_proxy_config = format!("globalThis.__edgeRuntimeProxyConfig = {proxy_json};");
    js_runtime.execute_script("edge-internal:///runtime_proxy_config.js", set_proxy_config)?;

    let set_vfs_config = format!(
        "globalThis.__edgeRuntimeVfsConfig = {{ totalQuotaBytes: {}, maxFileBytes: {} }};",
        isolate_config.vfs_total_quota_bytes, isolate_config.vfs_max_file_bytes
    );
    js_runtime.execute_script("edge-internal:///runtime_vfs_config.js", set_vfs_config)?;

    let dns_doh_endpoint_json = serde_json::to_string(&isolate_config.dns_doh_endpoint)
        .map_err(|e| anyhow::anyhow!("failed to serialize dns resolver endpoint: {e}"))?;
    let set_dns_config = format!(
        "globalThis.__edgeRuntimeDnsConfig = {{ dohEndpoint: {dns_doh_endpoint_json}, maxAnswers: {}, timeoutMs: {} }};",
        isolate_config.dns_max_answers,
        isolate_config.dns_timeout_ms,
    );
    js_runtime.execute_script("edge-internal:///runtime_dns_config.js", set_dns_config)?;

    let set_zlib_config = format!(
        "globalThis.__edgeRuntimeZlibConfig = {{ maxOutputLength: {}, maxInputLength: {}, operationTimeoutMs: {} }};",
        isolate_config.zlib_max_output_length,
        isolate_config.zlib_max_input_length,
        isolate_config.zlib_operation_timeout_ms,
    );
    js_runtime.execute_script("edge-internal:///runtime_zlib_config.js", set_zlib_config)?;

    let set_egress_config = format!(
        "globalThis.__edgeRuntimeEgressConfig = {{ maxRequestsPerExecution: {} }};",
        isolate_config.egress_max_requests_per_execution,
    );
    js_runtime.execute_script(
        "edge-internal:///runtime_egress_config.js",
        set_egress_config,
    )?;

    js_runtime.execute_script(
        "edge-internal:///runtime_bridge.js",
        deno_core::ascii_str!(
            r#"
            const __existingEdgeRuntime = globalThis.__edgeRuntime;
            const __existingPrimaryHandler = __existingEdgeRuntime?.handler ?? null;
            const __existingHandlers =
                __existingEdgeRuntime?._handlers instanceof Map
                    ? __existingEdgeRuntime._handlers
                    : new Map();

            globalThis.__edgeRuntime = {
                handler: __existingPrimaryHandler,
                _handlers: __existingHandlers,
                registerHandler(fn) {
                    this.handler = fn;
                    const bootstrapContextId =
                        globalThis.__edgeRuntimeCurrentBootstrapContextId || 'default';
                    this._handlers.set(bootstrapContextId, fn);
                },
                registerHandlerForContext(contextId, fn) {
                    if (!contextId || typeof contextId !== 'string') {
                        throw new Error('[thunder] invalid context id for registerHandlerForContext');
                    }
                    this._handlers.set(contextId, fn);
                    if (!this.handler) {
                        this.handler = fn;
                    }
                },
                resolveHandler(contextId) {
                    if (contextId && this._handlers.has(contextId)) {
                        return this._handlers.get(contextId);
                    }
                    if (this.handler) {
                        return this.handler;
                    }
                    if (this._handlers.has('default')) {
                        return this._handlers.get('default');
                    }
                    return null;
                },

                // Execution context tracking for resource cleanup
                _currentExecutionId: null,
                _timerRegistry: new Map(),       // executionId -> Set<timerId>
                _intervalRegistry: new Map(),    // executionId -> Set<intervalId>
                _abortRegistry: new Map(),       // executionId -> Set<AbortController>
                _promiseRegistry: new Map(),     // executionId -> Set<{promise, reject}>
                _wsRegistry: new Map(),          // executionId -> Set<WebSocket>
                _egressRegistry: new Map(),      // executionId -> number
                _egressLeaseRegistry: new Map(), // executionId -> Set<leaseId>
                _executionState: new Map(),      // executionId -> { active: boolean, token: number }
                _nextExecutionToken: 1,
                _lastBlockedNetworkLog: null,
                _currentTenant: 'default',
                _egressConfig: globalThis.__edgeRuntimeEgressConfig || {
                    maxRequestsPerExecution: 0,
                },
                _proxyConfig: globalThis.__edgeRuntimeProxyConfig || {
                    httpProxy: null,
                    httpsProxy: null,
                    tcpProxy: null,
                    httpNoProxy: [],
                    httpsNoProxy: [],
                    tcpNoProxy: [],
                },

                _clearAsyncHooksExecutionContext(executionId) {
                    try {
                        globalThis.__edgeRuntimeAsyncHooks?.clearExecutionContext?.(executionId);
                    } catch (_) {
                        // Keep request lifecycle resilient if async_hooks bridge throws.
                    }
                },

                startExecution(executionId) {
                    this._currentExecutionId = executionId;
                    this._timerRegistry.set(executionId, new Set());
                    this._intervalRegistry.set(executionId, new Set());
                    this._abortRegistry.set(executionId, new Set());
                    this._promiseRegistry.set(executionId, new Set());
                    this._wsRegistry.set(executionId, new Set());
                    this._egressRegistry.set(executionId, 0);
                    this._egressLeaseRegistry.set(executionId, new Set());
                    this._executionState.set(executionId, {
                        active: true,
                        token: this._nextExecutionToken++,
                    });
                    this._clearAsyncHooksExecutionContext(executionId);
                },

                _trackEgressLease(executionId, leaseId) {
                    if (!executionId || leaseId === null || leaseId === undefined) return;
                    let leases = this._egressLeaseRegistry.get(executionId);
                    if (!leases) {
                        leases = new Set();
                        this._egressLeaseRegistry.set(executionId, leases);
                    }
                    leases.add(String(leaseId));
                },

                _untrackEgressLease(executionId, leaseId) {
                    if (!executionId || leaseId === null || leaseId === undefined) return;
                    const leases = this._egressLeaseRegistry.get(executionId);
                    if (!leases) return;
                    leases.delete(String(leaseId));
                    if (leases.size === 0) {
                        this._egressLeaseRegistry.delete(executionId);
                    }
                },

                _releaseExecutionEgressLeases(executionId) {
                    if (!executionId) return 0;
                    let released = 0;
                    try {
                        released = Number(Deno.core.ops.op_edge_release_execution_egress_leases(executionId) || 0);
                    } catch (_) {
                        // Avoid surfacing cleanup failures to user handlers.
                    }
                    this._egressLeaseRegistry.delete(executionId);
                    return released;
                },
                
                consumeEgressToken(kind, target) {
                    const executionId = this._currentExecutionId;
                    if (!executionId) {
                        return;
                    }

                    const maxRequests = Number(this._egressConfig?.maxRequestsPerExecution || 0);
                    if (!Number.isFinite(maxRequests) || maxRequests <= 0) {
                        return;
                    }

                    const current = Number(this._egressRegistry.get(executionId) || 0);
                    const next = current + 1;
                    this._egressRegistry.set(executionId, next);

                    if (next > maxRequests) {
                        const apiKind = kind || 'network';
                        const apiTarget = target || '<unknown>';
                        throw new Error(
                            `[thunder] egress rate limit exceeded for execution '${executionId}' (${next}/${maxRequests}) kind='${apiKind}' target='${apiTarget}'`,
                        );
                    }
                },

                _captureExecutionSnapshot(executionId) {
                    if (!executionId) return null;
                    const state = this._executionState.get(executionId);
                    if (!state || !state.active) return null;
                    return { executionId, token: state.token };
                },

                _isExecutionSnapshotActive(snapshot) {
                    if (!snapshot || !snapshot.executionId) return false;
                    const state = this._executionState.get(snapshot.executionId);
                    return Boolean(state && state.active && state.token === snapshot.token);
                },

                _deactivateExecution(executionId) {
                    if (!executionId) return;
                    const state = this._executionState.get(executionId);
                    if (state) {
                        state.active = false;
                    }
                },

                registerWebSocketForCurrentExecution(socket) {
                    const executionId = this._currentExecutionId;
                    if (!executionId || !socket) return null;

                    let sockets = this._wsRegistry.get(executionId);
                    if (!sockets) {
                        sockets = new Set();
                        this._wsRegistry.set(executionId, sockets);
                    }
                    sockets.add(socket);
                    return executionId;
                },

                unregisterWebSocket(executionId, socket) {
                    if (!executionId || !socket) return;
                    const sockets = this._wsRegistry.get(executionId);
                    if (!sockets) return;
                    sockets.delete(socket);
                    if (sockets.size === 0) {
                        this._wsRegistry.delete(executionId);
                    }
                },

                _closeExecutionWebSockets(executionId, closeCode, closeReason) {
                    const sockets = this._wsRegistry.get(executionId);
                    if (!sockets) return;

                    for (const socket of Array.from(sockets)) {
                        try {
                            if (socket && (socket.readyState === 0 || socket.readyState === 1)) {
                                socket.close(closeCode, closeReason);
                            }
                        } catch (_) {
                            // Ignore close races.
                        }
                    }

                    this._wsRegistry.delete(executionId);
                },

                endExecution(executionId) {
                    this._deactivateExecution(executionId);
                    this._closeExecutionWebSockets(executionId, 1001, 'Execution ended');
                    this._releaseExecutionEgressLeases(executionId);
                    this._timerRegistry.delete(executionId);
                    this._intervalRegistry.delete(executionId);
                    this._abortRegistry.delete(executionId);
                    this._promiseRegistry.delete(executionId);
                    this._wsRegistry.delete(executionId);
                    this._egressRegistry.delete(executionId);
                    this._egressLeaseRegistry.delete(executionId);
                    if (this._currentExecutionId === executionId) {
                        this._currentExecutionId = null;
                    }
                    this._executionState.delete(executionId);
                    this._clearAsyncHooksExecutionContext(executionId);
                },

                clearExecutionTimers(executionId) {
                    this._deactivateExecution(executionId);
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

                    // Close tracked WebSocket connections for this execution
                    this._closeExecutionWebSockets(executionId, 1013, 'Request cancelled due to execution timeout');
                    this._releaseExecutionEgressLeases(executionId);

                    // Cleanup registries
                    this._timerRegistry.delete(executionId);
                    this._intervalRegistry.delete(executionId);
                    this._abortRegistry.delete(executionId);
                    this._promiseRegistry.delete(executionId);
                    this._wsRegistry.delete(executionId);
                    this._egressRegistry.delete(executionId);
                    this._egressLeaseRegistry.delete(executionId);
                    if (this._currentExecutionId === executionId) {
                        this._currentExecutionId = null;
                    }
                    this._executionState.delete(executionId);
                    this._clearAsyncHooksExecutionContext(executionId);
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

                _isBlockedNetworkError(error) {
                    const message = String(error?.message || error || '').toLowerCase();
                    return (
                        message.includes('requires net access') ||
                        (message.includes('permission') && message.includes('net')) ||
                        message.includes('blocked by permission')
                    );
                },

                _requestTarget(input) {
                    try {
                        if (typeof input === 'string') return input;
                        if (input instanceof URL) return input.toString();
                        if (input && typeof input.url === 'string') return input.url;
                        return String(input);
                    } catch (_) {
                        return '<unknown>';
                    }
                },

                _logBlockedNetworkRequest(input, error) {
                    const target = this._requestTarget(input);
                    const message = String(error?.message || error || 'unknown error');
                    this._lastBlockedNetworkLog = { target, message };
                    console.warn(`[thunder] blocked outbound request target='${target}' reason='${message}'`);
                },

                _listContainsHost(list, host) {
                    if (!Array.isArray(list) || list.length === 0) return false;
                    const normalizedHost = String(host || '').toLowerCase();
                    for (const rawEntry of list) {
                        const entry = String(rawEntry || '').trim().toLowerCase();
                        if (!entry) continue;
                        if (entry === '*') return true;
                        if (entry.startsWith('.')) {
                            const suffix = entry.slice(1);
                            if (normalizedHost === suffix || normalizedHost.endsWith(entry)) {
                                return true;
                            }
                        } else if (normalizedHost === entry || normalizedHost.endsWith(`.${entry}`)) {
                            return true;
                        }
                    }
                    return false;
                },

                _selectProxy(urlObj) {
                    if (!(urlObj instanceof URL)) return null;
                    const scheme = urlObj.protocol === 'https:' ? 'https' : (urlObj.protocol === 'http:' ? 'http' : null);
                    if (!scheme) return null;

                    const cfg = this._proxyConfig || {};
                    let proxyKind = null;
                    let proxyValue = null;
                    let noProxyList = [];

                    if (scheme === 'http' && cfg.httpProxy) {
                        proxyKind = 'http';
                        proxyValue = cfg.httpProxy;
                        noProxyList = cfg.httpNoProxy || [];
                    } else if (scheme === 'https' && cfg.httpsProxy) {
                        proxyKind = 'http';
                        proxyValue = cfg.httpsProxy;
                        noProxyList = cfg.httpsNoProxy || [];
                    } else if (cfg.tcpProxy) {
                        proxyKind = 'tcp';
                        proxyValue = cfg.tcpProxy;
                        noProxyList = cfg.tcpNoProxy || [];
                    }

                    if (!proxyKind || !proxyValue) {
                        return null;
                    }

                    if (this._listContainsHost(noProxyList, urlObj.hostname)) {
                        return null;
                    }

                    return { proxyKind, proxyValue };
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
                const execId = globalThis.__edgeRuntime._currentExecutionId;
                const executionSnapshot = globalThis.__edgeRuntime._captureExecutionSnapshot(execId);
                const timerId = globalThis.__originalSetTimeout(function() {
                    // Remove from registry when timer fires
                    if (executionSnapshot?.executionId) {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get(executionSnapshot.executionId);
                        if (timers) timers.delete(timerId);
                    }

                    // Skip execution if request lifecycle has been finalized.
                    if (!globalThis.__edgeRuntime._isExecutionSnapshotActive(executionSnapshot)) {
                        return;
                    }

                    // Call original callback
                    if (typeof fn === 'function') {
                        fn(...args);
                    } else {
                        // Handle string argument (eval-style, rare)
                        eval(fn);
                    }
                }, delay);

                if (execId) {
                    const timers = globalThis.__edgeRuntime._timerRegistry.get(execId);
                    if (timers) timers.add(timerId);
                }
                return timerId;
            };

            // Wrap setInterval to track by execution id
            globalThis.setInterval = function(fn, interval, ...args) {
                const execId = globalThis.__edgeRuntime._currentExecutionId;
                const executionSnapshot = globalThis.__edgeRuntime._captureExecutionSnapshot(execId);
                const intervalId = globalThis.__originalSetInterval(function(...invokeArgs) {
                    if (!globalThis.__edgeRuntime._isExecutionSnapshotActive(executionSnapshot)) {
                        globalThis.__originalClearInterval(intervalId);
                        if (executionSnapshot?.executionId) {
                            const intervals = globalThis.__edgeRuntime._intervalRegistry.get(executionSnapshot.executionId);
                            if (intervals) intervals.delete(intervalId);
                        }
                        return;
                    }

                    if (typeof fn === 'function') {
                        fn(...invokeArgs);
                    } else {
                        eval(fn);
                    }
                }, interval, ...args);

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

            // Wrap fetch to enforce centralized egress lease limits and track aborts.
            globalThis.fetch = async function(input, init = {}) {
                const execId = globalThis.__edgeRuntime._currentExecutionId;
                const tenant = String(globalThis.__edgeRuntime._currentTenant || 'default');

                const invokeFetch = async (requestInput, requestInit) => {
                    globalThis.__edgeRuntime.consumeEgressToken('fetch', globalThis.__edgeRuntime._requestTarget(requestInput));

                    let leaseId = null;
                    try {
                        leaseId = await Deno.core.ops.op_edge_acquire_egress_lease(
                            tenant,
                            String(execId || 'no-execution'),
                            75,
                        );
                        if (execId) {
                            globalThis.__edgeRuntime._trackEgressLease(execId, leaseId);
                        }
                    } catch (error) {
                        const target = globalThis.__edgeRuntime._requestTarget(requestInput);
                        const message = String(error?.message || error || 'capacity unavailable');
                        throw new Error(
                            `[thunder] outbound connection capacity exhausted target='${target}' tenant='${tenant}' reason='${message}'`,
                        );
                    }

                    let proxySelection = null;
                    let selectedUrl = null;
                    try {
                        const rawUrl = (typeof requestInput === 'string' || requestInput instanceof URL)
                            ? requestInput
                            : requestInput?.url;
                        if (rawUrl) {
                            selectedUrl = new URL(String(rawUrl));
                            proxySelection = globalThis.__edgeRuntime._selectProxy(selectedUrl);
                        }
                    } catch (_) {
                        // If URL parsing fails, fallback to original fetch path.
                    }

                    try {
                        return await globalThis.__originalFetch(requestInput, requestInit);
                    } catch (error) {
                        if (globalThis.__edgeRuntime._isBlockedNetworkError(error)) {
                            globalThis.__edgeRuntime._logBlockedNetworkRequest(requestInput, error);
                        }
                        if (proxySelection) {
                            const reason = String(error?.message || error || 'unknown proxy error');
                            const target = selectedUrl ? selectedUrl.toString() : globalThis.__edgeRuntime._requestTarget(requestInput);
                            throw new Error(`[thunder] outgoing proxy request failed kind='${proxySelection.proxyKind}' target='${target}' reason='${reason}'`);
                        }
                        throw error;
                    } finally {
                        if (leaseId !== null && leaseId !== undefined) {
                            try {
                                Deno.core.ops.op_edge_release_egress_lease(leaseId);
                            } catch (_) {
                                // Best-effort release. Reaper handles orphaned leases.
                            }
                            if (execId) {
                                globalThis.__edgeRuntime._untrackEgressLease(execId, leaseId);
                            }
                        }
                    }
                };

                // If no execution context, just call original fetch
                if (!execId) {
                    return await invokeFetch(input, init);
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
                    return await invokeFetch(input, init);
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

                const fetchPromise = invokeFetch(input, { ...init, signal });

                // Untrack controller when fetch completes (success or error)
                fetchPromise.finally(() => {
                    globalThis.__edgeRuntime._untrackAbortController(controller);
                }).catch(() => {});

                return await fetchPromise;
            };

            // Wrap queueMicrotask to track execution context
            // Note: Microtasks cannot be cancelled, but we track them for visibility
            globalThis.queueMicrotask = function(callback) {
                const execId = globalThis.__edgeRuntime._currentExecutionId;
                const executionSnapshot = globalThis.__edgeRuntime._captureExecutionSnapshot(execId);

                return globalThis.__originalQueueMicrotask(function() {
                    // We can't cancel queued microtasks, but we can skip callback execution
                    // when their originating request has already been finalized.
                    if (!globalThis.__edgeRuntime._isExecutionSnapshotActive(executionSnapshot)) {
                        return;
                    }

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
            globalThis.__edgeRuntime.handleRequest = async function(method, url, headersJson, body, streamId, contextId, functionName) {
                const handler = globalThis.__edgeRuntime.resolveHandler(contextId);
                if (!handler) {
                    return JSON.stringify({
                        status: 503,
                        headers: [["content-type", "application/json"]],
                        body_kind: 'inline',
                        body_base64: btoa(`{"error":"no handler registered for context '${contextId || 'default'}' function '${functionName || '<unknown>'}'"}`),
                    });
                }

                const previousTenant = globalThis.__edgeRuntime._currentTenant;
                globalThis.__edgeRuntime._currentTenant = String(
                    functionName || contextId || previousTenant || 'default',
                );

                try {
                    const parsedHeaders = JSON.parse(headersJson || '[]');
                    const requestHeaders = new Headers();
                    if (Array.isArray(parsedHeaders)) {
                        for (const pair of parsedHeaders) {
                            if (Array.isArray(pair) && pair.length >= 2) {
                                requestHeaders.append(String(pair[0]), String(pair[1]));
                            }
                        }
                    } else if (parsedHeaders && typeof parsedHeaders === 'object') {
                        for (const [key, value] of Object.entries(parsedHeaders)) {
                            requestHeaders.append(String(key), String(value));
                        }
                    }

                    const reqInit = {
                        method: method,
                        headers: requestHeaders,
                    };
                    if (body && body.length > 0 && method !== 'GET' && method !== 'HEAD') {
                        reqInit.body = body;
                    }
                    const request = new Request(url, reqInit);
                    const executeHandler = () => handler(request);
                    const response = globalThis.__edgeRuntimeAsyncHooks?.runWithExecutionContext
                        ? await globalThis.__edgeRuntimeAsyncHooks.runWithExecutionContext(
                            globalThis.__edgeRuntime._currentExecutionId || '',
                            executeHandler,
                        )
                        : await executeHandler();

                    let respHeaders = [];
                    response.headers.forEach((value, key) => {
                        respHeaders.push([key, value]);
                    });

                    if (typeof response.headers.getSetCookie === 'function') {
                        const withoutSetCookie = respHeaders.filter(([key]) => key.toLowerCase() !== 'set-cookie');
                        for (const cookieValue of response.headers.getSetCookie()) {
                            withoutSetCookie.push(['set-cookie', cookieValue]);
                        }
                        respHeaders = withoutSetCookie;
                    }

                    const hasBody =
                        response.body && method !== 'HEAD' && response.status !== 204 && response.status !== 304;

                    if (hasBody) {
                        const reader = response.body.getReader();
                        (async () => {
                            try {
                                while (true) {
                                    const { done, value } = await reader.read();
                                    if (done) {
                                        Deno.core.ops.op_edge_stream_end(streamId);
                                        break;
                                    }
                                    Deno.core.ops.op_edge_stream_chunk(streamId, value);
                                }
                            } catch (streamErr) {
                                Deno.core.ops.op_edge_stream_error(streamId, String(streamErr));
                            }
                        })();

                        return JSON.stringify({
                            status: response.status,
                            headers: respHeaders,
                            body_kind: 'stream',
                            stream_id: streamId,
                        });
                    }

                    return JSON.stringify({
                        status: response.status,
                        headers: respHeaders,
                        body_kind: 'inline',
                        body_base64: '',
                    });
                } catch (err) {
                    return JSON.stringify({
                        status: 500,
                        headers: [["content-type", "application/json"]],
                        body_kind: 'inline',
                        body_base64: btoa(JSON.stringify({ error: String(err) })),
                    });
                } finally {
                    globalThis.__edgeRuntime._currentTenant = previousTenant || 'default';
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
    headers: Vec<(String, String)>,
    body_kind: String,
    #[serde(default)]
    body_base64: String,
    #[serde(default)]
    stream_id: Option<String>,
}

/// Dispatch an HTTP request into the JS fetch handler and return the response.
pub async fn dispatch_request(
    js_runtime: &mut JsRuntime,
    request: http::Request<bytes::Bytes>,
) -> Result<IsolateResponse, Error> {
    dispatch_request_for_context(js_runtime, request, None, None).await
}

pub async fn dispatch_request_for_context(
    js_runtime: &mut JsRuntime,
    request: http::Request<bytes::Bytes>,
    context_id: Option<&str>,
    function_name: Option<&str>,
) -> Result<IsolateResponse, Error> {
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

    // Serialize headers to JSON preserving duplicate header semantics.
    let headers_list: Vec<(String, String)> = request
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let headers_json = serde_json::to_string(&headers_list)?;
    let context_id = context_id.unwrap_or("default");
    let function_name = function_name.unwrap_or("<unknown>");

    let body = request.into_body();
    let stream_id = Uuid::new_v4().to_string();
    let (chunk_tx, chunk_rx) = mpsc::unbounded_channel::<Result<bytes::Bytes, Error>>();
    register_response_stream(js_runtime, stream_id.clone(), chunk_tx);

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
        let stream_id_v8 = deno_core::v8::String::new(scope, &stream_id)
            .ok_or_else(|| anyhow::anyhow!("failed to allocate stream id string"))?;
        let context_id_v8 = deno_core::v8::String::new(scope, context_id)
            .ok_or_else(|| anyhow::anyhow!("failed to allocate context id string"))?;
        let function_name_v8 = deno_core::v8::String::new(scope, function_name)
            .ok_or_else(|| anyhow::anyhow!("failed to allocate function name string"))?;

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

        let args: [deno_core::v8::Local<deno_core::v8::Value>; 7] = [
            method_v8.into(),
            uri_v8.into(),
            headers_v8.into(),
            body_arg,
            stream_id_v8.into(),
            context_id_v8.into(),
            function_name_v8.into(),
        ];

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

    // Extract the JSON string from the resolved value.
    let json_str = {
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
        local
            .to_string(scope)
            .ok_or_else(|| anyhow::anyhow!("failed to convert JS result to string"))?
            .to_rust_string_lossy(scope)
    };

    // Parse the JSON response
    let js_response: JsResponse = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("failed to parse JS response: {e}, got: {json_str}"))?;

    // Build the HTTP response
    let mut builder = http::Response::builder().status(js_response.status);

    for (key, value) in &js_response.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }

    let response_parts = builder
        .body(())
        .map_err(|e| anyhow::anyhow!("failed to build HTTP response: {e}"))?
        .into_parts()
        .0;

    match js_response.body_kind.as_str() {
        "stream" => {
            let returned_stream_id = js_response
                .stream_id
                .ok_or_else(|| anyhow::anyhow!("missing stream_id for streaming response"))?;
            if returned_stream_id != stream_id {
                unregister_response_stream(js_runtime, &stream_id);
                return Err(anyhow::anyhow!(
                    "stream id mismatch: expected {stream_id}, got {returned_stream_id}"
                ));
            }

            Ok(IsolateResponse {
                parts: response_parts,
                body: IsolateResponseBody::Stream(chunk_rx),
            })
        }
        "inline" => {
            unregister_response_stream(js_runtime, &stream_id);
            let decoded = if js_response.body_base64.is_empty() {
                bytes::Bytes::new()
            } else {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(js_response.body_base64)
                    .map_err(|e| anyhow::anyhow!("invalid base64 response body: {e}"))?;
                bytes::Bytes::from(raw)
            };

            Ok(IsolateResponse {
                parts: response_parts,
                body: IsolateResponseBody::Full(decoded),
            })
        }
        other => {
            unregister_response_stream(js_runtime, &stream_id);
            Err(anyhow::anyhow!("unknown JS response body kind: {other}"))
        }
    }
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
        let mut runtime_extensions = extensions::get_extensions();
        runtime_extensions.push(response_stream_extension());
        let mut opts = RuntimeOptions {
            extensions: runtime_extensions,
            ..Default::default()
        };
        extensions::set_extension_transpiler(&mut opts);
        let mut runtime = JsRuntime::new(opts);
        ensure_response_stream_registry(&mut runtime);
        runtime
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
            response.parts.status, 503,
            "should return 503 when no handler registered"
        );
    }

    #[test]
    fn blocked_network_error_detection_matches_permission_errors() {
        let mut runtime = make_runtime();
        inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

        let val = runtime
            .execute_script(
                "<test>",
                deno_core::ascii_str!(
                    "globalThis.__edgeRuntime._isBlockedNetworkError(new Error('Requires net access to \\\"169.254.169.254\\\"'))"
                ),
            )
            .unwrap();

        deno_core::scope!(scope, runtime);
        let local = val.open(scope);
        assert!(local.is_true());
    }

    #[test]
    fn fetch_wrapper_logs_blocked_requests() {
        let mut runtime = make_runtime();
        inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

        runtime
            .execute_script(
                "<test>",
                deno_core::ascii_str!(
                    r#"
                    const err = new Error('Requires net access to "169.254.169.254"');
                    globalThis.__edgeRuntime._logBlockedNetworkRequest(
                        'http://169.254.169.254/latest/meta-data',
                        err,
                    );
                    "#
                ),
            )
            .unwrap();

        let val = runtime
            .execute_script(
                "<test>",
                deno_core::ascii_str!(
                    "globalThis.__edgeRuntime._lastBlockedNetworkLog?.target === 'http://169.254.169.254/latest/meta-data'"
                ),
            )
            .unwrap();

        deno_core::scope!(scope, runtime);
        let local = val.open(scope);
        assert!(local.is_true(), "expected warning log for blocked request");
    }

    #[test]
    fn dispatch_stream_response_returns_chunks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let mut runtime = make_runtime();
            inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

            runtime
                .execute_script(
                    "<test>",
                    deno_core::ascii_str!(
                        r#"
                        Deno.serve((_req) => {
                          const enc = new TextEncoder();
                          return new Response(
                            new ReadableStream({
                              start(controller) {
                                controller.enqueue(enc.encode("a"));
                                controller.enqueue(enc.encode("b"));
                                controller.close();
                              },
                            }),
                            { headers: { "content-type": "text/plain" } },
                          );
                        });
                        "#
                    ),
                )
                .unwrap();

            let request = http::Request::builder()
                .method("GET")
                .uri("/stream")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();

            let response = dispatch_request(&mut runtime, request)
                .await
                .expect("dispatch_request should succeed");

            let mut body_rx = match response.body {
                IsolateResponseBody::Stream(rx) => rx,
                IsolateResponseBody::Full(_) => panic!("expected stream body"),
            };

            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                runtime.run_event_loop(deno_core::PollEventLoopOptions {
                    wait_for_inspector: false,
                    pump_v8_message_loop: true,
                }),
            )
            .await;

            let mut out = Vec::new();
            while let Some(chunk) =
                tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                    .await
                    .expect("timed out receiving stream chunk")
            {
                let chunk = chunk.expect("chunk error");
                out.extend_from_slice(&chunk);
            }

            assert_eq!(out, b"ab");
        });
    }

    #[test]
    fn dispatch_for_context_uses_registered_context_handler() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let mut runtime = make_runtime();
            inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

            runtime
                .execute_script(
                    "<test>",
                    deno_core::ascii_str!(
                        r#"
                        globalThis.__edgeRuntime.registerHandlerForContext('ctx-a', () => {
                          return new Response('handler-a', { status: 200 });
                        });
                        globalThis.__edgeRuntime.registerHandlerForContext('ctx-b', () => {
                          return new Response('handler-b', { status: 200 });
                        });
                        "#
                    ),
                )
                .unwrap();

            let request_a = http::Request::builder()
                .method("GET")
                .uri("/ctx-a")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();
            let response_a = dispatch_request_for_context(
                &mut runtime,
                request_a,
                Some("ctx-a"),
                Some("ctx-function"),
            )
            .await
            .expect("dispatch_request_for_context should succeed for ctx-a");

            let request_b = http::Request::builder()
                .method("GET")
                .uri("/ctx-b")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();
            let response_b = dispatch_request_for_context(
                &mut runtime,
                request_b,
                Some("ctx-b"),
                Some("ctx-function"),
            )
            .await
            .expect("dispatch_request_for_context should succeed for ctx-b");

            let body_a = match response_a.body {
                IsolateResponseBody::Full(body) => body,
                IsolateResponseBody::Stream(mut body_rx) => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        runtime.run_event_loop(deno_core::PollEventLoopOptions {
                            wait_for_inspector: false,
                            pump_v8_message_loop: true,
                        }),
                    )
                    .await;

                    let mut out = Vec::new();
                    while let Some(chunk) =
                        tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                            .await
                            .expect("timed out receiving stream body for ctx-a")
                    {
                        let chunk = chunk.expect("chunk error");
                        out.extend_from_slice(&chunk);
                    }
                    bytes::Bytes::from(out)
                }
            };
            let body_b = match response_b.body {
                IsolateResponseBody::Full(body) => body,
                IsolateResponseBody::Stream(mut body_rx) => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        runtime.run_event_loop(deno_core::PollEventLoopOptions {
                            wait_for_inspector: false,
                            pump_v8_message_loop: true,
                        }),
                    )
                    .await;

                    let mut out = Vec::new();
                    while let Some(chunk) =
                        tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                            .await
                            .expect("timed out receiving stream body for ctx-b")
                    {
                        let chunk = chunk.expect("chunk error");
                        out.extend_from_slice(&chunk);
                    }
                    bytes::Bytes::from(out)
                }
            };

            assert_eq!(body_a, bytes::Bytes::from_static(b"handler-a"));
            assert_eq!(body_b, bytes::Bytes::from_static(b"handler-b"));
        });
    }

    #[test]
    fn dispatch_preserves_multiple_set_cookie_headers() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let mut runtime = make_runtime();
            inject_request_bridge(&mut runtime).expect("inject_request_bridge failed");

            runtime
                .execute_script(
                    "<test>",
                    deno_core::ascii_str!(
                        r#"
                        Deno.serve((_req) => {
                          const headers = new Headers();
                          headers.append('set-cookie', 'a=1; Path=/; HttpOnly');
                          headers.append('set-cookie', 'b=2; Path=/; Secure');
                          headers.append('x-custom', 'one');
                          headers.append('x-custom', 'two');
                          return new Response('ok', { status: 200, headers });
                        });
                        "#
                    ),
                )
                .unwrap();

            let request = http::Request::builder()
                .method("GET")
                .uri("/cookies")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();

            let response = dispatch_request(&mut runtime, request)
                .await
                .expect("dispatch_request should succeed");

            let set_cookie_values: Vec<String> = response
                .parts
                .headers
                .get_all(http::header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok().map(str::to_string))
                .collect();
            assert_eq!(set_cookie_values.len(), 2, "expected two set-cookie values");
            assert!(set_cookie_values
                .iter()
                .any(|v| v.contains("a=1") && v.contains("HttpOnly")));
            assert!(set_cookie_values
                .iter()
                .any(|v| v.contains("b=2") && v.contains("Secure")));

            let x_custom_values: Vec<String> = response
                .parts
                .headers
                .get_all("x-custom")
                .iter()
                .filter_map(|v| v.to_str().ok().map(str::to_string))
                .collect();
            // Non Set-Cookie list headers are merged by Fetch Headers semantics.
            assert_eq!(x_custom_values, vec!["one, two".to_string()]);
        });
    }

    #[test]
    fn dispatch_enforces_egress_rate_limit_per_execution() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let mut runtime = make_runtime();
            let isolate_config = IsolateConfig {
                egress_max_requests_per_execution: 1,
                ..IsolateConfig::default()
            };
            inject_request_bridge_with_proxy_and_config(
                &mut runtime,
                &OutgoingProxyConfig::default(),
                &isolate_config,
            )
            .expect("inject_request_bridge_with_proxy_and_config failed");

            runtime
                .execute_script(
                    "<test>",
                    deno_core::ascii_str!(
                        r#"
                        globalThis.__edgeMockFetchHandler = async () => new Response('ok', { status: 200 });

                        Deno.serve(async (_req) => {
                          try {
                            await fetch('https://example.com/one');
                            await fetch('https://example.com/two');
                            return new Response('unexpected-success', { status: 200 });
                          } catch (err) {
                            return new Response(String(err?.message || err), { status: 500 });
                          }
                        });

                                                globalThis.__edgeRuntime.startExecution('test-exec');
                        "#
                    ),
                )
                .unwrap();

            let request = http::Request::builder()
                .method("GET")
                .uri("/egress")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();

            let response = dispatch_request(&mut runtime, request)
                .await
                .expect("dispatch_request should succeed");

            assert_eq!(response.parts.status, 500);

            let body = match response.body {
                IsolateResponseBody::Full(body) => body,
                IsolateResponseBody::Stream(mut body_rx) => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        runtime.run_event_loop(deno_core::PollEventLoopOptions {
                            wait_for_inspector: false,
                            pump_v8_message_loop: true,
                        }),
                    )
                    .await;

                    let mut out = Vec::new();
                    while let Some(chunk) = tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        body_rx.recv(),
                    )
                    .await
                    .expect("timed out receiving egress error body")
                    {
                        let chunk = chunk.expect("chunk error");
                        out.extend_from_slice(&chunk);
                    }
                    bytes::Bytes::from(out)
                }
            };

            let body_text = String::from_utf8(body.to_vec()).expect("response body should be utf8");
            assert!(
                body_text.contains("[thunder] egress rate limit exceeded"),
                "unexpected body: {body_text}"
            );
        });
    }
}
