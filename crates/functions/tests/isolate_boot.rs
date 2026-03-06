//! Integration test: boots a JsRuntime with the same extensions used in
//! production and evaluates simple JS scripts to surface any missing ops,
//! modules, or boot-time errors.

use std::rc::Rc;
use std::sync::Arc;

use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::module_loader::EszipModuleLoader;

/// Helper: create a JsRuntime with the same config as production isolates.
fn make_runtime_with_eszip(eszip: Arc<eszip::EszipV2>) -> JsRuntime {
    let module_loader = Rc::new(EszipModuleLoader::new(eszip));
    let mut runtime_extensions = extensions::get_extensions();
    runtime_extensions.push(functions::handler::response_stream_extension());

    let mut opts = RuntimeOptions {
        module_loader: Some(module_loader),
        extensions: runtime_extensions,
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);

    let mut runtime = JsRuntime::new(opts);
    functions::handler::ensure_response_stream_registry(&mut runtime);
    runtime
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

/// Test 1: JsRuntime boots without panicking (extensions + transpiler).
#[test]
fn test_runtime_boots() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_boot.js",
        "globalThis.__edgeRuntime = { ok: true };",
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let mut js_runtime = make_runtime_with_eszip(eszip.clone());

        // Just running execute_script proves the runtime booted.
        js_runtime
            .execute_script("<test>", deno_core::ascii_str!("1 + 1"))
            .map(|_| ())
            .map_err(|e| format!("execute_script failed: {e}"))
    });

    assert!(result.is_ok(), "runtime boot failed: {:?}", result.err());
}

/// Test 2: Load and evaluate an eszip module (same path as deploy).
#[test]
fn test_module_load_and_eval() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_handler.js",
        r#"
        globalThis.__edgeRuntime = {
            handler: null,
            registerHandler(fn) { this.handler = fn; },
        };
        globalThis.__edgeRuntime.registerHandler(async (req) => {
            return new Response("hello from test");
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let root = runtime_core::isolate::determine_root_specifier(&eszip)
            .map_err(|e| format!("determine_root_specifier: {e}"))?;

        let mut js_runtime = make_runtime_with_eszip(eszip);

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

        // Verify handler was registered
        let val = js_runtime
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__edgeRuntime.handler !== null"),
            )
            .map_err(|e| format!("handler check failed: {e}"))?;

        {
            deno_core::scope!(scope, js_runtime);
            let local_val = val.open(scope);
            if !local_val.is_true() {
                return Err("handler was NOT registered".to_string());
            }
        }

        Ok(())
    });

    assert!(result.is_ok(), "module eval failed: {:?}", result.err());
}

/// Test 3: Full cycle — bundle JS, parse, boot runtime, inject bridge, load module, send request.
#[test]
fn test_full_request_cycle() {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip(
        "file:///test_full.js",
        r#"
        Deno.serve(async (req) => {
            return new Response("ok:" + req.method);
        });
        "#,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<String, String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let root = runtime_core::isolate::determine_root_specifier(&eszip)
            .map_err(|e| format!("determine_root_specifier: {e}"))?;

        let mut js_runtime = make_runtime_with_eszip(eszip);

        // Inject the request bridge (same as production)
        functions::handler::inject_request_bridge(&mut js_runtime)
            .map_err(|e| format!("inject_request_bridge: {e}"))?;

        // Load user module
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

        // Dispatch a request using the same handler as production.
        // The router forwards only the rewritten path (no scheme/host).
        let request = http::Request::builder()
            .method("GET")
            .uri("/test")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .unwrap();

        let response = functions::handler::dispatch_request(&mut js_runtime, request)
            .await
            .map_err(|e| format!("dispatch_request: {e}"))?;

        let body = match response.body {
            runtime_core::isolate::IsolateResponseBody::Full(bytes) => {
                String::from_utf8_lossy(&bytes).to_string()
            }
            runtime_core::isolate::IsolateResponseBody::Stream(mut rx) => {
                let mut buf = Vec::new();
                while let Some(next) = rx.recv().await {
                    let chunk = next.map_err(|e| format!("stream chunk error: {e}"))?;
                    buf.extend_from_slice(&chunk);
                }
                String::from_utf8(buf).map_err(|e| format!("stream utf8 body: {e}"))?
            }
        };
        Ok(body)
    });

    match result {
        Ok(body) => assert_eq!(body, "ok:GET", "unexpected response body: {body}"),
        Err(e) => panic!("full cycle failed: {e}"),
    }
}
