//! Integration tests for request timeout and timer tracking functionality.
//!
//! Tests cover:
//! - V8 terminate_execution on timeout
//! - Timer tracking by execution ID
//! - Timer cleanup on timeout
//! - Isolate reuse after timeout

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use functions::registry::FunctionRegistry;
use functions::types::{BundlePackage, FunctionStatus};
use runtime_core::extensions;
use runtime_core::isolate::IsolateConfig;
use runtime_core::module_loader::EszipModuleLoader;
use tokio_util::sync::CancellationToken;

/// Helper: create a JsRuntime with the same config as production isolates.
fn make_runtime_with_eszip(eszip: Arc<eszip::EszipV2>) -> JsRuntime {
    let module_loader = Rc::new(EszipModuleLoader::new(eszip));

    let mut opts = RuntimeOptions {
        module_loader: Some(module_loader),
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);

    JsRuntime::new(opts)
}

/// Helper: build an eszip from inline JS source.
fn build_eszip(specifier: &str, source: &str) -> Vec<u8> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(build_eszip_async(specifier, source))
}

async fn build_eszip_async(specifier: &str, source: &str) -> Vec<u8> {
    use deno_ast::{EmitOptions, TranspileOptions};
    use deno_graph::ast::CapturingModuleAnalyzer;
    use deno_graph::source::{LoadOptions, LoadResponse, Loader};
    use deno_graph::{BuildOptions, GraphKind, ModuleGraph};

    struct InlineLoader {
        specifier: String,
        source: String,
    }

    impl Loader for InlineLoader {
        fn load(
            &self,
            specifier: &deno_graph::ModuleSpecifier,
            _options: LoadOptions,
        ) -> deno_graph::source::LoadFuture {
            let spec = specifier.clone();
            let expected = self.specifier.clone();
            let source = self.source.clone();
            Box::pin(async move {
                if spec.as_str() == expected {
                    Ok(Some(LoadResponse::Module {
                        content: source.into_bytes().into(),
                        specifier: spec,
                        maybe_headers: None,
                        mtime: None,
                    }))
                } else {
                    Ok(None)
                }
            })
        }
    }

    let loader = InlineLoader {
        specifier: specifier.to_string(),
        source: source.to_string(),
    };
    let analyzer = CapturingModuleAnalyzer::default();
    let root = deno_graph::ModuleSpecifier::parse(specifier).unwrap();

    let mut graph = ModuleGraph::new(GraphKind::CodeOnly);
    graph
        .build(
            vec![root],
            vec![],
            &loader,
            BuildOptions {
                module_analyzer: &analyzer,
                ..Default::default()
            },
        )
        .await;

    let eszip = eszip::EszipV2::from_graph(eszip::FromGraphOptions {
        graph,
        parser: analyzer.as_capturing_parser(),
        module_kind_resolver: Default::default(),
        transpile_options: TranspileOptions::default(),
        emit_options: EmitOptions::default(),
        relative_file_base: None,
        npm_packages: None,
        npm_snapshot: Default::default(),
    })
    .expect("from_graph failed");

    eszip.into_bytes()
}

/// Parse eszip bytes into an EszipV2.
async fn parse_eszip(bytes: &[u8]) -> eszip::EszipV2 {
    let reader = futures_util::io::BufReader::new(futures_util::io::Cursor::new(bytes.to_vec()));
    let (eszip, loader_fut) = eszip::EszipV2::parse(reader).await.unwrap();
    tokio::spawn(loader_fut);
    eszip
}

// ─── Tests ────────────────────────────────────────────────────────────

/// Test 1: V8 terminate_execution stops infinite loops.
#[test]
fn test_terminate_execution_stops_infinite_loop() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_infinite.js",
        r#"
        Deno.serve(async (req) => {
            // Infinite loop that should be terminated
            while(true) {}
            return new Response("never");
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let root = runtime_core::isolate::determine_root_specifier(&eszip)
            .map_err(|e| format!("determine_root_specifier: {e}"))?;

        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        let module_id = js_runtime
            .load_main_es_module(&root)
            .await
            .map_err(|e| format!("load_main_es_module: {e}"))?;

        let eval = js_runtime.mod_evaluate(module_id);

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop: {e}"))?;

        eval.await.map_err(|e| format!("mod_evaluate: {e}"))?;

        // Get thread-safe handle for termination
        let v8_handle = js_runtime.v8_isolate().thread_safe_handle();
        let terminated = Arc::new(AtomicBool::new(false));
        let watchdog_terminated = terminated.clone();

        // Spawn watchdog thread that terminates after 100ms
        let watchdog = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if v8_handle.terminate_execution() {
                watchdog_terminated.store(true, Ordering::SeqCst);
            }
        });

        // Dispatch request (will be terminated by watchdog)
        let request = http::Request::builder()
            .method("GET")
            .uri("/test")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let result = functions::handler::dispatch_request(&mut js_runtime, request).await;

        watchdog.join().unwrap();

        // Verify termination occurred
        assert!(
            terminated.load(Ordering::SeqCst),
            "watchdog should have terminated execution"
        );

        // Verify dispatch returned an error (due to termination)
        assert!(result.is_err(), "dispatch should fail after termination");

        // Reset termination state
        js_runtime.v8_isolate().cancel_terminate_execution();

        // Verify isolate can be reused - execute simple script
        let check_result =
            js_runtime.execute_script("<reuse_check>", deno_core::ascii_str!("1 + 1"));
        assert!(
            check_result.is_ok(),
            "isolate should be reusable after cancel_terminate_execution"
        );

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 1.1 roadmap requirement: isolate timeout returns HTTP 504.
#[test]
fn test_isolate_timeout_returns_504() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_timeout_504.js",
        r#"
        Deno.serve(async (_req) => {
            while (true) {}
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result: Result<(), String> = rt.block_on(async {
        let bundle = functions::types::BundlePackage::eszip_only(eszip_bytes);
        let bundle_data =
            bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

        let mut config = IsolateConfig::default();
        config.wall_clock_timeout_ms = 100;

        let entry = functions::lifecycle::create_function(
            "timeout-504-test".to_string(),
            bundle_data,
            config,
            CancellationToken::new(),
        )
        .await
        .map_err(|e| format!("create_function: {e}"))?;

        let handle = entry
            .isolate_handle
            .clone()
            .ok_or_else(|| "missing isolate handle".to_string())?;

        let request = http::Request::builder()
            .method("GET")
            .uri("/timeout")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .map_err(|e| format!("build request: {e}"))?;

        let response = handle
            .send_request(request)
            .await
            .map_err(|e| format!("send_request: {e}"))?;

        let body_text = String::from_utf8_lossy(response.body()).to_string();
        if response.status() != 504 {
            return Err(format!(
                "expected 504, got {} body={}",
                response.status(),
                body_text
            ));
        }

        if !body_text.contains("request timeout") {
            return Err(format!("expected timeout body, got: {}", body_text));
        }

        functions::lifecycle::destroy_function(&entry).await;
        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test roadmap 1.2 requirement: infinite allocation should terminate isolate
/// and mark the function as Error in registry.
#[test]
fn test_heap_limit_infinite_allocation_marks_function_error() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_heap_oom.js",
        r#"
        Deno.serve(async (_req) => {
            let s = "";
            while (true) {
                s += "Hello";
            }
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result: Result<(), String> = rt.block_on(async {
        let bundle = BundlePackage::eszip_only(eszip_bytes);
        let bundle_data =
            bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

        let mut config = IsolateConfig::default();
        config.max_heap_size_bytes = 32 * 1024 * 1024;
        config.wall_clock_timeout_ms = 0;

        let registry = FunctionRegistry::new(CancellationToken::new(), IsolateConfig::default());
        let _ = registry
            .deploy(
                "heap-limit-test".to_string(),
                bytes::Bytes::from(bundle_data),
                Some(config),
            )
            .await
            .map_err(|e| format!("deploy: {e}"))?;

        let handle = registry
            .get_handle("heap-limit-test")
            .ok_or_else(|| "missing handle after deploy".to_string())?;

        let request = http::Request::builder()
            .method("GET")
            .uri("/heap")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .map_err(|e| format!("build request: {e}"))?;

        let send_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle.send_request(request),
        )
        .await;

        match send_result {
            Ok(Ok(resp)) => {
                // Depending on timing, JS may throw OOM first (500) before isolate exits.
                // The key assertion for this roadmap item is the registry Error transition.
                if resp.status() != 500 && resp.status() != 504 {
                    return Err(format!(
                        "unexpected response status for heap-limit path: {}",
                        resp.status()
                    ));
                }
            }
            Ok(Err(_)) => {
                // Also valid: isolate exited before sending response.
            }
            Err(_) => {
                return Err("timed out waiting for heap-limit request outcome".to_string());
            }
        }

        // Poll until registry reconciles dead isolate to Error.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(info) = registry.get_info("heap-limit-test") {
                if info.status == FunctionStatus::Error {
                    break;
                }
            }
            if std::time::Instant::now() > deadline {
                return Err(
                    "function was not marked Error after heap-limit termination".to_string()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        registry
            .delete("heap-limit-test")
            .await
            .map_err(|e| format!("delete function: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test roadmap 1.3 requirement: panic followed by request should fail fast
/// and mark function status as Error in the registry.
#[test]
fn test_panic_followed_by_request_marks_error_and_fails_fast() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_panic_recovery.js",
        r#"
        Deno.serve(async (_req) => {
            return new Response("ok");
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result: Result<(), String> = rt.block_on(async {
        std::env::set_var("EDGE_RUNTIME_TEST_PANIC_ON_PATH", "/panic-once");

        let bundle = BundlePackage::eszip_only(eszip_bytes);
        let bundle_data =
            bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

        let registry = FunctionRegistry::new(CancellationToken::new(), IsolateConfig::default());
        let _ = registry
            .deploy(
                "panic-recovery-test".to_string(),
                bytes::Bytes::from(bundle_data),
                None,
            )
            .await
            .map_err(|e| format!("deploy: {e}"))?;

        let handle_before = registry
            .get_handle("panic-recovery-test")
            .ok_or_else(|| "missing handle after deploy".to_string())?;

        let panic_req = http::Request::builder()
            .method("GET")
            .uri("/panic-once")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .map_err(|e| format!("build panic request: {e}"))?;

        // First request should fail because isolate panics and channel closes.
        let first = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle_before.send_request(panic_req),
        )
        .await
        .map_err(|_| "timed out waiting panic request result".to_string())?;
        if first.is_ok() {
            return Err("expected panic request to fail".to_string());
        }

        // Ensure only one injected panic occurs.
        std::env::remove_var("EDGE_RUNTIME_TEST_PANIC_ON_PATH");

        // Registry should expose Error state after panic.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let info_during = registry
            .get_info("panic-recovery-test")
            .ok_or_else(|| "missing function info during recovery".to_string())?;
        if info_during.status != FunctionStatus::Error {
            return Err(format!(
                "expected Error status during panic recovery, got {:?}",
                info_during.status
            ));
        }

        let follow_req = http::Request::builder()
            .method("GET")
            .uri("/ok")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .map_err(|e| format!("build follow-up request: {e}"))?;

        // With request channel closed on panic, a follow-up request must fail fast.
        let follow = handle_before.send_request(follow_req).await;
        if follow.is_ok() {
            return Err("expected follow-up request to fail fast after panic".to_string());
        }

        registry
            .delete("panic-recovery-test")
            .await
            .map_err(|e| format!("delete function: {e}"))?;
        Ok(())
    });

    std::env::remove_var("EDGE_RUNTIME_TEST_PANIC_ON_PATH");
    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test roadmap 2.2 requirement: graceful shutdown with request in-flight.
#[test]
fn test_graceful_shutdown_with_in_flight_request() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_shutdown_inflight.js",
        r#"
        Deno.serve(async (_req) => {
            await new Promise((resolve) => setTimeout(resolve, 400));
            return new Response("done");
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result: Result<(), String> = rt.block_on(async {
        let bundle = BundlePackage::eszip_only(eszip_bytes);
        let bundle_data =
            bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

        let registry = Arc::new(FunctionRegistry::new(
            CancellationToken::new(),
            IsolateConfig::default(),
        ));

        let _ = registry
            .deploy(
                "shutdown-inflight-test".to_string(),
                bytes::Bytes::from(bundle_data),
                None,
            )
            .await
            .map_err(|e| format!("deploy: {e}"))?;

        let handle = registry
            .get_handle("shutdown-inflight-test")
            .ok_or_else(|| "missing handle after deploy".to_string())?;

        let request = http::Request::builder()
            .method("GET")
            .uri("/inflight")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .map_err(|e| format!("build request: {e}"))?;

        let send_task = tokio::spawn(async move { handle.send_request(request).await });

        // Ensure request starts before shutdown signal.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let shutdown_started = std::time::Instant::now();
        registry
            .shutdown_all_with_deadline(std::time::Duration::from_secs(2))
            .await;
        let shutdown_elapsed = shutdown_started.elapsed();
        if shutdown_elapsed > std::time::Duration::from_secs(3) {
            return Err(format!(
                "graceful shutdown exceeded expected upper bound: {:?}",
                shutdown_elapsed
            ));
        }

        // Request may complete or fail during shutdown; both are acceptable.
        let request_result = tokio::time::timeout(std::time::Duration::from_secs(2), send_task)
            .await
            .map_err(|_| "timed out waiting in-flight request task".to_string())
            .and_then(|join| join.map_err(|e| format!("join error: {e}")))?;

        if let Ok(resp) = request_result {
            if resp.status() != 200 && resp.status() != 504 {
                return Err(format!(
                    "unexpected in-flight response status during shutdown: {}",
                    resp.status()
                ));
            }
        }

        if registry.count() != 0 {
            return Err(format!(
                "expected registry to be empty after shutdown, got {} entries",
                registry.count()
            ));
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 2: Timer tracking registers timers by execution ID.
#[test]
fn test_timer_tracking_registration() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_timer_reg.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        // Inject bridge with timer tracking
        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-exec-1");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Create a timer - should be tracked
        js_runtime
            .execute_script(
                "<create_timer>",
                deno_core::ascii_str!(r#"globalThis.__testTimerId = setTimeout(() => {}, 10000);"#),
            )
            .map_err(|e| format!("setTimeout: {e}"))?;

        // Verify timer was registered in the registry
        let check = js_runtime
            .execute_script(
                "<check_registry>",
                deno_core::ascii_str!(
                    r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-exec-1");
                        return timers && timers.has(globalThis.__testTimerId);
                    })();
                "#
                ),
            )
            .map_err(|e| format!("check registry: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check.open(scope);
            assert!(
                local_val.is_true(),
                "timer should be registered in the registry"
            );
        }

        // Clear timers for execution
        js_runtime
            .execute_script(
                "<clear_timers>",
                deno_core::ascii_str!(
                    r#"globalThis.__edgeRuntime.clearExecutionTimers("test-exec-1");"#
                ),
            )
            .map_err(|e| format!("clearExecutionTimers: {e}"))?;

        // Verify timer was removed from registry
        let check_cleared = js_runtime
            .execute_script(
                "<check_cleared>",
                deno_core::ascii_str!(
                    r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-exec-1");
                        return timers === undefined;
                    })();
                "#
                ),
            )
            .map_err(|e| format!("check cleared: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_cleared.open(scope);
            assert!(local_val.is_true(), "timer registry should be cleared");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 3: Interval tracking registers intervals by execution ID.
#[test]
fn test_interval_tracking_registration() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_interval_reg.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-exec-2");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Create an interval - should be tracked
        js_runtime
            .execute_script(
                "<create_interval>",
                deno_core::ascii_str!(r#"globalThis.__testIntervalId = setInterval(() => {}, 10000);"#),
            )
            .map_err(|e| format!("setInterval: {e}"))?;

        // Verify interval was registered
        let check = js_runtime
            .execute_script(
                "<check_registry>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const intervals = globalThis.__edgeRuntime._intervalRegistry.get("test-exec-2");
                        return intervals && intervals.has(globalThis.__testIntervalId);
                    })();
                "#),
            )
            .map_err(|e| format!("check registry: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check.open(scope);
            assert!(local_val.is_true(), "interval should be registered in the registry");
        }

        // Clear execution timers (includes intervals)
        js_runtime
            .execute_script(
                "<clear_timers>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.clearExecutionTimers("test-exec-2");"#),
            )
            .map_err(|e| format!("clearExecutionTimers: {e}"))?;

        // Verify interval was removed
        let check_cleared = js_runtime
            .execute_script(
                "<check_cleared>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const intervals = globalThis.__edgeRuntime._intervalRegistry.get("test-exec-2");
                        return intervals === undefined;
                    })();
                "#),
            )
            .map_err(|e| format!("check cleared: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_cleared.open(scope);
            assert!(local_val.is_true(), "interval registry should be cleared");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 4: Timer isolation - timers from different executions don't interfere.
#[test]
fn test_timer_isolation_between_executions() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_isolation.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start first execution and create timer
        js_runtime
            .execute_script(
                "<exec1_start>",
                deno_core::ascii_str!(
                    r#"
                    globalThis.__edgeRuntime.startExecution("exec-A");
                    globalThis.__timerA = setTimeout(() => {}, 10000);
                "#
                ),
            )
            .map_err(|e| format!("exec1: {e}"))?;

        // End first execution (simulates normal completion)
        js_runtime
            .execute_script(
                "<exec1_end>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.endExecution("exec-A");"#),
            )
            .map_err(|e| format!("exec1 end: {e}"))?;

        // Start second execution and create timer
        js_runtime
            .execute_script(
                "<exec2_start>",
                deno_core::ascii_str!(
                    r#"
                    globalThis.__edgeRuntime.startExecution("exec-B");
                    globalThis.__timerB = setTimeout(() => {}, 10000);
                "#
                ),
            )
            .map_err(|e| format!("exec2: {e}"))?;

        // Clear timers for exec-B only
        js_runtime
            .execute_script(
                "<clear_exec2>",
                deno_core::ascii_str!(
                    r#"globalThis.__edgeRuntime.clearExecutionTimers("exec-B");"#
                ),
            )
            .map_err(|e| format!("clear exec2: {e}"))?;

        // Verify exec-A's timer registry was cleaned up on endExecution
        let check_a = js_runtime
            .execute_script(
                "<check_a>",
                deno_core::ascii_str!(
                    r#"
                    globalThis.__edgeRuntime._timerRegistry.get("exec-A") === undefined;
                "#
                ),
            )
            .map_err(|e| format!("check A: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_a.open(scope);
            assert!(
                local_val.is_true(),
                "exec-A registry should be cleared after endExecution"
            );
        }

        // Verify exec-B's timer registry was cleared
        let check_b = js_runtime
            .execute_script(
                "<check_b>",
                deno_core::ascii_str!(
                    r#"
                    globalThis.__edgeRuntime._timerRegistry.get("exec-B") === undefined;
                "#
                ),
            )
            .map_err(|e| format!("check B: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_b.open(scope);
            assert!(
                local_val.is_true(),
                "exec-B registry should be cleared after clearExecutionTimers"
            );
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 5: Isolate remains functional after timeout + cleanup.
#[test]
fn test_isolate_reusable_after_timeout() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_reuse.js",
        r#"
        Deno.serve(async (req) => {
            const url = new URL(req.url);
            if (url.pathname === "/hang") {
                while(true) {} // Will be terminated
            }
            return new Response("ok");
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let root = runtime_core::isolate::determine_root_specifier(&eszip)
            .map_err(|e| format!("determine_root_specifier: {e}"))?;

        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        let module_id = js_runtime
            .load_main_es_module(&root)
            .await
            .map_err(|e| format!("load_main_es_module: {e}"))?;

        let eval = js_runtime.mod_evaluate(module_id);

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop: {e}"))?;

        eval.await.map_err(|e| format!("mod_evaluate: {e}"))?;

        // === First request: will timeout ===
        let v8_handle = js_runtime.v8_isolate().thread_safe_handle();
        let terminated = Arc::new(AtomicBool::new(false));
        let watchdog_terminated = terminated.clone();

        // Start execution tracking
        js_runtime
            .execute_script(
                "<start_exec1>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("req-1");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        let watchdog = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if v8_handle.terminate_execution() {
                watchdog_terminated.store(true, Ordering::SeqCst);
            }
        });

        let request1 = http::Request::builder()
            .method("GET")
            .uri("/hang")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let _result1 = functions::handler::dispatch_request(&mut js_runtime, request1).await;
        watchdog.join().unwrap();

        assert!(
            terminated.load(Ordering::SeqCst),
            "first request should have been terminated"
        );

        // Reset and cleanup
        js_runtime.v8_isolate().cancel_terminate_execution();
        js_runtime
            .execute_script(
                "<clear_exec1>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.clearExecutionTimers("req-1");"#),
            )
            .map_err(|e| format!("clearExecutionTimers: {e}"))?;

        // === Second request: should work normally ===
        js_runtime
            .execute_script(
                "<start_exec2>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("req-2");"#),
            )
            .map_err(|e| format!("startExecution 2: {e}"))?;

        let request2 = http::Request::builder()
            .method("GET")
            .uri("/normal")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let result2 = functions::handler::dispatch_request(&mut js_runtime, request2)
            .await
            .map_err(|e| format!("dispatch_request 2: {e}"))?;

        js_runtime
            .execute_script(
                "<end_exec2>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.endExecution("req-2");"#),
            )
            .map_err(|e| format!("endExecution 2: {e}"))?;

        let body = String::from_utf8_lossy(result2.body()).to_string();
        assert_eq!(
            body, "ok",
            "second request should succeed after first timeout"
        );
        assert_eq!(result2.status(), 200, "second request should return 200");

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 6: Fetch tracking with AbortController.
#[test]
fn test_fetch_abort_controller_tracking() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_fetch_track.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Verify abort registry exists
        let check_registry = js_runtime
            .execute_script(
                "<check_abort_registry>",
                deno_core::ascii_str!(r#"
                    globalThis.__edgeRuntime._abortRegistry instanceof Map;
                "#),
            )
            .map_err(|e| format!("check abort registry: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_registry.open(scope);
            assert!(local_val.is_true(), "_abortRegistry should be a Map");
        }

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-fetch-1");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Verify abort registry was created for this execution
        let check_exec_registry = js_runtime
            .execute_script(
                "<check_exec_registry>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const controllers = globalThis.__edgeRuntime._abortRegistry.get("test-fetch-1");
                        return controllers instanceof Set && controllers.size === 0;
                    })();
                "#),
            )
            .map_err(|e| format!("check exec registry: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_exec_registry.open(scope);
            assert!(local_val.is_true(), "abort registry should be created for execution");
        }

        // Clear execution and verify registry is removed
        js_runtime
            .execute_script(
                "<clear_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.clearExecutionTimers("test-fetch-1");"#),
            )
            .map_err(|e| format!("clearExecutionTimers: {e}"))?;

        let check_cleared = js_runtime
            .execute_script(
                "<check_cleared>",
                deno_core::ascii_str!(r#"
                    globalThis.__edgeRuntime._abortRegistry.get("test-fetch-1") === undefined;
                "#),
            )
            .map_err(|e| format!("check cleared: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_cleared.open(scope);
            assert!(local_val.is_true(), "abort registry should be cleared");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 7: Promise tracking registration.
#[test]
fn test_promise_tracking_registration() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_promise_track.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Verify promise registry exists
        let check_registry = js_runtime
            .execute_script(
                "<check_promise_registry>",
                deno_core::ascii_str!(r#"
                    globalThis.__edgeRuntime._promiseRegistry instanceof Map;
                "#),
            )
            .map_err(|e| format!("check promise registry: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_registry.open(scope);
            assert!(local_val.is_true(), "_promiseRegistry should be a Map");
        }

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-promise-1");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Track a promise manually
        js_runtime
            .execute_script(
                "<track_promise>",
                deno_core::ascii_str!(r#"
                    let rejectFn;
                    const p = new Promise((resolve, reject) => {
                        rejectFn = reject;
                        globalThis.__testReject = reject;
                    });
                    globalThis.__edgeRuntime._trackPromise(p, rejectFn);
                    globalThis.__testPromise = p;
                "#),
            )
            .map_err(|e| format!("track promise: {e}"))?;

        // Verify promise was tracked
        let check_tracked = js_runtime
            .execute_script(
                "<check_tracked>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const promises = globalThis.__edgeRuntime._promiseRegistry.get("test-promise-1");
                        return promises instanceof Set && promises.size === 1;
                    })();
                "#),
            )
            .map_err(|e| format!("check tracked: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_tracked.open(scope);
            assert!(local_val.is_true(), "promise should be tracked in registry");
        }

        // Clear execution - should reject tracked promises
        js_runtime
            .execute_script(
                "<clear_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.clearExecutionTimers("test-promise-1");"#),
            )
            .map_err(|e| format!("clearExecutionTimers: {e}"))?;

        // Verify registry is cleared
        let check_cleared = js_runtime
            .execute_script(
                "<check_cleared>",
                deno_core::ascii_str!(r#"
                    globalThis.__edgeRuntime._promiseRegistry.get("test-promise-1") === undefined;
                "#),
            )
            .map_err(|e| format!("check cleared: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_cleared.open(scope);
            assert!(local_val.is_true(), "promise registry should be cleared");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 8: Original functions are preserved.
#[test]
fn test_original_functions_preserved() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_originals.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Verify original functions exist
        let check_originals = js_runtime
            .execute_script(
                "<check_originals>",
                deno_core::ascii_str!(
                    r#"
                    typeof globalThis.__originalSetTimeout === 'function' &&
                    typeof globalThis.__originalSetInterval === 'function' &&
                    typeof globalThis.__originalClearTimeout === 'function' &&
                    typeof globalThis.__originalClearInterval === 'function' &&
                    typeof globalThis.__originalFetch === 'function' &&
                    typeof globalThis.__originalQueueMicrotask === 'function';
                "#
                ),
            )
            .map_err(|e| format!("check originals: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_originals.open(scope);
            assert!(
                local_val.is_true(),
                "all original functions should be preserved"
            );
        }

        // Verify wrapped functions exist
        let check_wrapped = js_runtime
            .execute_script(
                "<check_wrapped>",
                deno_core::ascii_str!(
                    r#"
                    typeof globalThis.setTimeout === 'function' &&
                    typeof globalThis.setInterval === 'function' &&
                    typeof globalThis.clearTimeout === 'function' &&
                    typeof globalThis.clearInterval === 'function' &&
                    typeof globalThis.fetch === 'function' &&
                    typeof globalThis.queueMicrotask === 'function';
                "#
                ),
            )
            .map_err(|e| format!("check wrapped: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_wrapped.open(scope);
            assert!(local_val.is_true(), "all wrapped functions should exist");
        }

        // Verify wrapped functions are different from originals
        let check_different = js_runtime
            .execute_script(
                "<check_different>",
                deno_core::ascii_str!(
                    r#"
                    globalThis.setTimeout !== globalThis.__originalSetTimeout &&
                    globalThis.setInterval !== globalThis.__originalSetInterval &&
                    globalThis.fetch !== globalThis.__originalFetch;
                "#
                ),
            )
            .map_err(|e| format!("check different: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_different.open(scope);
            assert!(
                local_val.is_true(),
                "wrapped functions should be different from originals"
            );
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 9: clearTimeout removes timer from registry.
#[test]
fn test_clear_timeout_removes_from_registry() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_clear_timeout.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(
                    r#"globalThis.__edgeRuntime.startExecution("test-clear-1");"#
                ),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Create a timer and then clear it
        js_runtime
            .execute_script(
                "<create_and_clear>",
                deno_core::ascii_str!(
                    r#"
                    globalThis.__testTimerId = setTimeout(() => {}, 10000);
                "#
                ),
            )
            .map_err(|e| format!("setTimeout: {e}"))?;

        // Verify timer is in registry
        let check_before = js_runtime
            .execute_script(
                "<check_before>",
                deno_core::ascii_str!(
                    r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-clear-1");
                        return timers && timers.has(globalThis.__testTimerId);
                    })();
                "#
                ),
            )
            .map_err(|e| format!("check before: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_before.open(scope);
            assert!(
                local_val.is_true(),
                "timer should be in registry before clearTimeout"
            );
        }

        // Clear the timer
        js_runtime
            .execute_script(
                "<clear_timer>",
                deno_core::ascii_str!(r#"clearTimeout(globalThis.__testTimerId);"#),
            )
            .map_err(|e| format!("clearTimeout: {e}"))?;

        // Verify timer was removed from registry
        let check_after = js_runtime
            .execute_script(
                "<check_after>",
                deno_core::ascii_str!(
                    r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-clear-1");
                        return timers && !timers.has(globalThis.__testTimerId);
                    })();
                "#
                ),
            )
            .map_err(|e| format!("check after: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_after.open(scope);
            assert!(
                local_val.is_true(),
                "timer should be removed from registry after clearTimeout"
            );
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 10: clearInterval removes interval from registry.
#[test]
fn test_clear_interval_removes_from_registry() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_clear_interval.js",
        "globalThis.__test = true;",
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-clear-2");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Create an interval
        js_runtime
            .execute_script(
                "<create_interval>",
                deno_core::ascii_str!(r#"
                    globalThis.__testIntervalId = setInterval(() => {}, 10000);
                "#),
            )
            .map_err(|e| format!("setInterval: {e}"))?;

        // Verify interval is in registry
        let check_before = js_runtime
            .execute_script(
                "<check_before>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const intervals = globalThis.__edgeRuntime._intervalRegistry.get("test-clear-2");
                        return intervals && intervals.has(globalThis.__testIntervalId);
                    })();
                "#),
            )
            .map_err(|e| format!("check before: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_before.open(scope);
            assert!(local_val.is_true(), "interval should be in registry before clearInterval");
        }

        // Clear the interval
        js_runtime
            .execute_script(
                "<clear_interval>",
                deno_core::ascii_str!(r#"clearInterval(globalThis.__testIntervalId);"#),
            )
            .map_err(|e| format!("clearInterval: {e}"))?;

        // Verify interval was removed from registry
        let check_after = js_runtime
            .execute_script(
                "<check_after>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const intervals = globalThis.__edgeRuntime._intervalRegistry.get("test-clear-2");
                        return intervals && !intervals.has(globalThis.__testIntervalId);
                    })();
                "#),
            )
            .map_err(|e| format!("check after: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_after.open(scope);
            assert!(local_val.is_true(), "interval should be removed from registry after clearInterval");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 11: Multiple sequential requests after timeout recovery.
#[test]
fn test_multiple_requests_after_timeout() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_multi_req.js",
        r#"
        let counter = 0;
        Deno.serve(async (req) => {
            const url = new URL(req.url);
            if (url.pathname === "/hang") {
                while(true) {}
            }
            counter++;
            return new Response("count:" + counter);
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let root = runtime_core::isolate::determine_root_specifier(&eszip)
            .map_err(|e| format!("determine_root_specifier: {e}"))?;

        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        let module_id = js_runtime
            .load_main_es_module(&root)
            .await
            .map_err(|e| format!("load_main_es_module: {e}"))?;

        let eval = js_runtime.mod_evaluate(module_id);

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop: {e}"))?;

        eval.await.map_err(|e| format!("mod_evaluate: {e}"))?;

        // === Request 1: timeout ===
        let v8_handle = js_runtime.v8_isolate().thread_safe_handle();
        let terminated = Arc::new(AtomicBool::new(false));
        let watchdog_terminated = terminated.clone();

        js_runtime
            .execute_script(
                "<start_exec1>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("req-1");"#),
            )
            .map_err(|e| format!("startExecution 1: {e}"))?;

        let watchdog = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if v8_handle.terminate_execution() {
                watchdog_terminated.store(true, Ordering::SeqCst);
            }
        });

        let request1 = http::Request::builder()
            .method("GET")
            .uri("/hang")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let _ = functions::handler::dispatch_request(&mut js_runtime, request1).await;
        watchdog.join().unwrap();

        assert!(
            terminated.load(Ordering::SeqCst),
            "first request should have been terminated"
        );

        js_runtime.v8_isolate().cancel_terminate_execution();
        js_runtime
            .execute_script(
                "<clear_exec1>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.clearExecutionTimers("req-1");"#),
            )
            .map_err(|e| format!("clearExecutionTimers 1: {e}"))?;

        // === Request 2, 3, 4: should all work ===
        for i in 2..=4 {
            let exec_id = format!("req-{}", i);
            js_runtime
                .execute_script(
                    "<start_exec>",
                    deno_core::FastString::from(format!(
                        r#"globalThis.__edgeRuntime.startExecution("{}");"#,
                        exec_id
                    )),
                )
                .map_err(|e| format!("startExecution {}: {e}", i))?;

            let request = http::Request::builder()
                .method("GET")
                .uri("/normal")
                .header("host", "localhost:9000")
                .body(bytes::Bytes::new())
                .unwrap();

            let result = functions::handler::dispatch_request(&mut js_runtime, request)
                .await
                .map_err(|e| format!("dispatch_request {}: {e}", i))?;

            js_runtime
                .execute_script(
                    "<end_exec>",
                    deno_core::FastString::from(format!(
                        r#"globalThis.__edgeRuntime.endExecution("{}");"#,
                        exec_id
                    )),
                )
                .map_err(|e| format!("endExecution {}: {e}", i))?;

            let body = String::from_utf8_lossy(result.body()).to_string();
            let expected = format!("count:{}", i - 1);
            assert_eq!(body, expected, "request {} should return {}", i, expected);
            assert_eq!(result.status(), 200, "request {} should return 200", i);
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 12: Nested timers are tracked correctly.
#[test]
fn test_nested_timers_tracking() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_nested.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-nested-1");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Create multiple timers and intervals
        js_runtime
            .execute_script(
                "<create_timers>",
                deno_core::ascii_str!(r#"
                    globalThis.__timer1 = setTimeout(() => {}, 10000);
                    globalThis.__timer2 = setTimeout(() => {}, 20000);
                    globalThis.__interval1 = setInterval(() => {}, 5000);
                    globalThis.__interval2 = setInterval(() => {}, 15000);
                "#),
            )
            .map_err(|e| format!("create timers: {e}"))?;

        // Verify all are registered
        let check_count = js_runtime
            .execute_script(
                "<check_count>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-nested-1");
                        const intervals = globalThis.__edgeRuntime._intervalRegistry.get("test-nested-1");
                        return timers && timers.size === 2 && intervals && intervals.size === 2;
                    })();
                "#),
            )
            .map_err(|e| format!("check count: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_count.open(scope);
            assert!(local_val.is_true(), "should have 2 timers and 2 intervals registered");
        }

        // Clear execution - all should be removed
        js_runtime
            .execute_script(
                "<clear_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.clearExecutionTimers("test-nested-1");"#),
            )
            .map_err(|e| format!("clearExecutionTimers: {e}"))?;

        // Verify all are gone
        let check_cleared = js_runtime
            .execute_script(
                "<check_cleared>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-nested-1");
                        const intervals = globalThis.__edgeRuntime._intervalRegistry.get("test-nested-1");
                        return timers === undefined && intervals === undefined;
                    })();
                "#),
            )
            .map_err(|e| format!("check cleared: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_cleared.open(scope);
            assert!(local_val.is_true(), "all timers and intervals should be cleared");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

/// Test 13: Timer callback execution removes from registry.
#[test]
fn test_timer_callback_removes_from_registry() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///test_callback.js", "globalThis.__test = true;");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip);

        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Start execution context
        js_runtime
            .execute_script(
                "<start_exec>",
                deno_core::ascii_str!(r#"globalThis.__edgeRuntime.startExecution("test-callback-1");"#),
            )
            .map_err(|e| format!("startExecution: {e}"))?;

        // Create a very short timer and store its ID
        js_runtime
            .execute_script(
                "<create_timer>",
                deno_core::ascii_str!(r#"
                    globalThis.__callbackFired = false;
                    globalThis.__testTimerId = setTimeout(() => {
                        globalThis.__callbackFired = true;
                    }, 10);
                "#),
            )
            .map_err(|e| format!("create timer: {e}"))?;

        // Verify timer is in registry before callback fires
        let check_before = js_runtime
            .execute_script(
                "<check_before>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-callback-1");
                        return timers && timers.has(globalThis.__testTimerId);
                    })();
                "#),
            )
            .map_err(|e| format!("check before: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_before.open(scope);
            assert!(local_val.is_true(), "timer should be in registry before callback");
        }

        // Wait a bit and run event loop to let the timer fire
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop: {e}"))?;

        // Verify callback fired
        let check_fired = js_runtime
            .execute_script(
                "<check_fired>",
                deno_core::ascii_str!(r#"globalThis.__callbackFired === true;"#),
            )
            .map_err(|e| format!("check fired: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_fired.open(scope);
            assert!(local_val.is_true(), "callback should have fired");
        }

        // Verify timer was removed from registry after callback
        let check_after = js_runtime
            .execute_script(
                "<check_after>",
                deno_core::ascii_str!(r#"
                    (function() {
                        const timers = globalThis.__edgeRuntime._timerRegistry.get("test-callback-1");
                        return timers && !timers.has(globalThis.__testTimerId);
                    })();
                "#),
            )
            .map_err(|e| format!("check after: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = check_after.open(scope);
            assert!(local_val.is_true(), "timer should be removed from registry after callback fires");
        }

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}
