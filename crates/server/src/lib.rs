pub mod router;
pub mod service;
pub mod tls;
pub mod graceful;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Error;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use functions::registry::FunctionRegistry;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub tls: Option<TlsConfig>,
    pub rate_limit_rps: Option<u64>,
    pub graceful_exit_deadline_secs: u64,
}

/// TLS configuration.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

/// Start the HTTP server and block until shutdown.
pub async fn run_server(
    config: ServerConfig,
    registry: Arc<FunctionRegistry>,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    let router = router::Router::new(registry);
    let svc = service::EdgeService::new(router);

    let listener = TcpListener::bind(config.addr).await?;

    // Optional TLS acceptor
    let tls_acceptor = if let Some(ref tls_config) = config.tls {
        Some(tls::build_tls_acceptor(tls_config)?)
    } else {
        None
    };

    let scheme = if tls_acceptor.is_some() { "https" } else { "http" };
    info!("edge-runtime listening on {}://{}", scheme, config.addr);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer_addr)) => {
                        let svc = svc.clone();
                        let tls_acceptor = tls_acceptor.clone();
                        tokio::spawn(async move {
                            let maybe_stream = if let Some(acceptor) = tls_acceptor {
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => tls::MaybeHttpsStream::Tls(tls_stream),
                                    Err(e) => {
                                        tracing::warn!("TLS handshake failed from {}: {}", peer_addr, e);
                                        return;
                                    }
                                }
                            } else {
                                tls::MaybeHttpsStream::Plain(stream)
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

    Ok(())
}
