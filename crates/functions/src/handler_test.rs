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
fn default_function_handler_accepts_any_method() {
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
                    globalThis.__edgeRuntime.registerHandlerFromModuleExports({
                      default: async (req) => new Response(`method:${req.method}`, { status: 200 }),
                    });
                    "#
                ),
            )
            .unwrap();

        let request = http::Request::builder()
            .method("PATCH")
            .uri("/any-method")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let response = dispatch_request(&mut runtime, request)
            .await
            .expect("dispatch_request should succeed");

        assert_eq!(response.parts.status, 200);
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
                while let Some(chunk) =
                    tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                        .await
                        .expect("timed out receiving any-method body")
                {
                    let chunk = chunk.expect("chunk error");
                    out.extend_from_slice(&chunk);
                }
                bytes::Bytes::from(out)
            }
        };
        assert_eq!(body, bytes::Bytes::from_static(b"method:PATCH"));
    });
}

#[test]
fn default_object_handler_returns_405_when_method_is_missing() {
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
                    globalThis.__edgeRuntime.registerHandlerFromModuleExports({
                      default: {
                        GET: async (_req) => new Response('ok-get', { status: 200 }),
                        POST: async (_req) => new Response('ok-post', { status: 200 }),
                      },
                    });
                    "#
                ),
            )
            .unwrap();

        let request = http::Request::builder()
            .method("PUT")
            .uri("/method-miss")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let response = dispatch_request(&mut runtime, request)
            .await
            .expect("dispatch_request should succeed");

        assert_eq!(response.parts.status, 405);
        assert_eq!(
            response
                .parts
                .headers
                .get(http::header::ALLOW)
                .and_then(|v| v.to_str().ok()),
            Some("GET, POST")
        );

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
                while let Some(chunk) =
                    tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                        .await
                        .expect("timed out receiving 405 body")
                {
                    let chunk = chunk.expect("chunk error");
                    out.extend_from_slice(&chunk);
                }
                bytes::Bytes::from(out)
            }
        };
        assert_eq!(
            body,
            bytes::Bytes::from_static(br#"{"error":"method_not_allowed"}"#)
        );
    });
}

#[test]
fn dispatch_auto_normalizes_response_like_objects() {
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
                    globalThis.__edgeRuntime.registerHandler(() => {
                      return {
                        statusCode: 201,
                        toResponse() {
                          return new Response(JSON.stringify({ created: true }), {
                            status: this.statusCode,
                            headers: { 'content-type': 'application/json' },
                          });
                        },
                      };
                    });
                    "#
                ),
            )
            .unwrap();

        let request = http::Request::builder()
            .method("GET")
            .uri("/normalize-draft")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let response = dispatch_request(&mut runtime, request)
            .await
            .expect("dispatch_request should succeed");

        assert_eq!(response.parts.status, 201);
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
                while let Some(chunk) =
                    tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                        .await
                        .expect("timed out receiving normalized draft body")
                {
                    let chunk = chunk.expect("chunk error");
                    out.extend_from_slice(&chunk);
                }
                bytes::Bytes::from(out)
            }
        };

        assert_eq!(body, bytes::Bytes::from_static(br#"{"created":true}"#));
    });
}

#[test]
fn dispatch_auto_normalizes_plain_object_to_json_response() {
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
                    globalThis.__edgeRuntime.registerHandler(() => {
                      return {
                        ok: true,
                        source: 'plain-object',
                      };
                    });
                    "#
                ),
            )
            .unwrap();

        let request = http::Request::builder()
            .method("GET")
            .uri("/normalize-object")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let response = dispatch_request(&mut runtime, request)
            .await
            .expect("dispatch_request should succeed");

        assert_eq!(response.parts.status, 200);
        assert_eq!(
            response
                .parts
                .headers
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json; charset=utf-8")
        );

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
                while let Some(chunk) =
                    tokio::time::timeout(std::time::Duration::from_millis(100), body_rx.recv())
                        .await
                        .expect("timed out receiving normalized object body")
                {
                    let chunk = chunk.expect("chunk error");
                    out.extend_from_slice(&chunk);
                }
                bytes::Bytes::from(out)
            }
        };

        assert_eq!(
            body,
            bytes::Bytes::from_static(br#"{"ok":true,"source":"plain-object"}"#)
        );
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
