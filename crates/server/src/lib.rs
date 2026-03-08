pub mod admin_router;
pub mod body_limits;
pub mod bundle_signature;
pub mod graceful;
pub mod ingress_router;
pub mod middleware;
pub mod router;
pub mod service;
pub mod tls;
pub mod trace_context;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use functions::registry::FunctionRegistry;

use crate::admin_router::AdminRouter;
use crate::bundle_signature::{BundleSignatureConfig, BundleSignatureVerifier};
use crate::ingress_router::IngressRouter;
use crate::service::EdgeService;

// Re-export for convenience
pub use crate::body_limits::BodyLimitsConfig;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration Types
// ─────────────────────────────────────────────────────────────────────────────

/// Dual-listener server configuration.
#[derive(Debug, Clone)]
pub struct DualServerConfig {
    pub admin: AdminListenerConfig,
    pub ingress: IngressListenerConfig,
    pub graceful_exit_deadline_secs: u64,
    /// Maximum concurrent connections across all listeners.
    pub max_connections: usize,
}

/// Admin listener configuration (TCP only).
#[derive(Debug, Clone)]
pub struct AdminListenerConfig {
    /// Address to bind (default: 0.0.0.0:9000)
    pub addr: SocketAddr,
    /// API key for authentication. None = no auth (dev mode).
    pub api_key: Option<String>,
    /// Optional TLS configuration.
    pub tls: Option<TlsConfig>,
    /// Body size limits.
    pub body_limits: BodyLimitsConfig,
    /// Bundle signature verification policy for deploy/update endpoints.
    pub bundle_signature: BundleSignatureConfig,
}

/// Ingress listener configuration (TCP or Unix socket).
#[derive(Debug, Clone)]
pub struct IngressListenerConfig {
    /// Listener type: TCP or Unix socket.
    pub listener_type: IngressListenerType,
    /// Optional TLS (only for TCP).
    pub tls: Option<TlsConfig>,
    /// Rate limit in requests per second.
    pub rate_limit_rps: Option<u64>,
    /// Body size limits.
    pub body_limits: BodyLimitsConfig,
}

/// Ingress listener type.
#[derive(Debug, Clone)]
pub enum IngressListenerType {
    /// TCP socket with address.
    Tcp(SocketAddr),
    /// Unix domain socket with path.
    Unix(PathBuf),
}

/// Legacy server configuration (single listener).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub tls: Option<TlsConfig>,
    pub rate_limit_rps: Option<u64>,
    pub graceful_exit_deadline_secs: u64,
    /// Body size limits.
    pub body_limits: BodyLimitsConfig,
    /// Maximum concurrent connections.
    pub max_connections: usize,
}

/// TLS configuration.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Dual Server (New Architecture)
// ─────────────────────────────────────────────────────────────────────────────

/// Start the dual-listener HTTP server.
///
/// - Admin listener on `config.admin.addr` (default port 9000) with API key auth
/// - Ingress listener on TCP port or Unix socket for function requests
pub async fn run_dual_server(
    config: DualServerConfig,
    registry: Arc<FunctionRegistry>,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    // Warn if no API key configured
    if config.admin.api_key.is_none() {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "admin API running without authentication (no --api-key set). \
             This is insecure for production use."
        );
    }

    // Create connection semaphore shared across all listeners
    let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));
    info!(
        function_name = "runtime",
        request_id = "system",
        "connection limit set to {} concurrent connections",
        config.max_connections
    );

    // Create routers with shared registry and body limits
    let admin_router = AdminRouter::new(
        registry.clone(),
        config.admin.api_key.clone(),
        config.admin.body_limits,
        BundleSignatureVerifier::from_config(config.admin.bundle_signature.clone())?,
    );
    let ingress_router = IngressRouter::new(
        registry.clone(),
        config.ingress.body_limits,
        config.ingress.rate_limit_rps,
    );

    // Spawn admin listener
    let admin_shutdown = shutdown.clone();
    let admin_config = config.admin.clone();
    let admin_semaphore = connection_semaphore.clone();
    let admin_handle = tokio::spawn(async move {
        if let Err(e) =
            run_admin_listener(admin_config, admin_router, admin_shutdown, admin_semaphore).await
        {
            error!(
                function_name = "runtime",
                request_id = "system",
                "admin listener error: {}",
                e
            );
        }
    });

    // Spawn ingress listener
    let ingress_shutdown = shutdown.clone();
    let ingress_config = config.ingress.clone();
    let ingress_semaphore = connection_semaphore.clone();
    let ingress_handle = tokio::spawn(async move {
        if let Err(e) = run_ingress_listener(
            ingress_config,
            ingress_router,
            ingress_shutdown,
            ingress_semaphore,
        )
        .await
        {
            error!(
                function_name = "runtime",
                request_id = "system",
                "ingress listener error: {}",
                e
            );
        }
    });

    // Wait for shutdown signal
    shutdown.cancelled().await;
    info!(
        function_name = "runtime",
        request_id = "system",
        "shutdown signal received, stopping listeners..."
    );

    // Wait for listeners to finish with deadline
    let deadline = Duration::from_secs(config.graceful_exit_deadline_secs);
    let _ = tokio::time::timeout(deadline, async {
        let _ = admin_handle.await;
        let _ = ingress_handle.await;
    })
    .await;

    info!(
        function_name = "runtime",
        request_id = "system",
        "waited up to {}s for connections to drain",
        config.graceful_exit_deadline_secs
    );

    registry
        .shutdown_all_with_deadline(Duration::from_secs(config.graceful_exit_deadline_secs))
        .await;

    Ok(())
}

/// Run the admin listener (TCP only, with optional TLS).
async fn run_admin_listener(
    config: AdminListenerConfig,
    router: AdminRouter,
    shutdown: CancellationToken,
    connection_semaphore: Arc<Semaphore>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(config.addr).await?;

    let tls_acceptor = if let Some(ref tls_config) = config.tls {
        Some(tls::build_dynamic_tls_acceptor(
            tls_config.clone(),
            shutdown.clone(),
            "admin listener",
        )?)
    } else {
        None
    };

    if tls_acceptor.is_none() {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "admin listener started without TLS on {}. Traffic is unencrypted.",
            config.addr
        );
    }

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    info!(
        function_name = "runtime",
        request_id = "system",
        "admin API listening on {}://{}",
        scheme,
        config.addr
    );

    let svc = EdgeService::new(router);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer_addr)) => {
                        // Try to acquire connection permit
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(function_name = "runtime", request_id = "system", "admin: connection limit reached, rejecting {}", peer_addr);
                                drop(stream);
                                continue;
                            }
                        };

                        let svc = svc.clone();
                        let tls_acceptor = tls_acceptor.clone();
                        tokio::spawn(async move {
                            // Permit is held for the duration of this task
                            let _permit = permit;

                            let maybe_stream = if let Some(acceptor) = tls_acceptor {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => tls::MaybeHttpsStream::TcpTls(tls_stream),
                                    Err(e) => {
                                        tracing::warn!("admin TLS handshake failed from {}: {}", peer_addr, e);
                                        return;
                                    }
                                }
                            } else {
                                tls::MaybeHttpsStream::TcpPlain(stream)
                            };

                            let io = TokioIo::new(maybe_stream);
                            let conn = hyper_util::server::conn::auto::Builder::new(
                                hyper_util::rt::TokioExecutor::new(),
                            );
                            if let Err(e) = conn.serve_connection(io, svc).await {
                                tracing::debug!("admin connection error from {}: {}", peer_addr, e);
                            }
                        });
                    }
                    Err(e) => {
                        error!(function_name = "runtime", request_id = "system", "admin accept error: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!(function_name = "runtime", request_id = "system", "admin listener stopping");
                break;
            }
        }
    }

    Ok(())
}

/// Run the ingress listener (TCP or Unix socket).
async fn run_ingress_listener(
    config: IngressListenerConfig,
    router: IngressRouter,
    shutdown: CancellationToken,
    connection_semaphore: Arc<Semaphore>,
) -> Result<(), Error> {
    match config.listener_type {
        IngressListenerType::Tcp(addr) => {
            run_tcp_ingress(addr, config.tls, router, shutdown, connection_semaphore).await
        }
        IngressListenerType::Unix(path) => {
            run_unix_ingress(path, router, shutdown, connection_semaphore).await
        }
    }
}

/// Run ingress on TCP socket.
async fn run_tcp_ingress(
    addr: SocketAddr,
    tls_config: Option<TlsConfig>,
    router: IngressRouter,
    shutdown: CancellationToken,
    connection_semaphore: Arc<Semaphore>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(addr).await?;

    let tls_acceptor = if let Some(ref tls) = tls_config {
        Some(tls::build_dynamic_tls_acceptor(
            tls.clone(),
            shutdown.clone(),
            "ingress listener",
        )?)
    } else {
        None
    };

    if tls_acceptor.is_none() {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "ingress listener started without TLS on {}. Traffic is unencrypted.",
            addr
        );
    }

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    info!(
        function_name = "runtime",
        request_id = "system",
        "ingress listening on {}://{}",
        scheme,
        addr
    );

    let svc = EdgeService::new(router);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer_addr)) => {
                        // Try to acquire connection permit
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(function_name = "runtime", request_id = "system", "ingress: connection limit reached, rejecting {}", peer_addr);
                                drop(stream);
                                continue;
                            }
                        };

                        let svc = svc.clone();
                        let tls_acceptor = tls_acceptor.clone();
                        tokio::spawn(async move {
                            // Permit is held for the duration of this task
                            let _permit = permit;

                            let maybe_stream = if let Some(acceptor) = tls_acceptor {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => tls::MaybeHttpsStream::TcpTls(tls_stream),
                                    Err(e) => {
                                        tracing::warn!("ingress TLS handshake failed from {}: {}", peer_addr, e);
                                        return;
                                    }
                                }
                            } else {
                                tls::MaybeHttpsStream::TcpPlain(stream)
                            };

                            let io = TokioIo::new(maybe_stream);
                            let conn = hyper_util::server::conn::auto::Builder::new(
                                hyper_util::rt::TokioExecutor::new(),
                            );
                            if let Err(e) = conn.serve_connection(io, svc).await {
                                tracing::debug!("ingress connection error from {}: {}", peer_addr, e);
                            }
                        });
                    }
                    Err(e) => {
                        error!(function_name = "runtime", request_id = "system", "ingress accept error: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!(function_name = "runtime", request_id = "system", "ingress TCP listener stopping");
                break;
            }
        }
    }

    Ok(())
}

/// Run ingress on Unix socket.
async fn run_unix_ingress(
    path: PathBuf,
    router: IngressRouter,
    shutdown: CancellationToken,
    connection_semaphore: Arc<Semaphore>,
) -> Result<(), Error> {
    // Clean up stale socket file if exists
    if path.exists() {
        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    info!(
        function_name = "runtime",
        request_id = "system",
        "ingress listening on unix:{}",
        path.display()
    );

    let svc = EdgeService::new(router);
    let cleanup_path = path.clone();

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        // Try to acquire connection permit
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(function_name = "runtime", request_id = "system", "unix ingress: connection limit reached, rejecting connection");
                                drop(stream);
                                continue;
                            }
                        };

                        let svc = svc.clone();
                        tokio::spawn(async move {
                            // Permit is held for the duration of this task
                            let _permit = permit;

                            let maybe_stream = tls::MaybeHttpsStream::Unix(stream);
                            let io = TokioIo::new(maybe_stream);
                            let conn = hyper_util::server::conn::auto::Builder::new(
                                hyper_util::rt::TokioExecutor::new(),
                            );
                            if let Err(e) = conn.serve_connection(io, svc).await {
                                tracing::debug!("unix connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!(function_name = "runtime", request_id = "system", "unix accept error: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!(function_name = "runtime", request_id = "system", "ingress Unix listener stopping");
                break;
            }
        }
    }

    // Cleanup socket file
    if let Err(e) = std::fs::remove_file(&cleanup_path) {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "failed to remove Unix socket {}: {}",
            cleanup_path.display(),
            e
        );
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy Single Server (Backward Compatibility)
// ─────────────────────────────────────────────────────────────────────────────

/// Start the HTTP server and block until shutdown.
///
/// This is the legacy single-listener interface. For new deployments,
/// use `run_dual_server` instead.
pub async fn run_server(
    config: ServerConfig,
    registry: Arc<FunctionRegistry>,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    let router = router::Router::new(registry.clone(), config.body_limits, config.rate_limit_rps);
    let svc = service::EdgeService::new(router);

    let listener = TcpListener::bind(config.addr).await?;

    // Create connection semaphore
    let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));
    info!(
        function_name = "runtime",
        request_id = "system",
        "connection limit set to {} concurrent connections",
        config.max_connections
    );

    // Optional TLS acceptor
    let tls_acceptor = if let Some(ref tls_config) = config.tls {
        Some(tls::build_dynamic_tls_acceptor(
            tls_config.clone(),
            shutdown.clone(),
            "legacy listener",
        )?)
    } else {
        None
    };

    if tls_acceptor.is_none() {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "server started without TLS on {}. Traffic is unencrypted.",
            config.addr
        );
    }

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    info!(
        function_name = "runtime",
        request_id = "system",
        "thunder listening on {}://{}",
        scheme,
        config.addr
    );

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer_addr)) => {
                        // Try to acquire connection permit
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(function_name = "runtime", request_id = "system", "connection limit reached, rejecting {}", peer_addr);
                                drop(stream);
                                continue;
                            }
                        };

                        let svc = svc.clone();
                        let tls_acceptor = tls_acceptor.clone();
                        tokio::spawn(async move {
                            // Permit is held for the duration of this task
                            let _permit = permit;

                            let maybe_stream = if let Some(acceptor) = tls_acceptor {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => tls::MaybeHttpsStream::TcpTls(tls_stream),
                                    Err(e) => {
                                        tracing::warn!("TLS handshake failed from {}: {}", peer_addr, e);
                                        return;
                                    }
                                }
                            } else {
                                tls::MaybeHttpsStream::TcpPlain(stream)
                            };

                            let io = TokioIo::new(maybe_stream);
                            let conn = hyper_util::server::conn::auto::Builder::new(
                                hyper_util::rt::TokioExecutor::new(),
                            );
                            if let Err(e) = conn.serve_connection(io, svc).await {
                                // Connection errors are normal (client disconnects, etc.)
                                tracing::debug!("connection error from {}: {}", peer_addr, e);
                            }
                        });
                    }
                    Err(e) => {
                        error!(function_name = "runtime", request_id = "system", "failed to accept connection: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!(function_name = "runtime", request_id = "system", "shutdown signal received, stopping server...");
                break;
            }
        }
    }

    // Graceful shutdown: wait for in-flight connections
    info!(
        function_name = "runtime",
        request_id = "system",
        "waiting up to {}s for connections to drain",
        config.graceful_exit_deadline_secs
    );
    tokio::time::sleep(std::time::Duration::from_secs(
        config.graceful_exit_deadline_secs,
    ))
    .await;

    registry
        .shutdown_all_with_deadline(Duration::from_secs(config.graceful_exit_deadline_secs))
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
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
                outgoing_proxy: runtime_core::isolate::OutgoingProxyConfig::default(),
            },
            PoolLimits::default(),
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
        serde_json::from_str(body).unwrap_or_else(|err| {
            panic!("failed to parse response body as json: {err}; body={body}")
        })
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

        let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind admin probe listener");
        let admin_addr = admin_probe
            .local_addr()
            .expect("failed to get admin local addr");
        drop(admin_probe);

        let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ingress probe listener");
        let ingress_addr = ingress_probe
            .local_addr()
            .expect("failed to get ingress local addr");
        drop(ingress_probe);

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

        let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind admin probe listener");
        let admin_addr = admin_probe
            .local_addr()
            .expect("failed to get admin local addr");
        drop(admin_probe);

        let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ingress probe listener");
        let ingress_addr = ingress_probe
            .local_addr()
            .expect("failed to get ingress local addr");
        drop(ingress_probe);

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
    async fn e2e_ingress_streaming_returns_progressive_chunked_body() {
        init_deno_platform();

        let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind admin probe listener");
        let admin_addr = admin_probe
            .local_addr()
            .expect("failed to get admin local addr");
        drop(admin_probe);

        let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ingress probe listener");
        let ingress_addr = ingress_probe
            .local_addr()
            .expect("failed to get ingress local addr");
        drop(ingress_probe);

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
    async fn e2e_ingress_streaming_long_chunked_body_completes() {
        init_deno_platform();

        let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind admin probe listener");
        let admin_addr = admin_probe
            .local_addr()
            .expect("failed to get admin local addr");
        drop(admin_probe);

        let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ingress probe listener");
        let ingress_addr = ingress_probe
            .local_addr()
            .expect("failed to get ingress local addr");
        drop(ingress_probe);

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
            .write_all(
                b"GET /stream-long-e2e HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
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
    async fn e2e_ingress_preserves_http_header_semantics_on_rewrite() {
        init_deno_platform();

        let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind admin probe listener");
        let admin_addr = admin_probe
            .local_addr()
            .expect("failed to get admin local addr");
        drop(admin_probe);

        let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ingress probe listener");
        let ingress_addr = ingress_probe
            .local_addr()
            .expect("failed to get ingress local addr");
        drop(ingress_probe);

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

        let admin_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind admin probe listener");
        let admin_addr = admin_probe
            .local_addr()
            .expect("failed to get admin local addr");
        drop(admin_probe);

        let ingress_probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind ingress probe listener");
        let ingress_addr = ingress_probe
            .local_addr()
            .expect("failed to get ingress local addr");
        drop(ingress_probe);

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
              await new Promise((resolve) => setTimeout(resolve, 120));
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

        let in_flight = tokio::spawn(send_plain_http(
            addr,
            "GET /ctx-slo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        ));

        tokio::time::sleep(Duration::from_millis(25)).await;

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
                .get("saturated_rejections")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                >= 1,
            "expected at least one saturated rejection: {metrics}"
        );
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

        let completed = in_flight.await.expect("join failure for in-flight request");
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
        let server_handle = tokio::spawn(async move {
            run_server(server_config, server_registry, server_shutdown).await
        });

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
            total_isolates >= 1 && total_isolates <= 4,
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
        let second_read = tokio::time::timeout(Duration::from_millis(300), second.read(&mut buf))
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
        let held_n =
            tokio::time::timeout(Duration::from_secs(2), held.read_to_end(&mut held_response))
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
                        let _ =
                            tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf))
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
}
