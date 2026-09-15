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
#[path = "lib_test.rs"]
mod tests;
