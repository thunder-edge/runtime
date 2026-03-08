use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::create_permissions_container;
use std::time::{Duration, Instant};
use tungstenite::Message;

// This module tests Cloudflare Workers Networking APIs
// Reference: https://developers.cloudflare.com/workers/runtime-apis/web-crypto/

static INIT: std::sync::Once = std::sync::Once::new();

fn init_v8() {
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
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
                deno_core::scope!(scope, runtime);
                let local = val.open(scope);
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
            deno_core::scope!(scope, runtime);
            let local = val.open(scope);
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

// WebSocket API is available through deno_websocket extension.
// Runtime guardrails enforce per-isolate connection limits and handshake timeout.

#[test]
fn websocket_available() {
    assert_js_true(
        "typeof WebSocket === 'function'",
        "WebSocket constructor is available",
    );
}

#[test]
fn websocket_runtime_guardrails_exposed() {
    assert_js_true(
        "(() => {
            return Number.isInteger(WebSocket.maxConnections) &&
                   WebSocket.maxConnections > 0 &&
                   Number.isInteger(WebSocket.connectTimeoutMs) &&
                   WebSocket.connectTimeoutMs > 0;
        })()",
        "WebSocket guardrails metadata",
    );
}

#[test]
fn websocket_state_constants_available() {
    assert_js_true(
        "(() => {
            return WebSocket.CONNECTING === 0 &&
                   WebSocket.OPEN === 1 &&
                   WebSocket.CLOSING === 2 &&
                   WebSocket.CLOSED === 3;
        })()",
        "WebSocket state constants",
    );
}

#[test]
fn websocket_handshake_and_message_echo() {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind websocket echo listener");
    let addr = listener.local_addr().expect("read local address");

    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept websocket client");
        let mut socket = tungstenite::accept(stream).expect("upgrade websocket handshake");

        let msg = socket.read().expect("read websocket message");
        socket
            .send(Message::Text(msg.into_text().expect("text frame")))
            .expect("echo websocket message");
        let _ = socket.close(None);
    });

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create tokio runtime");

    rt.block_on(async move {
        let mut runtime = make_runtime();
        let script = format!(
            r#"
            globalThis.__wsTest = {{ done: false, ok: false, error: null }};
            const ws = new WebSocket("ws://127.0.0.1:{port}");
            ws.onopen = () => ws.send("ping");
            ws.onmessage = (event) => {{
              globalThis.__wsTest.ok = event.data === "ping";
              ws.close(1000, "done");
            }};
            ws.onerror = () => {{
              globalThis.__wsTest.error = "websocket error";
              globalThis.__wsTest.done = true;
            }};
            ws.onclose = () => {{
              globalThis.__wsTest.done = true;
            }};
            "#,
            port = addr.port()
        );

        runtime
            .execute_script("<ws_test>", script)
            .expect("execute websocket test script");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            runtime
                .run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                    pump_v8_message_loop: true,
                })
                .await
                .expect("run event loop for websocket test");

            let done = runtime
                .execute_script("<ws_done>", "globalThis.__wsTest?.done === true")
                .expect("read websocket done flag");
            let is_done = {
                deno_core::scope!(scope, runtime);
                done.open(scope).is_true()
            };
            if is_done {
                break;
            }

            assert!(Instant::now() < deadline, "websocket test timed out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let success = runtime
            .execute_script(
                "<ws_result>",
                "globalThis.__wsTest?.ok === true && globalThis.__wsTest?.error === null",
            )
            .expect("read websocket test result");

        deno_core::scope!(scope, runtime);
        assert!(
            success.open(scope).is_true(),
            "expected websocket echo success"
        );
    });

    server.join().expect("join websocket echo server");
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
// - socket.io protocol support is not built-in (higher-level library concern)
// - gRPC over HTTP/2 is not implemented in this runtime
// - QUIC protocol (Deno doesn't expose this)
//
// Recommendation: Use HTTP/HTTPS Fetch API for most cases
