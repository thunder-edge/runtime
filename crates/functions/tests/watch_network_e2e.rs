//! E2E: watch-mode network policy should allow private network access.
//!
//! This test emulates watch mode by disabling SSRF protection in IsolateConfig,
//! deploys a function, and verifies it can fetch a private local endpoint.

use functions::registry::FunctionRegistry;
use functions::types::BundlePackage;
use runtime_core::isolate::IsolateConfig;
use runtime_core::ssrf::SsrfConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

static INIT: std::sync::Once = std::sync::Once::new();

fn init_runtime() {
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        deno_core::JsRuntime::init_platform(None);
    });
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

#[test]
fn watch_mode_allows_fetch_to_private_localhost() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result: Result<(), String> = rt.block_on(async {
        // Local private endpoint used by the function fetch.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("bind local server: {e}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| format!("local addr: {e}"))?;

        let server_task = tokio::spawn(async move {
            if let Ok((mut stream, _peer)) = listener.accept().await {
                let mut buf = [0_u8; 2048];
                let _ = stream.read(&mut buf).await;
                let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\npong";
                let _ = stream.write_all(response).await;
                let _ = stream.shutdown().await;
            }
        });

        let js_source = format!(
            r#"
            Deno.serve(async (_req) => {{
                const r = await fetch("http://127.0.0.1:{}/ping");
                const txt = await r.text();
                return new Response("ok:" + txt);
            }});
            "#,
            local_addr.port()
        );

        let eszip_bytes = build_eszip_async("file:///watch_e2e.ts", &js_source).await;
        let bundle = BundlePackage::eszip_only(eszip_bytes);
        let bundle_data =
            bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

        let mut watch_like_config = IsolateConfig::default();
        watch_like_config.ssrf_config = SsrfConfig::disabled();

        let registry = FunctionRegistry::new(CancellationToken::new(), IsolateConfig::default());
        registry
            .deploy(
                "watch-network-e2e".to_string(),
                bytes::Bytes::from(bundle_data),
                Some(watch_like_config),
                None,
            )
            .await
            .map_err(|e| format!("deploy: {e}"))?;

        let handle = registry
            .get_handle("watch-network-e2e")
            .ok_or_else(|| "missing handle after deploy".to_string())?;

        let req = http::Request::builder()
            .method("GET")
            .uri("/invoke")
            .header("host", "localhost:9000")
            .body(bytes::Bytes::new())
            .map_err(|e| format!("build request: {e}"))?;

        let resp = handle
            .send_request(req)
            .await
            .map_err(|e| format!("send_request: {e}"))?;

        if resp.parts.status != 200 {
            return Err(format!(
                "expected 200 from function, got {}",
                resp.parts.status
            ));
        }

        let body = match resp.body {
            runtime_core::isolate::IsolateResponseBody::Full(bytes) => {
                String::from_utf8(bytes.to_vec()).map_err(|e| format!("response body utf8: {e}"))?
            }
            runtime_core::isolate::IsolateResponseBody::Stream(mut rx) => {
                let mut buf = Vec::new();
                while let Some(next) = rx.recv().await {
                    let chunk = next.map_err(|e| format!("stream chunk error: {e}"))?;
                    buf.extend_from_slice(&chunk);
                }
                String::from_utf8(buf).map_err(|e| format!("response body utf8: {e}"))?
            }
        };
        if body != "ok:pong" {
            return Err(format!("unexpected function body: '{body}'"));
        }

        registry
            .delete("watch-network-e2e")
            .await
            .map_err(|e| format!("delete function: {e}"))?;

        tokio::time::timeout(std::time::Duration::from_secs(1), server_task)
            .await
            .map_err(|_| "local server task timeout".to_string())
            .and_then(|join| join.map_err(|e| format!("local server task join: {e}")))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}
