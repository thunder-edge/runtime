pub mod admin_router;
pub mod body_limits;
pub mod graceful;
pub mod ingress_router;
pub mod middleware;
pub mod router;
pub mod service;
pub mod tls;

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
            "admin API running without authentication (no --api-key set). \
             This is insecure for production use."
        );
    }

    // Create connection semaphore shared across all listeners
    let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));
    info!(
        "connection limit set to {} concurrent connections",
        config.max_connections
    );

    // Create routers with shared registry and body limits
    let admin_router = AdminRouter::new(
        registry.clone(),
        config.admin.api_key.clone(),
        config.admin.body_limits,
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
            error!("admin listener error: {}", e);
        }
    });

    // Spawn ingress listener
    let ingress_shutdown = shutdown.clone();
    let ingress_config = config.ingress.clone();
    let ingress_semaphore = connection_semaphore.clone();
    let ingress_handle = tokio::spawn(async move {
        if let Err(e) =
            run_ingress_listener(ingress_config, ingress_router, ingress_shutdown, ingress_semaphore)
                .await
        {
            error!("ingress listener error: {}", e);
        }
    });

    // Wait for shutdown signal
    shutdown.cancelled().await;
    info!("shutdown signal received, stopping listeners...");

    // Wait for listeners to finish with deadline
    let deadline = Duration::from_secs(config.graceful_exit_deadline_secs);
    let _ = tokio::time::timeout(deadline, async {
        let _ = admin_handle.await;
        let _ = ingress_handle.await;
    })
    .await;

    info!(
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
        Some(tls::build_tls_acceptor(tls_config)?)
    } else {
        None
    };

    if tls_acceptor.is_none() {
        warn!(
            "admin listener started without TLS on {}. Traffic is unencrypted.",
            config.addr
        );
    }

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    info!("admin API listening on {}://{}", scheme, config.addr);

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
                                warn!("admin: connection limit reached, rejecting {}", peer_addr);
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
                        error!("admin accept error: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!("admin listener stopping");
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
        Some(tls::build_tls_acceptor(tls)?)
    } else {
        None
    };

    if tls_acceptor.is_none() {
        warn!(
            "ingress listener started without TLS on {}. Traffic is unencrypted.",
            addr
        );
    }

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    info!("ingress listening on {}://{}", scheme, addr);

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
                                warn!("ingress: connection limit reached, rejecting {}", peer_addr);
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
                        error!("ingress accept error: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!("ingress TCP listener stopping");
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
    info!("ingress listening on unix:{}", path.display());

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
                                warn!("unix ingress: connection limit reached, rejecting connection");
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
                        error!("unix accept error: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!("ingress Unix listener stopping");
                break;
            }
        }
    }

    // Cleanup socket file
    if let Err(e) = std::fs::remove_file(&cleanup_path) {
        warn!("failed to remove Unix socket {}: {}", cleanup_path.display(), e);
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
    let router = router::Router::new(
        registry.clone(),
        config.body_limits,
        config.rate_limit_rps,
    );
    let svc = service::EdgeService::new(router);

    let listener = TcpListener::bind(config.addr).await?;

    // Create connection semaphore
    let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));
    info!(
        "connection limit set to {} concurrent connections",
        config.max_connections
    );

    // Optional TLS acceptor
    let tls_acceptor = if let Some(ref tls_config) = config.tls {
        Some(tls::build_tls_acceptor(tls_config)?)
    } else {
        None
    };

    if tls_acceptor.is_none() {
        warn!(
            "server started without TLS on {}. Traffic is unencrypted.",
            config.addr
        );
    }

    let scheme = if tls_acceptor.is_some() { "https" } else { "http" };
    info!("edge-runtime listening on {}://{}", scheme, config.addr);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer_addr)) => {
                        // Try to acquire connection permit
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!("connection limit reached, rejecting {}", peer_addr);
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
                        error!("failed to accept connection: {}", e);
                    }
                }
            }
            _ = shutdown.cancelled() => {
                info!("shutdown signal received, stopping server...");
                break;
            }
        }
    }

    // Graceful shutdown: wait for in-flight connections
    info!(
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

    use functions::registry::FunctionRegistry;
    use rcgen::generate_simple_self_signed;
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, RootCertStore};
    use runtime_core::isolate::IsolateConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;

    static RUSTLS_INIT: Once = Once::new();

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

    fn make_temp_tls_files() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, Vec<u8>) {
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
        let server_handle = tokio::spawn(async move {
            run_server(server_config, registry, server_shutdown).await
        });

        tokio::time::sleep(Duration::from_millis(120)).await;

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
}
