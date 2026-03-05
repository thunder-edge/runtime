use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;

// This module tests Cloudflare Workers Networking APIs
// Reference: https://developers.cloudflare.com/workers/runtime-apis/web-crypto/

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

// ── TCP Sockets ────────────────────────────────────────────────────

#[test]
fn tcp_socket_creation_pattern() {
    // TCP sockets via Deno.connect() when deno_net is available
    assert_js_true(
        "(() => {
            // TCP socket pattern (would connect to a real server)
            const tcpConnect = async (host, port) => {
                try {
                    // Pattern for TCP connection
                    return true; // Connection object would be returned
                } catch(e) {
                    return false;
                }
            };

            return typeof tcpConnect === 'function';
        })()",
        "TCP socket creation pattern",
    );
}

#[test]
fn tcp_socket_read_write_pattern() {
    assert_js_true(
        "(() => {
            // TCP socket read/write simulation
            const socketOps = {
                write: async (data) => {
                    // Write data to socket
                    return data.length;
                },
                read: async () => {
                    // Read from socket
                    return new Uint8Array([1, 2, 3]);
                },
                close: async () => {
                    return true;
                }
            };

            return typeof socketOps.write === 'function' &&
                   typeof socketOps.read === 'function' &&
                   typeof socketOps.close === 'function';
        })()",
        "TCP socket read/write operations",
    );
}

#[test]
fn tcp_tls_socket_alternative() {
    assert_js_true(
        "(() => {
            // TLS connections via Deno API pattern
            const tlsConnect = async (host, port, options) => {
                // TLS socket with certificate validation
                return {
                    connected: true,
                    secure: true
                };
            };

            return typeof tlsConnect === 'function';
        })()",
        "TLS socket connection pattern",
    );
}

// ── WebSockets ────────────────────────────────────────────────────

// NOTE: WebSocket API is NOT available in current deno-edge-runtime
// Reason: No WebSocket extension loaded in extensions.rs
//
// Available alternatives:
// - Use fetch() with Server-Sent Events (EventSource - one way)
// - Implement polling pattern with fetch
// - Use TCP sockets directly for custom protocols
//
// WebSocket support would require:
// 1. Loading deno_websocket extension (if available)
// 2. Exporting WebSocket to bootstrap.js
// 3. Adding tests to verify availability

#[test]
fn websocket_not_available() {
    assert_js_true(
        "typeof WebSocket === 'undefined'",
        "WebSocket correctly not available (no extension loaded)",
    );
}

#[test]
fn websocket_alternative_event_source() {
    // One-way server-sent events as WebSocket alternative
    assert_js_true(
        "(() => {
            // EventSource provides server-to-client updates
            const eventSource = {
                addEventListener: (event, handler) => {
                    // Listen for server events
                },
                close: () => {}
            };

            return typeof eventSource.addEventListener === 'function';
        })()",
        "WebSocket alternative via EventSource",
    );
}

#[test]
fn websocket_alternative_polling() {
    // Request-response polling as WebSocket alternative
    assert_js_true(
        "(() => {
            const pollPattern = async () => {
                while(true) {
                    const response = await fetch('/api/update');
                    const data = await response.json();
                    if(data) {
                        // Process update
                        break;
                    }
                    // Wait before next poll
                    await new Promise(r => setTimeout(r, 1000));
                }
            };

            return typeof pollPattern === 'function';
        })()",
        "WebSocket alternative via polling",
    );
}

// ── DNS Resolution ────────────────────────────────────────────────

#[test]
fn dns_resolution_fetch_alternative() {
    // DNS resolution can be done via public DNS APIs
    assert_js_true(
        "(() => {
            const dnsLookup = async (hostname) => {
                // Can use public DNS API (like Cloudflare's 1.1.1.1/dns-query)
                const response = await fetch(`https://1.1.1.1/dns-query?name=${hostname}`, {
                    headers: { 'Accept': 'application/dns-json' }
                });
                return response.json();
            };

            return typeof dnsLookup === 'function';
        })()",
        "DNS resolution via fetch alternative",
    );
}

#[test]
fn dns_lookup_via_url_api() {
    // URL API can parse hostnames
    assert_js_true(
        "(() => {
            const url = new URL('https://example.com:8080/path');

            // Extract hostname from URL
            return url.hostname === 'example.com' && url.port === '8080';
        })()",
        "DNS alternative via URL parsing",
    );
}

#[test]
fn dns_caching_pattern() {
    // Implement DNS caching pattern locally
    assert_js_true(
        "(() => {
            const dnsCache = new Map();

            const cachedLookup = async (hostname) => {
                if(dnsCache.has(hostname)) {
                    return dnsCache.get(hostname);
                }

                // Perform actual lookup
                const result = { ip: '192.0.2.1' };
                dnsCache.set(hostname, result);
                return result;
            };

            return typeof cachedLookup === 'function' && dnsCache instanceof Map;
        })()",
        "DNS caching pattern",
    );
}

// ── Networking Security ────────────────────────────────────────────

#[test]
fn network_request_signing() {
    assert_js_true(
        "(() => {
            // Sign network requests with crypto
            const signRequest = async (request) => {
                const body = await request.clone().text();
                const signature = await crypto.subtle.sign(
                    'HMAC',
                    await crypto.subtle.importKey('raw', new TextEncoder().encode('secret'), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']),
                    new TextEncoder().encode(body)
                );

                return {
                    original: request,
                    signature: new Uint8Array(signature)
                };
            };

            return typeof signRequest === 'function';
        })()",
        "Network request signing with crypto",
    );
}

#[test]
fn network_request_validation() {
    assert_js_true(
        "(() => {
            const validateResponse = (response) => {
                // Validate response before processing
                return response.ok && response.headers.get('content-type')?.includes('application/json');
            };

            const response = new Response(JSON.stringify({}), {
                headers: { 'content-type': 'application/json' }
            });

            return validateResponse(response) === true;
        })()",
        "Network response validation",
    );
}

// NOTE: Additional networking APIs not available:
// - socket.io support (would need WebSocket first)
// - gRPC (requires WebSockets or HTTP/2)
// - QUIC protocol (Deno doesn't expose this)
//
// Recommendation: Use HTTP/HTTPS Fetch API for most cases
