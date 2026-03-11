pub mod admin_router;
pub mod body_limits;
pub mod bundle_signature;
pub mod function_route_matcher;
pub mod graceful;
pub mod global_routing;
pub mod ingress_router;
pub mod middleware;
pub mod router;
pub mod service;
pub mod tls;
pub mod trace_context;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use functions::registry::FunctionRegistry;
use serde::Serialize;

use crate::admin_router::AdminRouter;
use crate::bundle_signature::{BundleSignatureConfig, BundleSignatureVerifier};
use crate::global_routing::{load_global_routing_table_from_env, GlobalRoutingState};
use crate::ingress_router::IngressRouter;
use crate::service::EdgeService;

// Re-export for convenience
pub use crate::body_limits::BodyLimitsConfig;

const FD_RESERVED_RATIO: f64 = 0.10;
const FD_RESERVED_ABSOLUTE: usize = 64;
const ACCEPT_EMFILE_BACKOFF: Duration = Duration::from_millis(50);
const ACCEPT_PERMIT_WAIT: Duration = Duration::from_millis(500);
const DEFAULT_NOFILE_TARGET: usize = 10_000;
const NOFILE_TARGET_ENV: &str = "EDGE_RUNTIME_NOFILE_TARGET";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerConnectionCapacitySnapshot {
    pub configured_max_connections: u64,
    pub effective_max_connections: u64,
    pub soft_limit: u64,
    pub reserved_fd: u64,
    pub fd_budget: u64,
}

#[derive(Debug)]
struct ListenerConnectionCapacityState {
    configured_max_connections: AtomicU64,
    effective_max_connections: AtomicU64,
    soft_limit: AtomicU64,
    reserved_fd: AtomicU64,
    fd_budget: AtomicU64,
}

impl ListenerConnectionCapacityState {
    fn new() -> Self {
        Self {
            configured_max_connections: AtomicU64::new(0),
            effective_max_connections: AtomicU64::new(0),
            soft_limit: AtomicU64::new(0),
            reserved_fd: AtomicU64::new(0),
            fd_budget: AtomicU64::new(0),
        }
    }

    fn store(&self, snapshot: &ListenerConnectionCapacitySnapshot) {
        self.configured_max_connections
            .store(snapshot.configured_max_connections, Ordering::Relaxed);
        self.effective_max_connections
            .store(snapshot.effective_max_connections, Ordering::Relaxed);
        self.soft_limit.store(snapshot.soft_limit, Ordering::Relaxed);
        self.reserved_fd
            .store(snapshot.reserved_fd, Ordering::Relaxed);
        self.fd_budget.store(snapshot.fd_budget, Ordering::Relaxed);
    }

    fn snapshot(&self) -> ListenerConnectionCapacitySnapshot {
        ListenerConnectionCapacitySnapshot {
            configured_max_connections: self.configured_max_connections.load(Ordering::Relaxed),
            effective_max_connections: self.effective_max_connections.load(Ordering::Relaxed),
            soft_limit: self.soft_limit.load(Ordering::Relaxed),
            reserved_fd: self.reserved_fd.load(Ordering::Relaxed),
            fd_budget: self.fd_budget.load(Ordering::Relaxed),
        }
    }
}

static LISTENER_CONNECTION_CAPACITY: OnceLock<ListenerConnectionCapacityState> = OnceLock::new();

fn listener_connection_capacity_state() -> &'static ListenerConnectionCapacityState {
    LISTENER_CONNECTION_CAPACITY.get_or_init(ListenerConnectionCapacityState::new)
}

pub fn current_listener_connection_capacity() -> ListenerConnectionCapacitySnapshot {
    listener_connection_capacity_state().snapshot()
}

fn fd_soft_limit() -> Option<usize> {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    // Safety: getrlimit writes to a valid pointer to `rlimit`.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) };
    if rc == 0 {
        Some(lim.rlim_cur as usize)
    } else {
        None
    }
}

fn resolve_nofile_target() -> usize {
    match std::env::var(NOFILE_TARGET_ENV) {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(0) => {
                warn!(
                    function_name = "runtime",
                    request_id = "system",
                    env = NOFILE_TARGET_ENV,
                    value = %raw,
                    default = DEFAULT_NOFILE_TARGET,
                    "invalid nofile target (must be > 0), falling back to default"
                );
                DEFAULT_NOFILE_TARGET
            }
            Ok(v) => v,
            Err(_) => {
                warn!(
                    function_name = "runtime",
                    request_id = "system",
                    env = NOFILE_TARGET_ENV,
                    value = %raw,
                    default = DEFAULT_NOFILE_TARGET,
                    "failed to parse nofile target, falling back to default"
                );
                DEFAULT_NOFILE_TARGET
            }
        },
        Err(_) => DEFAULT_NOFILE_TARGET,
    }
}

fn maybe_raise_nofile_limit() {
    let target = resolve_nofile_target();
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    // Safety: getrlimit writes to a valid pointer to `rlimit`.
    let get_rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) };
    if get_rc != 0 {
        warn!(
            function_name = "runtime",
            request_id = "system",
            target,
            error = %std::io::Error::last_os_error(),
            "failed to read RLIMIT_NOFILE"
        );
        return;
    }

    let current = lim.rlim_cur as usize;
    let hard = lim.rlim_max as usize;
    if current >= target {
        info!(
            function_name = "runtime",
            request_id = "system",
            current,
            hard,
            target,
            "RLIMIT_NOFILE already satisfies target"
        );
        return;
    }

    let desired = target.min(hard);
    if desired <= current {
        warn!(
            function_name = "runtime",
            request_id = "system",
            current,
            hard,
            target,
            "cannot raise RLIMIT_NOFILE: target exceeds hard limit"
        );
        return;
    }

    let new_lim = libc::rlimit {
        rlim_cur: desired as libc::rlim_t,
        rlim_max: lim.rlim_max,
    };

    // Safety: setrlimit reads a valid pointer to immutable `rlimit` data.
    let set_rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &new_lim) };
    if set_rc != 0 {
        warn!(
            function_name = "runtime",
            request_id = "system",
            current,
            hard,
            target,
            desired,
            error = %std::io::Error::last_os_error(),
            "failed to raise RLIMIT_NOFILE"
        );
        return;
    }

    info!(
        function_name = "runtime",
        request_id = "system",
        previous = current,
        current = desired,
        hard,
        target,
        env = NOFILE_TARGET_ENV,
        "raised RLIMIT_NOFILE"
    );
}

fn compute_listener_connection_capacity(configured: usize) -> ListenerConnectionCapacitySnapshot {
    let configured = configured.max(1);
    let Some(soft_limit) = fd_soft_limit() else {
        return ListenerConnectionCapacitySnapshot {
            configured_max_connections: configured as u64,
            effective_max_connections: configured as u64,
            soft_limit: 0,
            reserved_fd: 0,
            fd_budget: 0,
        };
    };

    if soft_limit == 0 {
        return ListenerConnectionCapacitySnapshot {
            configured_max_connections: configured as u64,
            effective_max_connections: configured as u64,
            soft_limit: 0,
            reserved_fd: 0,
            fd_budget: 0,
        };
    }

    let ratio_reserved = ((soft_limit as f64) * FD_RESERVED_RATIO).round() as usize;
    let reserved_candidate = FD_RESERVED_ABSOLUTE.max(ratio_reserved);
    // Keep room for active listeners even when process soft limit is low.
    let max_reserved = soft_limit.saturating_sub(32);
    let reserved = reserved_candidate.min(max_reserved);
    let fd_budget = soft_limit.saturating_sub(reserved).max(1);
    let effective = configured.min(fd_budget);

    ListenerConnectionCapacitySnapshot {
        configured_max_connections: configured as u64,
        effective_max_connections: effective as u64,
        soft_limit: soft_limit as u64,
        reserved_fd: reserved as u64,
        fd_budget: fd_budget as u64,
    }
}

fn is_fd_exhaustion(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(code) if code == libc::EMFILE || code == libc::ENFILE)
}

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
    maybe_raise_nofile_limit();

    // Warn if no API key configured
    if config.admin.api_key.is_none() {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "admin API running without authentication (no --api-key set). \
             This is insecure for production use."
        );
    }

    // Create connection semaphore shared across all listeners.
    // Clamp with FD budget so configured limits cannot exceed process capacity.
    let capacity = compute_listener_connection_capacity(config.max_connections);
    let effective_max_connections = capacity.effective_max_connections as usize;
    listener_connection_capacity_state().store(&capacity);
    // Keep a dedicated admin pool so control-plane stays responsive under ingress load.
    let min_admin_connections = 16usize.min(effective_max_connections.max(1));
    let admin_connections = (effective_max_connections / 10)
        .max(min_admin_connections)
        .min(256)
        .min(effective_max_connections.max(1));
    let ingress_connections = effective_max_connections.max(1);

    let admin_connection_semaphore = Arc::new(Semaphore::new(admin_connections));
    let ingress_connection_semaphore = Arc::new(Semaphore::new(ingress_connections));
    if effective_max_connections < config.max_connections {
        warn!(
            function_name = "runtime",
            request_id = "system",
            configured = config.max_connections,
            effective = effective_max_connections,
            "max_connections clamped by RLIMIT_NOFILE budget"
        );
    }
    info!(
        function_name = "runtime",
        request_id = "system",
        "connection limits set: total_effective={}, ingress_pool={}, admin_pool={}",
        effective_max_connections,
        ingress_connections,
        admin_connections,
    );

    // Load once so ingress/admin share the exact same routing snapshot.
    let global_routing = GlobalRoutingState::new(load_global_routing_table_from_env());

    // Create routers with shared registry and body limits
    let admin_router = AdminRouter::new_with_global_routing_state(
        registry.clone(),
        config.admin.api_key.clone(),
        config.admin.body_limits,
        BundleSignatureVerifier::from_config(config.admin.bundle_signature.clone())?,
        global_routing.clone(),
    );
    let ingress_router = IngressRouter::new_with_global_routing_state(
        registry.clone(),
        config.ingress.body_limits,
        config.ingress.rate_limit_rps,
        global_routing,
    );

    // Spawn admin listener
    let admin_shutdown = shutdown.clone();
    let admin_config = config.admin.clone();
    let admin_semaphore = admin_connection_semaphore.clone();
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
    let ingress_semaphore = ingress_connection_semaphore.clone();
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
                        let permit = match tokio::time::timeout(
                            ACCEPT_PERMIT_WAIT,
                            connection_semaphore.clone().acquire_owned(),
                        )
                        .await
                        {
                            Ok(Ok(permit)) => permit,
                            Ok(Err(_)) => {
                                tracing::debug!("admin semaphore closed before serving {}", peer_addr);
                                return Ok(());
                            }
                            Err(_) => {
                                warn!(
                                    function_name = "runtime",
                                    request_id = "system",
                                    listener = "admin",
                                    peer_addr = %peer_addr,
                                    wait_timeout_ms = ACCEPT_PERMIT_WAIT.as_millis() as u64,
                                    "connection refused by runtime: permit wait timeout"
                                );
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
                        if is_fd_exhaustion(&e) {
                            error!(
                                function_name = "runtime",
                                request_id = "system",
                                listener = "admin",
                                "connection refused by runtime: accept failed due to fd exhaustion: {}",
                                e
                            );
                            tokio::time::sleep(ACCEPT_EMFILE_BACKOFF).await;
                        } else {
                            error!(function_name = "runtime", request_id = "system", "admin accept error: {}", e);
                        }
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
                        let permit = match tokio::time::timeout(
                            ACCEPT_PERMIT_WAIT,
                            connection_semaphore.clone().acquire_owned(),
                        )
                        .await
                        {
                            Ok(Ok(permit)) => permit,
                            Ok(Err(_)) => {
                                tracing::debug!("ingress semaphore closed before serving {}", peer_addr);
                                return Ok(());
                            }
                            Err(_) => {
                                warn!(
                                    function_name = "runtime",
                                    request_id = "system",
                                    listener = "ingress_tcp",
                                    peer_addr = %peer_addr,
                                    wait_timeout_ms = ACCEPT_PERMIT_WAIT.as_millis() as u64,
                                    "connection refused by runtime: permit wait timeout"
                                );
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
                        if is_fd_exhaustion(&e) {
                            error!(
                                function_name = "runtime",
                                request_id = "system",
                                listener = "ingress_tcp",
                                "connection refused by runtime: accept failed due to fd exhaustion: {}",
                                e
                            );
                            tokio::time::sleep(ACCEPT_EMFILE_BACKOFF).await;
                        } else {
                            error!(function_name = "runtime", request_id = "system", "ingress accept error: {}", e);
                        }
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
                        let permit = match tokio::time::timeout(
                            ACCEPT_PERMIT_WAIT,
                            connection_semaphore.clone().acquire_owned(),
                        )
                        .await
                        {
                            Ok(Ok(permit)) => permit,
                            Ok(Err(_)) => {
                                tracing::debug!("unix ingress semaphore closed before serving connection");
                                return Ok(());
                            }
                            Err(_) => {
                                warn!(
                                    function_name = "runtime",
                                    request_id = "system",
                                    listener = "ingress_unix",
                                    wait_timeout_ms = ACCEPT_PERMIT_WAIT.as_millis() as u64,
                                    "connection refused by runtime: permit wait timeout"
                                );
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
                        if is_fd_exhaustion(&e) {
                            error!(
                                function_name = "runtime",
                                request_id = "system",
                                listener = "ingress_unix",
                                "connection refused by runtime: accept failed due to fd exhaustion: {}",
                                e
                            );
                            tokio::time::sleep(ACCEPT_EMFILE_BACKOFF).await;
                        } else {
                            error!(function_name = "runtime", request_id = "system", "unix accept error: {}", e);
                        }
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
    maybe_raise_nofile_limit();

    let router = router::Router::new(registry.clone(), config.body_limits, config.rate_limit_rps);
    let svc = service::EdgeService::new(router);

    let listener = TcpListener::bind(config.addr).await?;

    // Create connection semaphore.
    // Clamp with FD budget so configured limits cannot exceed process capacity.
    let capacity = compute_listener_connection_capacity(config.max_connections);
    let effective_max_connections = capacity.effective_max_connections as usize;
    listener_connection_capacity_state().store(&capacity);
    let connection_semaphore = Arc::new(Semaphore::new(effective_max_connections));
    if effective_max_connections < config.max_connections {
        warn!(
            function_name = "runtime",
            request_id = "system",
            configured = config.max_connections,
            effective = effective_max_connections,
            "max_connections clamped by RLIMIT_NOFILE budget"
        );
    }
    info!(
        function_name = "runtime",
        request_id = "system",
        "connection limit set to {} concurrent connections",
        effective_max_connections
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
                        let permit = match tokio::time::timeout(
                            ACCEPT_PERMIT_WAIT,
                            connection_semaphore.clone().acquire_owned(),
                        )
                        .await
                        {
                            Ok(Ok(permit)) => permit,
                            Ok(Err(_)) => {
                                tracing::debug!("legacy semaphore closed before serving {}", peer_addr);
                                return Ok(());
                            }
                            Err(_) => {
                                warn!(
                                    function_name = "runtime",
                                    request_id = "system",
                                    listener = "legacy",
                                    peer_addr = %peer_addr,
                                    wait_timeout_ms = ACCEPT_PERMIT_WAIT.as_millis() as u64,
                                    "connection refused by runtime: permit wait timeout"
                                );
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
                        if is_fd_exhaustion(&e) {
                            error!(
                                function_name = "runtime",
                                request_id = "system",
                                listener = "legacy",
                                "connection refused by runtime: accept failed due to fd exhaustion: {}",
                                e
                            );
                            tokio::time::sleep(ACCEPT_EMFILE_BACKOFF).await;
                        } else {
                            error!(function_name = "runtime", request_id = "system", "failed to accept connection: {}", e);
                        }
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

        panic!(
            "timed out waiting for routing saturation to be observed in metrics: {last_metrics}"
        );
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
    async fn e2e_ingress_streaming_response_exceeds_limit_without_rejection() {
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
                      controller.enqueue(encoder.encode(`chunk-${String(i).padStart(2, '0')}-xxxxxxxxxxxxxxxxxxxxxxxxxxxx\n`));
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
            response.starts_with("HTTP/1.1 200"),
            "expected stream response to bypass size rejection and return 200: {response}"
        );
        assert!(
            !response.contains("response body too large"),
            "did not expect full-body size check error on streaming path: {response}"
        );
        assert!(
            response.to_ascii_lowercase().contains("transfer-encoding: chunked"),
            "expected chunked transfer in streaming response: {response}"
        );
        assert!(
            response.len() > tiny_limit,
            "expected full HTTP payload to exceed configured max_response_body_bytes ({}), got {}",
            tiny_limit,
            response.len()
        );
        assert!(
            response.contains("chunk-63"),
            "expected late-stream marker proving long streamed body completed: {response}"
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
        tokio::time::timeout(Duration::from_secs(3), in_flight.read_to_end(&mut completed_buf))
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
