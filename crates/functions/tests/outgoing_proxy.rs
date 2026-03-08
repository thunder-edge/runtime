use functions::registry::{FunctionRegistry, PoolRuntimeConfig};
use functions::types::BundlePackage;
use functions::types::PoolLimits;
use runtime_core::isolate::{IsolateConfig, IsolateResponseBody, OutgoingProxyConfig};
use runtime_core::ssrf::SsrfConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

static INIT: std::sync::Once = std::sync::Once::new();
static PROXY_ENV_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

fn lock_proxy_env_tests() -> std::sync::MutexGuard<'static, ()> {
    PROXY_ENV_TEST_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("proxy env test lock poisoned")
}

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
    let root = deno_graph::ModuleSpecifier::parse(specifier).expect("invalid root specifier");

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

async fn deploy_inline_function(
    name: &str,
    source: &str,
    config: IsolateConfig,
    outgoing_proxy: OutgoingProxyConfig,
) -> Result<FunctionRegistry, String> {
    let eszip_bytes = build_eszip_async("file:///proxy_test.ts", source).await;
    let bundle = BundlePackage::eszip_only(eszip_bytes);
    let bundle_data = bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

    let registry = FunctionRegistry::new_with_pool(
        CancellationToken::new(),
        config.clone(),
        PoolRuntimeConfig {
            outgoing_proxy,
            ..PoolRuntimeConfig::default()
        },
        PoolLimits::default(),
    );
    registry
        .deploy(
            name.to_string(),
            bytes::Bytes::from(bundle_data),
            Some(config),
            None,
        )
        .await
        .map_err(|e| format!("deploy failed: {e}"))?;

    Ok(registry)
}

async fn invoke_text(
    registry: &FunctionRegistry,
    name: &str,
    path: &str,
) -> Result<(u16, String), String> {
    let handle = registry
        .get_handle(name)
        .ok_or_else(|| format!("missing handle for {name}"))?;

    let req = http::Request::builder()
        .method("GET")
        .uri(path)
        .header("host", "localhost:9000")
        .body(bytes::Bytes::new())
        .map_err(|e| format!("build request: {e}"))?;

    let response = handle
        .send_request(req)
        .await
        .map_err(|e| format!("send request: {e}"))?;

    let text = match response.body {
        IsolateResponseBody::Full(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        IsolateResponseBody::Stream(mut rx) => {
            let mut out = Vec::new();
            while let Some(next) = rx.recv().await {
                let chunk = next.map_err(|e| format!("stream chunk error: {e}"))?;
                out.extend_from_slice(&chunk);
            }
            String::from_utf8(out).map_err(|e| format!("stream utf8: {e}"))?
        }
    };

    Ok((response.parts.status.as_u16(), text))
}

async fn start_one_shot_http_server(
    response_body: &'static str,
) -> Result<std::net::SocketAddr, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind server: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("local addr: {e}"))?;

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0_u8; 2048];
            let _ = stream.read(&mut buf).await;
            let body = response_body.as_bytes();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                response_body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });

    Ok(addr)
}

fn base_config() -> IsolateConfig {
    let mut cfg = IsolateConfig::default();
    cfg.ssrf_config = SsrfConfig::disabled();
    cfg
}

#[test]
fn outgoing_http_proxy_routes_request() {
    let _guard = lock_proxy_env_tests();
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let proxy_addr = start_one_shot_http_server("PROXIED-HTTP").await?;

        let source = r#"
            Deno.serve(async (req) => {
              try {
                const target = new URL(req.url).searchParams.get('url') || 'http://example.invalid/';
                const resp = await fetch(target);
                const text = await resp.text();
                return new Response(text, { status: resp.status });
              } catch (err) {
                                const debug = JSON.stringify({
                                    createHttpClient: typeof Deno?.createHttpClient,
                                    proxyConfig: globalThis.__edgeRuntime?._proxyConfig,
                                });
                                return new Response(String(err) + " | debug=" + debug, { status: 502 });
              }
            });
        "#;

        let cfg = base_config();
        let outgoing_proxy = OutgoingProxyConfig {
            http_proxy: Some(format!("http://{}", proxy_addr)),
            https_proxy: None,
            tcp_proxy: None,
            http_no_proxy: vec![],
            https_no_proxy: vec![],
            tcp_no_proxy: vec![],
        };

        let registry =
            deploy_inline_function("proxy-http-route", source, cfg, outgoing_proxy).await?;
        let (status, body) = invoke_text(
            &registry,
            "proxy-http-route",
            "/?url=http://does-not-resolve.invalid/path",
        )
        .await?;

        if status != 200 || body != "PROXIED-HTTP" {
            return Err(format!("expected proxied response; status={status}, body={body}"));
        }

        registry
            .delete("proxy-http-route")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn outgoing_http_no_proxy_bypasses_proxy() {
    let _guard = lock_proxy_env_tests();
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let direct_addr = start_one_shot_http_server("DIRECT-BYPASS").await?;
        let _proxy_addr = start_one_shot_http_server("PROXIED-HTTP").await?;

        let source = r#"
            Deno.serve(async (req) => {
              try {
                const target = new URL(req.url).searchParams.get('url') || 'http://example.invalid/';
                const resp = await fetch(target);
                const text = await resp.text();
                return new Response(text, { status: resp.status });
              } catch (err) {
                                const debug = JSON.stringify({
                                    createHttpClient: typeof Deno?.createHttpClient,
                                    proxyConfig: globalThis.__edgeRuntime?._proxyConfig,
                                });
                                return new Response(String(err) + " | debug=" + debug, { status: 502 });
              }
            });
        "#;

        let cfg = base_config();
        let outgoing_proxy = OutgoingProxyConfig {
            http_proxy: Some(format!("http://{}", _proxy_addr)),
            https_proxy: None,
            tcp_proxy: None,
            http_no_proxy: vec!["127.0.0.1".to_string()],
            https_no_proxy: vec![],
            tcp_no_proxy: vec![],
        };

        let registry =
            deploy_inline_function("proxy-http-bypass", source, cfg, outgoing_proxy).await?;
        let path = format!("/?url=http://{}/bypass", direct_addr);
        let (status, body) = invoke_text(&registry, "proxy-http-bypass", &path).await?;

        if status != 200 || body != "DIRECT-BYPASS" {
            return Err(format!(
                "expected direct bypass response; status={status}, body={body}"
            ));
        }

        registry
            .delete("proxy-http-bypass")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn outgoing_proxy_unavailable_returns_clear_error() {
    let _guard = lock_proxy_env_tests();
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async (req) => {
              try {
                const target = new URL(req.url).searchParams.get('url') || 'http://example.invalid/';
                const resp = await fetch(target);
                const text = await resp.text();
                return new Response(text, { status: resp.status });
              } catch (err) {
                                const debug = JSON.stringify({
                                    createHttpClient: typeof Deno?.createHttpClient,
                                    proxyConfig: globalThis.__edgeRuntime?._proxyConfig,
                                });
                                return new Response(String(err) + " | debug=" + debug, { status: 502 });
              }
            });
        "#;

        let cfg = base_config();
        let outgoing_proxy = OutgoingProxyConfig {
            http_proxy: Some("http://127.0.0.1:9".to_string()),
            https_proxy: None,
            tcp_proxy: None,
            http_no_proxy: vec![],
            https_no_proxy: vec![],
            tcp_no_proxy: vec![],
        };

        let registry = deploy_inline_function("proxy-http-down", source, cfg, outgoing_proxy).await?;
        let (status, body) = invoke_text(
            &registry,
            "proxy-http-down",
            "/?url=http://does-not-resolve.invalid/unreachable",
        )
        .await?;

        if status != 502 || body.trim().is_empty() {
            return Err(format!(
                "expected clear proxy error; status={status}, body={body}"
            ));
        }

        registry
            .delete("proxy-http-down")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}
