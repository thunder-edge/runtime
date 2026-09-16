use super::*;

use std::sync::Arc;
use std::sync::Once;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use functions::registry::{FunctionRegistry, PoolRuntimeConfig};
use functions::types::{BundlePackage, PoolLimits};
use rcgen::generate_simple_self_signed;
use runtime_core::isolate::IsolateConfig;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

static RUSTLS_INIT: Once = Once::new();
static DENO_INIT: Once = Once::new();

fn init_rustls_provider() {
    RUSTLS_INIT.call_once(|| {
        let provider = rustls::crypto::ring::default_provider();
        provider
            .install_default()
            .expect("failed to install rustls crypto provider");
    });
}

fn make_test_registry() -> Arc<FunctionRegistry> {
    Arc::new(FunctionRegistry::new(
        CancellationToken::new(),
        IsolateConfig::default(),
    ))
}

fn make_pool_enabled_registry() -> Arc<FunctionRegistry> {
    Arc::new(FunctionRegistry::new_with_pool(
        CancellationToken::new(),
        IsolateConfig::default(),
        PoolRuntimeConfig {
            enabled: true,
            global_max_isolates: 64,
            min_free_memory_mib: 0,
            capacity_wait_timeout_ms: 75,
            capacity_wait_max_waiters: 20_000,
            outgoing_proxy: runtime_core::isolate::OutgoingProxyConfig::default(),
        },
        PoolLimits::default(),
        functions::types::ContextPoolLimits::default(),
    ))
}

fn init_deno_platform() {
    DENO_INIT.call_once(|| {
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
    let root = deno_graph::ModuleSpecifier::parse(specifier).expect("invalid specifier");

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
    .expect("from_graph failed for e2e eszip fixture");

    eszip.into_bytes()
}

async fn send_plain_http(addr: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("failed to connect to server");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("failed to write request");

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("timed out waiting for response")
        .expect("failed to read response");

    String::from_utf8_lossy(&response).to_string()
}

async fn send_plain_http_and_abort_after_body_prefix(addr: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("failed to connect to server");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("failed to write request");

    let mut response = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
            .await
            .expect("timed out waiting for streaming response")
            .expect("failed to read streaming response");
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..read]);

        let has_headers = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .is_some_and(|header_end| response.len() > header_end + 4);
        if has_headers {
            break;
        }
    }

    String::from_utf8_lossy(&response).to_string()
}

async fn send_plain_http_bytes(addr: SocketAddr, head: &str, body: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("failed to connect to server");
    stream
        .write_all(head.as_bytes())
        .await
        .expect("failed to write request head");
    stream
        .write_all(body)
        .await
        .expect("failed to write request body");

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("timed out waiting for response")
        .expect("failed to read response");

    String::from_utf8_lossy(&response).to_string()
}

fn parse_http_json_body(response: &str) -> Value {
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("");
    serde_json::from_str(body)
        .unwrap_or_else(|err| panic!("failed to parse response body as json: {err}; body={body}"))
}

async fn wait_for_tcp_listener(addr: SocketAddr) {
    // Poll listener readiness to avoid fixed startup sleeps in E2E tests.
    for _ in 0..60 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    panic!("listener did not become ready in time: {addr}");
}

async fn reserve_dual_listener_addrs() -> (SocketAddr, SocketAddr) {
    // Keep both probe listeners alive simultaneously so the OS cannot recycle
    // the same ephemeral port for admin and ingress.
    let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind admin probe listener");
    let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ingress probe listener");

    let admin_addr = admin_probe
        .local_addr()
        .expect("failed to get admin local addr");
    let ingress_addr = ingress_probe
        .local_addr()
        .expect("failed to get ingress local addr");

    assert_ne!(
        admin_addr, ingress_addr,
        "expected admin and ingress listeners to use distinct addresses"
    );

    drop(ingress_probe);
    drop(admin_probe);

    (admin_addr, ingress_addr)
}

async fn wait_for_routing_saturation(addr: SocketAddr) {
    let mut last_metrics = String::new();
    for _ in 0..100 {
        let metrics_resp = send_plain_http(
            addr,
            "GET /_internal/metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        last_metrics = metrics_resp.clone();

        if metrics_resp.starts_with("HTTP/1.1 200") {
            let metrics = parse_http_json_body(&metrics_resp);
            let routing = metrics.get("routing");
            let total_contexts = routing
                .and_then(|r| r.get("total_contexts"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let saturated_contexts = routing
                .and_then(|r| r.get("saturated_contexts"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let saturated_isolates = routing
                .and_then(|r| r.get("saturated_isolates"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let total_active_requests = routing
                .and_then(|r| r.get("total_active_requests"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if total_contexts >= 1
                && saturated_contexts >= 1
                && saturated_isolates >= 1
                && total_active_requests >= 1
            {
                return;
            }
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    panic!("timed out waiting for routing saturation to be observed in metrics: {last_metrics}");
}

fn make_temp_tls_files() -> (
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    Vec<u8>,
) {
    let cert = generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("failed to generate self-signed certificate");

    let cert_pem = cert.serialize_pem().expect("failed to serialize cert pem");
    let key_pem = cert.serialize_private_key_pem();
    let cert_der = cert.serialize_der().expect("failed to serialize cert der");

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("edge-server-tls-test-{unique}"));
    std::fs::create_dir_all(&dir).expect("failed to create temp dir for tls test");

    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert_pem).expect("failed to write cert.pem");
    std::fs::write(&key_path, key_pem).expect("failed to write key.pem");

    (dir, cert_path, key_path, cert_der)
}

#[tokio::test]
async fn e2e_tls_accepts_https_connection() {
    init_rustls_provider();

    let (temp_dir, cert_path, key_path, cert_der) = make_temp_tls_files();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get local addr");
    drop(probe_listener);

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let server_config = ServerConfig {
        addr,
        tls: Some(TlsConfig {
            cert_path: cert_path.to_string_lossy().to_string(),
            key_path: key_path.to_string_lossy().to_string(),
        }),
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig::default(),
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let server_handle =
        tokio::spawn(async move { run_server(server_config, registry, server_shutdown).await });

    wait_for_tcp_listener(addr).await;

    let mut roots = RootCertStore::empty();
    roots
        .add(cert_der.into())
        .expect("failed to add self-signed cert to root store");

    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(client_config));
    let tcp = TcpStream::connect(addr)
        .await
        .expect("failed to connect tcp to server");

    let server_name = ServerName::try_from("localhost").expect("invalid server name");
    let mut tls_stream = connector
        .connect(server_name, tcp)
        .await
        .expect("tls handshake failed");

    tls_stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("failed to write request over tls");

    let mut response = vec![0_u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), tls_stream.read(&mut response))
        .await
        .expect("timed out waiting for response")
        .expect("failed to read response");

    assert!(n > 0, "server returned empty response over TLS");
    let response_text = String::from_utf8_lossy(&response[..n]);
    assert!(
        response_text.starts_with("HTTP/1.1"),
        "expected HTTP response, got: {response_text}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");

    std::fs::remove_dir_all(temp_dir).expect("failed to cleanup temp tls dir");
}

#[tokio::test]
async fn e2e_admin_pool_endpoints_update_and_read_limits() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get local addr");
    drop(probe_listener);

    let registry = make_pool_enabled_registry();
    let shutdown = CancellationToken::new();

    let hello_eszip = build_eszip_async(
        "file:///pool_e2e.ts",
        r#"
        Deno.serve(async () => new Response("ok"));
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(hello_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "pool-e2e-fn".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy test function");

    let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ingress probe listener");
    let ingress_addr = ingress_probe
        .local_addr()
        .expect("failed to get ingress local addr");
    drop(ingress_probe);

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let put_body = r#"{"min":1,"max":2}"#;
    let put_req = format!(
        "PUT /_internal/functions/pool-e2e-fn/pool HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        put_body.len(),
        put_body
    );
    let put_resp = send_plain_http(addr, &put_req).await;
    assert!(
        put_resp.starts_with("HTTP/1.1 200"),
        "unexpected PUT response: {put_resp}"
    );
    assert!(
        put_resp.contains("\"pool\":{\"min\":1,\"max\":2"),
        "PUT response should include updated pool limits: {put_resp}"
    );

    let get_req = "GET /_internal/functions/pool-e2e-fn/pool HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let get_resp = send_plain_http(addr, get_req).await;
    assert!(
        get_resp.starts_with("HTTP/1.1 200"),
        "unexpected GET response: {get_resp}"
    );
    assert!(
        get_resp.contains("\"min\":1") && get_resp.contains("\"max\":2"),
        "GET response should return updated limits: {get_resp}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_deploy_corrupted_bundle_returns_400_without_crash() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let bad_payload = b"not-a-valid-bundle";
    let bad_req = format!(
        "POST /_internal/functions?name=corrupted HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        bad_payload.len(),
        String::from_utf8_lossy(bad_payload)
    );
    let bad_resp = send_plain_http(admin_addr, &bad_req).await;
    assert!(
        bad_resp.starts_with("HTTP/1.1 400"),
        "expected 400 for corrupted bundle deploy, got: {bad_resp}"
    );

    let list_req =
        "GET /_internal/functions HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let list_resp = send_plain_http(admin_addr, list_req).await;
    assert!(
        list_resp.starts_with("HTTP/1.1 200"),
        "server should remain alive after bad bundle deploy: {list_resp}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_admin_auth_and_public_ingress_behavior() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let hello_eszip = build_eszip_async(
        "file:///auth_e2e.ts",
        r#"
        Deno.serve(async () => new Response("auth-ok"));
        "#,
    )
    .await;

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: Some("secret-key".to_string()),
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let body = bincode::serialize(&BundlePackage::eszip_only(hello_eszip))
        .expect("serialize bundle package");

    let req_no_key_head = format!(
        "POST /_internal/functions HTTP/1.1\r\nHost: localhost\r\nx-function-name: auth-e2e\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let resp_no_key = send_plain_http_bytes(admin_addr, &req_no_key_head, &body).await;
    assert!(
        resp_no_key.starts_with("HTTP/1.1 401"),
        "expected 401 without API key: {resp_no_key}"
    );

    let req_wrong_key_head = format!(
        "POST /_internal/functions HTTP/1.1\r\nHost: localhost\r\nx-function-name: auth-e2e\r\nX-API-Key: wrong-key\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let resp_wrong_key = send_plain_http_bytes(admin_addr, &req_wrong_key_head, &body).await;
    assert!(
        resp_wrong_key.starts_with("HTTP/1.1 401"),
        "expected 401 with wrong API key: {resp_wrong_key}"
    );

    let req_ok_key_head = format!(
        "POST /_internal/functions HTTP/1.1\r\nHost: localhost\r\nx-function-name: auth-e2e\r\nX-API-Key: secret-key\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let resp_ok_key = send_plain_http_bytes(admin_addr, &req_ok_key_head, &body).await;
    assert!(
        resp_ok_key.starts_with("HTTP/1.1 200") || resp_ok_key.starts_with("HTTP/1.1 201"),
        "expected success with correct API key: {resp_ok_key}"
    );

    let ingress_req = "GET /auth-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let ingress_resp = send_plain_http(ingress_addr, ingress_req).await;
    assert!(
        ingress_resp.starts_with("HTTP/1.1 200"),
        "expected public ingress request without API key to work: {ingress_resp}"
    );

    let admin_ingress_req =
        "GET /auth-e2e HTTP/1.1\r\nHost: localhost\r\nX-API-Key: secret-key\r\nConnection: close\r\n\r\n";
    let admin_ingress_resp = send_plain_http(admin_addr, admin_ingress_req).await;
    assert!(
        admin_ingress_resp.starts_with("HTTP/1.1 404"),
        "expected admin listener to reject function ingress route: {admin_ingress_resp}"
    );
    assert!(
        admin_ingress_resp.contains("admin listener serves only /_internal/* routes"),
        "expected admin 404 response to include ingress hint: {admin_ingress_resp}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_host_based_routing_updates_via_put_and_routes_without_prefix() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let hello_eszip = build_eszip_async(
        "file:///host_routing_e2e.ts",
        r#"
        Deno.serve(async (req) => {
          const path = new URL(req.url).pathname;
          return new Response(`host-route-ok:${path}`);
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(hello_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "host-route-fn".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy host routing test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let before_req =
        "GET /api/ping HTTP/1.1\r\nHost: api.customer-a.local\r\nConnection: close\r\n\r\n";
    let before_resp = send_plain_http(ingress_addr, before_req).await;
    assert!(
        before_resp.starts_with("HTTP/1.1 400") || before_resp.starts_with("HTTP/1.1 404"),
        "expected unresolved route before PUT update: {before_resp}"
    );

    let routing_manifest = r#"{
        "manifestVersion": 1,
        "routes": [
            {
                "host": "api.customer-a.local",
                "path": "/api/:name",
                "targetFunction": "host-route-fn"
            }
        ]
    }"#;
    let put_req = format!(
        "PUT /_internal/routing HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        routing_manifest.len(),
        routing_manifest
    );
    let put_resp = send_plain_http(admin_addr, &put_req).await;
    assert!(
        put_resp.starts_with("HTTP/1.1 200"),
        "expected routing PUT success: {put_resp}"
    );

    let after_req =
        "GET /api/ping HTTP/1.1\r\nHost: api.customer-a.local\r\nConnection: close\r\n\r\n";
    let after_resp = send_plain_http(ingress_addr, after_req).await;
    assert!(
        after_resp.starts_with("HTTP/1.1 200"),
        "expected host-based routing request success after PUT: {after_resp}"
    );
    assert!(
        after_resp.contains("host-route-ok:/api/ping"),
        "expected request path to be forwarded unchanged for host-based route: {after_resp}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_streaming_returns_progressive_chunked_body() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let stream_eszip = build_eszip_async(
        "file:///stream_e2e.ts",
        r#"
        Deno.serve(async () => {
          const encoder = new TextEncoder();
          const stream = new ReadableStream({
            start(controller) {
              controller.enqueue(encoder.encode('first-'));
              setTimeout(() => controller.enqueue(encoder.encode('second')), 120);
              setTimeout(() => controller.close(), 180);
            },
          });
          return new Response(stream, {
            headers: { 'content-type': 'text/plain' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "stream-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy streaming test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    // Warm up isolate to avoid cold-start skew in progressivity timing assertions.
    let warmup_resp = send_plain_http(
        ingress_addr,
        "GET /stream-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        warmup_resp.starts_with("HTTP/1.1 200"),
        "warmup request failed: {warmup_resp}"
    );

    let mut stream = TcpStream::connect(ingress_addr)
        .await
        .expect("failed to connect to ingress");
    stream
        .write_all(b"GET /stream-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("failed to write ingress request");

    let mut first_buf = [0_u8; 4096];
    let first_n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut first_buf))
        .await
        .expect("timed out waiting for first streamed bytes")
        .expect("failed to read first streamed bytes");
    assert!(first_n > 0, "expected response bytes for first chunk");

    let first_text = String::from_utf8_lossy(&first_buf[..first_n]).to_string();
    let first_lower = first_text.to_ascii_lowercase();
    assert!(
        first_text.starts_with("HTTP/1.1 200"),
        "expected 200 response, got: {first_text}"
    );
    assert!(
        first_lower.contains("transfer-encoding: chunked"),
        "expected chunked transfer-encoding for streaming response: {first_text}"
    );
    assert!(
        first_text.contains("first-"),
        "expected first chunk in early response bytes: {first_text}"
    );

    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut tail))
        .await
        .expect("timed out waiting for streamed response completion")
        .expect("failed to read streamed response tail");

    let tail_text = String::from_utf8_lossy(&tail).to_string();
    let full_text = format!("{first_text}{tail_text}");
    assert!(
        full_text.contains("second"),
        "expected delayed second chunk in response body: {full_text}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_sse_streams_events_progressively() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;
    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let sse_eszip = build_eszip_async(
        "file:///sse_stream_e2e.ts",
        r#"
        Deno.serve(() => {
          const encoder = new TextEncoder();
          const stream = new ReadableStream({
            start(controller) {
              controller.enqueue(encoder.encode('event: message\ndata: first\n\n'));
              setTimeout(() => controller.enqueue(
                encoder.encode('event: message\ndata: second\n\n'),
              ), 80);
              setTimeout(() => controller.close(), 120);
            },
          });
          return new Response(stream, {
            headers: {
              'cache-control': 'no-cache',
              'content-type': 'text/event-stream',
            },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(sse_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize SSE bundle");
    registry
        .deploy(
            "sse-stream-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy SSE test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };
    let server_handle = tokio::spawn({
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        async move { run_dual_server(server_config, registry, shutdown).await }
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let mut stream = TcpStream::connect(ingress_addr)
        .await
        .expect("failed to connect to SSE ingress");
    stream
        .write_all(b"GET /sse-stream-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("failed to write SSE request");

    let mut first_buf = [0_u8; 4096];
    let first_n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut first_buf))
        .await
        .expect("timed out waiting for first SSE event")
        .expect("failed to read first SSE event");
    let first_text = String::from_utf8_lossy(&first_buf[..first_n]).to_string();
    let first_lower = first_text.to_ascii_lowercase();
    assert!(
        first_text.starts_with("HTTP/1.1 200"),
        "expected SSE 200: {first_text}"
    );
    assert!(
        first_lower.contains("content-type: text/event-stream"),
        "expected SSE content type: {first_text}"
    );
    assert!(
        first_text.contains("data: first"),
        "expected first SSE event before the delayed event: {first_text}"
    );

    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut tail))
        .await
        .expect("timed out waiting for SSE completion")
        .expect("failed to read SSE tail");
    let full_text = format!("{first_text}{}", String::from_utf8_lossy(&tail));
    assert!(
        full_text.contains("data: second"),
        "expected delayed second SSE event: {full_text}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_streaming_long_chunked_body_completes() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let stream_eszip = build_eszip_async(
        "file:///long_stream_e2e.ts",
        r#"
        Deno.serve(async () => {
          const encoder = new TextEncoder();
          const stream = new ReadableStream({
            start(controller) {
              (async () => {
                for (let i = 0; i < 300; i++) {
                  controller.enqueue(encoder.encode(`chunk-${String(i).padStart(4, '0')}\n`));
                  if (i % 25 === 0) {
                    await new Promise((resolve) => setTimeout(resolve, 2));
                  }
                }
                controller.close();
              })().catch((err) => controller.error(err));
            },
          });
          return new Response(stream, {
            headers: { 'content-type': 'text/plain' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "stream-long-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy long streaming test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let mut stream = TcpStream::connect(ingress_addr)
        .await
        .expect("failed to connect to ingress");
    stream
        .write_all(b"GET /stream-long-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("failed to write ingress request");

    let mut first_buf = [0_u8; 4096];
    let first_n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut first_buf))
        .await
        .expect("timed out waiting for first streamed bytes")
        .expect("failed to read first streamed bytes");
    assert!(first_n > 0, "expected first streamed bytes");

    let first_text = String::from_utf8_lossy(&first_buf[..first_n]).to_string();
    let first_lower = first_text.to_ascii_lowercase();
    assert!(
        first_text.starts_with("HTTP/1.1 200"),
        "expected 200 response, got: {first_text}"
    );
    assert!(
        first_lower.contains("transfer-encoding: chunked"),
        "expected chunked transfer for long stream response: {first_text}"
    );
    assert!(
        first_text.contains("chunk-0000"),
        "expected initial chunk marker in first response bytes: {first_text}"
    );

    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut tail))
        .await
        .expect("timed out waiting for long streamed response completion")
        .expect("failed to read long streamed response tail");

    let tail_text = String::from_utf8_lossy(&tail).to_string();
    let full_text = format!("{first_text}{tail_text}");
    assert!(
        full_text.contains("chunk-0299"),
        "expected final chunk marker in long response body"
    );
    assert!(
        full_text.matches("chunk-").count() >= 250,
        "expected many streamed chunks in body"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_streaming_response_exceeds_limit_rejects_before_headers() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let stream_eszip = build_eszip_async(
        "file:///stream_limit_gap_e2e.ts",
        r#"
        Deno.serve(async () => {
          const encoder = new TextEncoder();
          const stream = new ReadableStream({
            start(controller) {
              (async () => {
                for (let i = 0; i < 64; i++) {
                  const payload = i === 0
                    ? `chunk-${String(i).padStart(2, '0')}-${'x'.repeat(600)}\n`
                    : `chunk-${String(i).padStart(2, '0')}-xxxxxxxxxxxxxxxxxxxxxxxxxxxx\n`;
                  controller.enqueue(encoder.encode(payload));
                }
                controller.close();
              })().catch((err) => controller.error(err));
            },
          });

          return new Response(stream, {
            headers: { 'content-type': 'text/plain' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "stream-limit-gap-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy streaming limit gap test function");

    let tiny_limit = 512usize;
    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig {
                max_request_body_bytes: 1024,
                max_response_body_bytes: tiny_limit,
            },
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig {
                max_request_body_bytes: 1024,
                max_response_body_bytes: tiny_limit,
            },
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let response = send_plain_http(
        ingress_addr,
        "GET /stream-limit-gap-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(
        response.starts_with("HTTP/1.1 413"),
        "expected oversized first chunk to be rejected before headers: {response}"
    );
    assert!(
        response.contains(r#""error":"response body too large""#),
        "expected stable response body limit error: {response}"
    );
    assert!(
        !response.contains("chunk-63"),
        "expected oversized stream to stop before later chunks: {response}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_streaming_response_truncates_after_headers() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;
    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let stream_eszip = build_eszip_async(
        "file:///stream_limit_truncate_e2e.ts",
        r#"
        let cancelCount = 0;
        Deno.serve((req) => {
          if (new URL(req.url).pathname === '/check') {
            return new Response(JSON.stringify({
              cancelCount,
              activeStreams: globalThis.__edgeRuntime._streamExecutions.size,
              pendingCancels: globalThis.__edgeRuntime._responseStreamCancels.size,
            }), {
              headers: { 'content-type': 'application/json' },
            });
          }

          const encoder = new TextEncoder();
          const stream = new ReadableStream({
            start(controller) {
              controller.enqueue(encoder.encode('abcdefghij'));
              controller.enqueue(encoder.encode(
                'klmnopqrst' + 'z'.repeat(400) + 'tail-marker',
              ));
            },
            cancel() {
              cancelCount += 1;
            },
          });
          return new Response(stream, {
            headers: { 'content-type': 'text/plain', 'content-length': '431' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "stream-limit-truncate-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy streaming truncation test function");

    let tiny_limit = 128usize;
    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig {
                max_request_body_bytes: 1024,
                max_response_body_bytes: tiny_limit,
            },
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig {
                max_request_body_bytes: 1024,
                max_response_body_bytes: tiny_limit,
            },
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_handle = tokio::spawn({
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        async move { run_dual_server(server_config, registry, shutdown).await }
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let response = send_plain_http(
        ingress_addr,
        "GET /stream-limit-truncate-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected original status after header commit: {response}"
    );
    let response_lower = response.to_ascii_lowercase();
    assert!(
        response_lower.contains("transfer-encoding: chunked"),
        "expected chunked response after content-length removal: {response}"
    );
    assert!(
        !response_lower.contains("content-length: 431"),
        "expected stale content-length to be removed: {response}"
    );
    assert!(
        response.contains("abcdefghij") && response.contains("klmno"),
        "expected the allowed prefix to be forwarded: {response}"
    );
    assert!(
        !response.contains("tail-marker"),
        "expected bytes after the configured limit to be truncated: {response}"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let check_response = loop {
        let check_response = send_plain_http(
            ingress_addr,
            "GET /stream-limit-truncate-e2e/check HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        if check_response.contains(r#""cancelCount":1"#) || tokio::time::Instant::now() >= deadline
        {
            break check_response;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(
        check_response.starts_with("HTTP/1.1 200"),
        "expected a subsequent request after stream cleanup: {check_response}"
    );
    assert!(
        check_response.contains(r#""cancelCount":1"#)
            && check_response.contains(r#""activeStreams":0"#)
            && check_response.contains(r#""pendingCancels":0"#),
        "expected limit cancellation exactly once with empty registries: {check_response}"
    );

    for expected_cancel_count in 2..=4 {
        let repeated_response = send_plain_http(
            ingress_addr,
            "GET /stream-limit-truncate-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            repeated_response.starts_with("HTTP/1.1 200"),
            "expected repeated limited stream to return 200: {repeated_response}"
        );

        let expected_count = format!(r#""cancelCount":{expected_cancel_count}"#);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let repeated_check = loop {
            let repeated_check = send_plain_http(
                ingress_addr,
                "GET /stream-limit-truncate-e2e/check HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await;
            if repeated_check.contains(&expected_count) || tokio::time::Instant::now() >= deadline {
                break repeated_check;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert!(
            repeated_check.contains(&expected_count)
                && repeated_check.contains(r#""activeStreams":0"#)
                && repeated_check.contains(r#""pendingCancels":0"#),
            "expected repeated limit cleanup at count {expected_cancel_count}: {repeated_check}"
        );
    }

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_streaming_producer_error_cleans_up_without_appending_json() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;
    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let stream_eszip = build_eszip_async(
        "file:///stream_error_cleanup_e2e.ts",
        r#"
        let errorCount = 0;
        Deno.serve((req) => {
          if (new URL(req.url).pathname === '/check') {
            return new Response(JSON.stringify({
              errorCount,
              activeStreams: globalThis.__edgeRuntime._streamExecutions.size,
              pendingCancels: globalThis.__edgeRuntime._responseStreamCancels.size,
            }), {
              headers: { 'content-type': 'application/json' },
            });
          }

          const encoder = new TextEncoder();
          return new Response(new ReadableStream({
            start(controller) {
              controller.enqueue(encoder.encode('before-error'));
              setTimeout(() => {
                errorCount += 1;
                controller.error(new Error('producer boom'));
              }, 25);
            },
          }), {
            headers: { 'content-type': 'text/plain' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "stream-error-cleanup-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy streaming error test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_handle = tokio::spawn({
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        async move { run_dual_server(server_config, registry, shutdown).await }
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let response = send_plain_http(
        ingress_addr,
        "GET /stream-error-cleanup-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected original status for partial producer error: {response}"
    );
    assert!(
        response.contains("before-error"),
        "expected bytes emitted before producer error: {response}"
    );
    assert!(
        !response.contains("producer boom") && !response.contains(r#""error":"#),
        "expected no JSON error appended to partial body: {response}"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let check_response = loop {
        let check_response = send_plain_http(
            ingress_addr,
            "GET /stream-error-cleanup-e2e/check HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        if check_response.contains(r#""errorCount":1"#) || tokio::time::Instant::now() >= deadline {
            break check_response;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(
        check_response.contains(r#""errorCount":1"#)
            && check_response.contains(r#""activeStreams":0"#)
            && check_response.contains(r#""pendingCancels":0"#),
        "expected producer error cleanup to clear stream registries: {check_response}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_streaming_client_abort_cancels_producer_and_releases_route() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;
    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let stream_eszip = build_eszip_async(
        "file:///stream_abort_cleanup_e2e.ts",
        r#"
        let cancelCount = 0;
        Deno.serve((req) => {
          const path = new URL(req.url).pathname;
          if (path === '/stream') {
            const encoder = new TextEncoder();
            return new Response(new ReadableStream({
              start(controller) {
                controller.enqueue(encoder.encode('first-chunk'));
              },
              cancel() {
                cancelCount += 1;
              },
            }), {
              headers: { 'content-type': 'text/plain' },
            });
          }

          return new Response(JSON.stringify({
            cancelCount,
            activeStreams: globalThis.__edgeRuntime._streamExecutions.size,
            pendingCancels: globalThis.__edgeRuntime._responseStreamCancels.size,
          }), {
            headers: { 'content-type': 'application/json' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "stream-abort-cleanup-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy streaming abort test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_handle = tokio::spawn({
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        async move { run_dual_server(server_config, registry, shutdown).await }
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    for expected_cancel_count in 1..=8 {
        let partial_response = send_plain_http_and_abort_after_body_prefix(
            ingress_addr,
            "GET /stream-abort-cleanup-e2e/stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            partial_response.starts_with("HTTP/1.1 200"),
            "expected streaming response before client abort: {partial_response}"
        );
        assert!(
            partial_response.contains("first-chunk"),
            "expected first chunk before client abort: {partial_response}"
        );

        let expected_count = format!(r#""cancelCount":{expected_cancel_count}"#);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let check_response = loop {
            let check_response = send_plain_http(
                ingress_addr,
                "GET /stream-abort-cleanup-e2e/check HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await;
            if check_response.contains(&expected_count) || tokio::time::Instant::now() >= deadline {
                break check_response;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };

        assert!(
            check_response.starts_with("HTTP/1.1 200"),
            "expected route target to be released after abort: {check_response}"
        );
        assert!(
            check_response.contains(&expected_count),
            "expected producer cancellation count {expected_cancel_count}: {check_response}"
        );
        assert!(
            check_response.contains(r#""activeStreams":0"#)
                && check_response.contains(r#""pendingCancels":0"#),
            "expected execution stream registries to be empty after abort: {check_response}"
        );
    }

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_legacy_router_streaming_abort_cleans_up_once() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get probe address");
    drop(probe_listener);

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();
    let stream_eszip = build_eszip_async(
        "file:///legacy_stream_abort_cleanup_e2e.ts",
        r#"
        let cancelCount = 0;
        Deno.serve((req) => {
          const path = new URL(req.url).pathname;
          if (path === '/stream') {
            const encoder = new TextEncoder();
            return new Response(new ReadableStream({
              start(controller) {
                controller.enqueue(encoder.encode('legacy-first-chunk'));
              },
              cancel() {
                cancelCount += 1;
              },
            }), {
              headers: { 'content-type': 'text/plain' },
            });
          }

          return new Response(JSON.stringify({
            cancelCount,
            activeStreams: globalThis.__edgeRuntime._streamExecutions.size,
            pendingCancels: globalThis.__edgeRuntime._responseStreamCancels.size,
          }), {
            headers: { 'content-type': 'application/json' },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");
    registry
        .deploy(
            "legacy-stream-abort-cleanup-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy legacy streaming abort test function");

    let server_config = ServerConfig {
        addr,
        tls: None,
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig::default(),
        max_connections: 128,
    };
    let server_handle = tokio::spawn({
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        async move { run_server(server_config, registry, shutdown).await }
    });

    wait_for_tcp_listener(addr).await;

    let partial_response = send_plain_http_and_abort_after_body_prefix(
        addr,
        "GET /legacy-stream-abort-cleanup-e2e/stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        partial_response.starts_with("HTTP/1.1 200")
            && partial_response.contains("legacy-first-chunk"),
        "expected legacy router to send first chunk before abort: {partial_response}"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let check_response = loop {
        let check_response = send_plain_http(
            addr,
            "GET /legacy-stream-abort-cleanup-e2e/check HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        if check_response.contains(r#""cancelCount":1"#) || tokio::time::Instant::now() >= deadline
        {
            break check_response;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(
        check_response.contains(r#""cancelCount":1"#)
            && check_response.contains(r#""activeStreams":0"#)
            && check_response.contains(r#""pendingCancels":0"#),
        "expected legacy router abort cleanup exactly once: {check_response}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_legacy_router_streaming_limit_cancels_producer() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get probe address");
    drop(probe_listener);

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();
    let stream_eszip = build_eszip_async(
        "file:///legacy_stream_limit_e2e.ts",
        r#"
        let cancelCount = 0;
        Deno.serve((req) => {
          if (new URL(req.url).pathname === '/check') {
            return new Response(cancelCount === 1 ? 'cancelled' : 'pending');
          }

          const encoder = new TextEncoder();
          return new Response(new ReadableStream({
            start(controller) {
              controller.enqueue(encoder.encode('abcdefghij'));
              controller.enqueue(encoder.encode('klmnopqrst' + 'z'.repeat(100)));
            },
            cancel() {
              cancelCount += 1;
            },
          }), {
            headers: {
              'content-length': '110',
              'content-type': 'text/plain',
            },
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(stream_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize legacy limit bundle");
    registry
        .deploy(
            "legacy-stream-limit-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy legacy stream limit function");

    let server_config = ServerConfig {
        addr,
        tls: None,
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 15,
        },
        max_connections: 128,
    };
    let server_handle = tokio::spawn({
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        async move { run_server(server_config, registry, shutdown).await }
    });

    wait_for_tcp_listener(addr).await;
    let response = send_plain_http(
        addr,
        "GET /legacy-stream-limit-e2e/stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected legacy limited stream to preserve status: {response}"
    );
    assert!(
        response.contains("abcdefghij") && response.contains("klmno"),
        "expected legacy router to forward only the allowed prefix: {response}"
    );
    assert!(
        !response.contains("klmnop")
            && !response
                .to_ascii_lowercase()
                .contains("content-length: 110"),
        "expected legacy router to truncate and remove stale content length: {response}"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let check_response = loop {
        let check_response = send_plain_http(
            addr,
            "GET /legacy-stream-limit-e2e/check HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        if check_response.contains(r#""cancelCount":1"#) || tokio::time::Instant::now() >= deadline
        {
            break check_response;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(
        check_response.contains("cancelled"),
        "expected legacy limit to cancel the producer exactly once: {check_response}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_ingress_preserves_http_header_semantics_on_rewrite() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let semantics_eszip = build_eszip_async(
        "file:///http_semantics_e2e.ts",
        r#"
        Deno.serve((req) => {
          const acceptEncoding = req.headers.get('accept-encoding') || '';
          const headers = new Headers();
          headers.append('set-cookie', 'a=1; Path=/; HttpOnly');
          headers.append('set-cookie', 'b=2; Path=/; Secure');
          headers.set('content-encoding', 'gzip');
          headers.set('x-accept-encoding', acceptEncoding);
          return new Response('semantics-ok', { status: 200, headers });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(semantics_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "http-semantics-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy http semantics test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let raw_response = send_plain_http(
        ingress_addr,
        "GET /http-semantics-e2e HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip, br\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(
        raw_response.starts_with("HTTP/1.1 200"),
        "expected 200 response: {raw_response}"
    );

    let response_lower = raw_response.to_ascii_lowercase();
    assert!(
        response_lower.contains("content-encoding: gzip"),
        "expected content-encoding header preserved: {raw_response}"
    );
    assert!(
        response_lower.contains("x-accept-encoding: gzip, br"),
        "expected accept-encoding forwarded through rewrite path: {raw_response}"
    );

    let set_cookie_count = response_lower.matches("set-cookie:").count();
    assert_eq!(
        set_cookie_count, 2,
        "expected two independent set-cookie headers without flattening: {raw_response}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_async_local_storage_isolated_between_overlapping_requests() {
    init_deno_platform();

    let (admin_addr, ingress_addr) = reserve_dual_listener_addrs().await;

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let als_eszip = build_eszip_async(
        "file:///als_e2e.ts",
        r#"
        import { AsyncLocalStorage } from 'node:async_hooks';

        const als = new AsyncLocalStorage();

        Deno.serve(async (req) => {
                        const requestId = req.headers.get('x-req-id') ?? 'missing';
                        const delayMs = Number(req.headers.get('x-delay-ms') ?? '15');

          return await als.run(requestId, async () => {
            await new Promise((resolve) => setTimeout(resolve, delayMs));
            const afterTimer = als.getStore();
            await Promise.resolve();
            const afterPromise = als.getStore();

            if (afterTimer !== requestId || afterPromise !== requestId) {
              return new Response(
                `leak:${requestId}:${String(afterTimer)}:${String(afterPromise)}`,
                { status: 500, headers: { 'content-type': 'text/plain' } },
              );
            }

            return new Response(
              `ok:${requestId}:${String(afterTimer)}:${String(afterPromise)}`,
              { headers: { 'content-type': 'text/plain' } },
            );
          });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(als_eszip);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "als-e2e".to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .expect("failed to deploy async context test function");

    let server_config = DualServerConfig {
        admin: AdminListenerConfig {
            addr: admin_addr,
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(ingress_addr),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let registry_for_server = registry.clone();
    let server_handle = tokio::spawn(async move {
        run_dual_server(server_config, registry_for_server, server_shutdown).await
    });

    wait_for_tcp_listener(admin_addr).await;
    wait_for_tcp_listener(ingress_addr).await;

    let warmup_resp = send_plain_http(
        ingress_addr,
        "GET /als-e2e HTTP/1.1\r\nHost: localhost\r\nx-req-id: warmup\r\nx-delay-ms: 1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        warmup_resp.starts_with("HTTP/1.1 200"),
        "warmup request failed: {warmup_resp}"
    );

    let req_a = send_plain_http(
        ingress_addr,
        "GET /als-e2e HTTP/1.1\r\nHost: localhost\r\nx-req-id: alpha\r\nx-delay-ms: 45\r\nConnection: close\r\n\r\n",
    );
    let req_b = send_plain_http(
        ingress_addr,
        "GET /als-e2e HTTP/1.1\r\nHost: localhost\r\nx-req-id: beta\r\nx-delay-ms: 5\r\nConnection: close\r\n\r\n",
    );
    let (resp_a, resp_b) = tokio::join!(req_a, req_b);

    assert!(
        resp_a.starts_with("HTTP/1.1 200"),
        "alpha response failed: {resp_a}"
    );
    assert!(
        resp_b.starts_with("HTTP/1.1 200"),
        "beta response failed: {resp_b}"
    );
    assert!(
        resp_a.contains("ok:alpha:alpha:alpha"),
        "expected alpha ALS context to remain isolated: {resp_a}"
    );
    assert!(
        resp_b.contains("ok:beta:beta:beta"),
        "expected beta ALS context to remain isolated: {resp_b}"
    );
    assert!(
        !resp_a.contains("ok:alpha:beta:beta") && !resp_b.contains("ok:beta:alpha:alpha"),
        "unexpected cross-request context contamination detected"
    );

    let post_resp = send_plain_http(
        ingress_addr,
        "GET /als-e2e HTTP/1.1\r\nHost: localhost\r\nx-req-id: gamma\r\nx-delay-ms: 1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        post_resp.contains("ok:gamma:gamma:gamma"),
        "expected no stale context leak after overlapping requests: {post_resp}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_context_saturation_returns_503_and_reports_routing_metrics() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get local addr");
    drop(probe_listener);

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let eszip_bytes = build_eszip_async(
        "file:///context_saturation.ts",
        r#"
        Deno.serve(async (_req) => {
                        await new Promise((resolve) => setTimeout(resolve, 800));
          return new Response('ok', { headers: { 'content-type': 'text/plain' } });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(eszip_bytes);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "ctx-slo".to_string(),
            bytes::Bytes::from(bundle_data),
            Some(IsolateConfig {
                context_pool_enabled: true,
                max_contexts_per_isolate: 1,
                max_active_requests_per_context: 1,
                ..IsolateConfig::default()
            }),
            None,
        )
        .await
        .expect("failed to deploy ctx-slo function");

    let server_config = ServerConfig {
        addr,
        tls: None,
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig::default(),
        max_connections: 128,
    };

    let server_shutdown = shutdown.clone();
    let server_handle =
        tokio::spawn(async move { run_server(server_config, registry, server_shutdown).await });

    wait_for_tcp_listener(addr).await;

    let mut in_flight = TcpStream::connect(addr)
        .await
        .expect("failed to connect in-flight request");
    in_flight
        .write_all(b"GET /ctx-slo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("failed to write in-flight request");

    wait_for_routing_saturation(addr).await;

    let saturated = send_plain_http(
        addr,
        "GET /ctx-slo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        saturated.starts_with("HTTP/1.1 503"),
        "expected 503 while context is saturated, got: {saturated}"
    );
    assert!(
        saturated.contains("capacity exhausted"),
        "expected capacity exhausted payload, got: {saturated}"
    );

    let metrics_resp = send_plain_http(
        addr,
        "GET /_internal/metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        metrics_resp.starts_with("HTTP/1.1 200"),
        "expected 200 from metrics endpoint, got: {metrics_resp}"
    );

    let metrics = parse_http_json_body(&metrics_resp);
    let routing = metrics
        .get("routing")
        .unwrap_or_else(|| panic!("missing routing field in metrics: {metrics}"));
    assert!(
        routing
            .get("saturated_contexts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 1,
        "expected saturated contexts while request is in flight: {metrics}"
    );
    assert!(
        routing
            .get("saturated_isolates")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 1,
        "expected saturated isolates while request is in flight: {metrics}"
    );

    let mut completed_buf = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        in_flight.read_to_end(&mut completed_buf),
    )
    .await
    .expect("timed out waiting in-flight response")
    .expect("failed to read in-flight response");
    let completed = String::from_utf8_lossy(&completed_buf).to_string();
    assert!(
        completed.starts_with("HTTP/1.1 200"),
        "expected first request to complete with 200, got: {completed}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
#[ignore = "chaos test: high concurrency burst with pool scaling and deterministic status validation"]
async fn chaos_context_isolate_burst_keeps_statuses_deterministic() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get local addr");
    drop(probe_listener);

    let registry = make_pool_enabled_registry();
    let shutdown = CancellationToken::new();

    let eszip_bytes = build_eszip_async(
        "file:///context_chaos.ts",
        r#"
        Deno.serve(async (_req) => {
          await new Promise((resolve) => setTimeout(resolve, 25));
          return new Response('ok-chaos', { headers: { 'content-type': 'text/plain' } });
        });
        "#,
    )
    .await;
    let bundle = BundlePackage::eszip_only(eszip_bytes);
    let bundle_data = bincode::serialize(&bundle).expect("failed to serialize bundle");

    registry
        .deploy(
            "ctx-chaos".to_string(),
            bytes::Bytes::from(bundle_data),
            Some(IsolateConfig {
                context_pool_enabled: true,
                max_contexts_per_isolate: 2,
                max_active_requests_per_context: 1,
                ..IsolateConfig::default()
            }),
            None,
        )
        .await
        .expect("failed to deploy ctx-chaos");

    registry
        .set_pool_limits("ctx-chaos", 1, 4)
        .await
        .expect("failed to set pool limits for chaos test");

    let server_config = ServerConfig {
        addr,
        tls: None,
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig::default(),
        max_connections: 512,
    };

    let server_shutdown = shutdown.clone();
    let server_registry = registry.clone();
    let server_handle =
        tokio::spawn(
            async move { run_server(server_config, server_registry, server_shutdown).await },
        );

    wait_for_tcp_listener(addr).await;

    let mut tasks = Vec::with_capacity(96);
    for _ in 0..96 {
        tasks.push(tokio::spawn(send_plain_http(
            addr,
            "GET /ctx-chaos HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )));
    }

    let mut ok_200 = 0_u64;
    let mut ok_503 = 0_u64;

    for task in tasks {
        let response = task.await.expect("join failed for chaos request");
        if response.starts_with("HTTP/1.1 200") {
            ok_200 = ok_200.saturating_add(1);
        } else if response.starts_with("HTTP/1.1 503") {
            ok_503 = ok_503.saturating_add(1);
        } else {
            panic!("unexpected response status under chaos burst: {response}");
        }
    }

    assert!(ok_200 > 0, "expected at least some successful responses");
    assert!(
        ok_200 + ok_503 == 96,
        "unexpected status distribution: ok_200={ok_200}, ok_503={ok_503}"
    );

    let metrics_resp = send_plain_http(
        addr,
        "GET /_internal/metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    let metrics = parse_http_json_body(&metrics_resp);
    let total_isolates = metrics
        .get("routing")
        .and_then(|routing| routing.get("total_isolates"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        (1..=4).contains(&total_isolates),
        "expected isolate count to stay within configured pool limits, got: {total_isolates}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
async fn e2e_connection_limit_drops_excess_connections() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get local addr");
    drop(probe_listener);

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let server_config = ServerConfig {
        addr,
        tls: None,
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig::default(),
        max_connections: 1,
    };

    let server_shutdown = shutdown.clone();
    let server_handle =
        tokio::spawn(async move { run_server(server_config, registry, server_shutdown).await });

    wait_for_tcp_listener(addr).await;

    // Occupy the only permit with a slow request.
    let mut held = TcpStream::connect(addr)
        .await
        .expect("failed to connect held request");
    held.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .expect("failed to write partial request");

    // Second connection should be dropped by connection limiter.
    let mut second = TcpStream::connect(addr)
        .await
        .expect("failed to connect second request");
    second
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("failed to write second request");
    let mut buf = [0_u8; 128];
    let second_read = tokio::time::timeout(
        ACCEPT_PERMIT_WAIT + Duration::from_millis(700),
        second.read(&mut buf),
    )
    .await
    .expect("timed out reading second connection");
    match second_read {
        Ok(0) => {
            // EOF: server closed socket immediately.
        }
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => {
            // Also valid: kernel reset because server dropped connection abruptly.
        }
        Ok(n) => {
            panic!("expected dropped second connection, received {n} bytes");
        }
        Err(err) => {
            panic!("unexpected second connection read error: {err}");
        }
    }

    // Finish first request so server can respond.
    held.write_all(b"Connection: close\r\n\r\n")
        .await
        .expect("failed to complete held request");
    let mut held_response = Vec::new();
    let held_n = tokio::time::timeout(Duration::from_secs(2), held.read_to_end(&mut held_response))
        .await
        .expect("timed out waiting held response")
        .expect("failed to read held response");
    assert!(held_n > 0, "expected held request to eventually complete");

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}

#[tokio::test]
#[ignore = "stress test: opens 20k connections and may exceed local CI resource limits"]
async fn stress_20k_connections_excess_are_dropped() {
    init_deno_platform();

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind probe listener");
    let addr = probe_listener
        .local_addr()
        .expect("failed to get local addr");
    drop(probe_listener);

    let registry = make_test_registry();
    let shutdown = CancellationToken::new();

    let server_config = ServerConfig {
        addr,
        tls: None,
        rate_limit_rps: None,
        graceful_exit_deadline_secs: 1,
        body_limits: BodyLimitsConfig::default(),
        max_connections: 10_000,
    };

    let server_shutdown = shutdown.clone();
    let server_handle =
        tokio::spawn(async move { run_server(server_config, registry, server_shutdown).await });

    wait_for_tcp_listener(addr).await;

    let mut tasks = Vec::with_capacity(20_000);
    for _ in 0..20_000usize {
        tasks.push(tokio::spawn(async move {
            match TcpStream::connect(addr).await {
                Ok(mut stream) => {
                    let _ = stream
                        .write_all(
                            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let mut buf = [0_u8; 64];
                    let _ = tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf))
                        .await;
                    true
                }
                Err(_) => false,
            }
        }));
    }

    let mut connected = 0usize;
    for task in tasks {
        if task.await.expect("join failed") {
            connected += 1;
        }
    }

    assert!(connected > 0, "at least some connections should succeed");

    // After stress, server should still answer.
    let probe = send_plain_http(
        addr,
        "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        probe.starts_with("HTTP/1.1"),
        "server did not respond after stress: {probe}"
    );

    shutdown.cancel();
    let server_result = tokio::time::timeout(Duration::from_secs(3), server_handle)
        .await
        .expect("server task did not finish in time")
        .expect("server join error");
    server_result.expect("server returned error");
}
