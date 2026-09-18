use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, ValueEnum};
use tokio_util::sync::CancellationToken;
use tracing::info;

use functions::registry::{FunctionRegistry, PoolRuntimeConfig};
use functions::types::{ContextPoolLimits, PoolLimits};
use runtime_core::isolate::{IsolateConfig, OutgoingProxyConfig};
use runtime_core::ssrf::SsrfConfig;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SourceMapMode {
    None,
    Inline,
}

#[derive(Args)]
pub struct StartArgs {
    // ─────────────────────────────────────────────────────────────────────────
    // Admin Listener Configuration (port 9000 by default)
    // ─────────────────────────────────────────────────────────────────────────
    /// Admin API host (default: 0.0.0.0 when binding)
    #[arg(long, env = "EDGE_RUNTIME_ADMIN_HOST")]
    admin_host: Option<String>,

    /// Admin API port (default: 9000 when binding)
    #[arg(long, env = "EDGE_RUNTIME_ADMIN_PORT")]
    admin_port: Option<u16>,

    /// Inherited listening TCP file descriptor for the admin API (Unix only).
    #[arg(
        long,
        env = "EDGE_RUNTIME_ADMIN_FD",
        conflicts_with_all = ["admin_host", "admin_port"]
    )]
    admin_fd: Option<i32>,

    /// API key for admin endpoint authentication (required in production)
    #[arg(long, env = "EDGE_RUNTIME_API_KEY")]
    api_key: Option<String>,

    /// TLS certificate file path for admin API
    #[arg(long, env = "EDGE_RUNTIME_ADMIN_TLS_CERT")]
    admin_tls_cert: Option<String>,

    /// TLS private key file path for admin API
    #[arg(long, env = "EDGE_RUNTIME_ADMIN_TLS_KEY")]
    admin_tls_key: Option<String>,

    /// Require Ed25519 signature verification for bundle deploy/update on admin API.
    #[arg(
        long,
        default_value_t = false,
        env = "EDGE_RUNTIME_REQUIRE_BUNDLE_SIGNATURE"
    )]
    require_bundle_signature: bool,

    /// Path to bundle signature Ed25519 public key (PEM, base64 raw 32-byte key, or hex).
    #[arg(long, env = "EDGE_RUNTIME_BUNDLE_PUBLIC_KEY_PATH")]
    bundle_public_key_path: Option<String>,

    // ─────────────────────────────────────────────────────────────────────────
    // Ingress Listener Configuration (TCP port or Unix socket)
    // ─────────────────────────────────────────────────────────────────────────
    /// Ingress IP address to bind (default: 0.0.0.0 for TCP binding)
    #[arg(long, env = "EDGE_RUNTIME_HOST")]
    host: Option<String>,

    /// Ingress port to listen on (mutually exclusive with --unix-socket)
    #[arg(short, long, env = "EDGE_RUNTIME_PORT")]
    port: Option<u16>,

    /// Unix socket path for ingress (mutually exclusive with --port)
    #[arg(long, env = "EDGE_RUNTIME_UNIX_SOCKET")]
    unix_socket: Option<PathBuf>,

    /// Inherited listening TCP file descriptor for ingress (Unix only).
    #[arg(
        long,
        env = "EDGE_RUNTIME_INGRESS_FD",
        conflicts_with_all = ["host", "port", "unix_socket"]
    )]
    ingress_fd: Option<i32>,

    /// TLS certificate file path for ingress (TCP only)
    #[arg(long, env = "EDGE_RUNTIME_TLS_CERT")]
    tls_cert: Option<String>,

    /// TLS private key file path for ingress (TCP only)
    #[arg(long, env = "EDGE_RUNTIME_TLS_KEY")]
    tls_key: Option<String>,

    // ─────────────────────────────────────────────────────────────────────────
    // Security Options
    // ─────────────────────────────────────────────────────────────────────────
    /// Disable SSRF protection (allows fetch to private IPs) - NOT recommended for production
    #[arg(
        long,
        default_value_t = false,
        env = "EDGE_RUNTIME_DISABLE_SSRF_PROTECTION"
    )]
    disable_ssrf_protection: bool,

    /// Allow specific private subnets despite SSRF protection (comma-separated CIDRs).
    /// Example: --allow-private-net "10.1.0.0/16,10.2.0.0/16"
    #[arg(long, value_delimiter = ',', env = "EDGE_RUNTIME_ALLOW_PRIVATE_NET")]
    allow_private_net: Vec<String>,

    /// Outgoing HTTP proxy URL (eg. http://proxy.local:8080, socks5://proxy.local:1080)
    #[arg(long, env = "EDGE_RUNTIME_HTTP_OUTGOING_PROXY")]
    http_outgoing_proxy: Option<String>,

    /// Outgoing HTTPS proxy URL (eg. http://proxy.local:8080, socks5://proxy.local:1080)
    #[arg(long, env = "EDGE_RUNTIME_HTTPS_OUTGOING_PROXY")]
    https_outgoing_proxy: Option<String>,

    /// Outgoing TCP proxy endpoint (host:port or tcp://host:port)
    #[arg(long, env = "EDGE_RUNTIME_TCP_OUTGOING_PROXY")]
    tcp_outgoing_proxy: Option<String>,

    /// Bypass list for HTTP proxy (comma-separated hosts/domains)
    #[arg(long, value_delimiter = ',', env = "EDGE_RUNTIME_HTTP_NO_PROXY")]
    http_no_proxy: Vec<String>,

    /// Bypass list for HTTPS proxy (comma-separated hosts/domains)
    #[arg(long, value_delimiter = ',', env = "EDGE_RUNTIME_HTTPS_NO_PROXY")]
    https_no_proxy: Vec<String>,

    /// Bypass list for TCP proxy (comma-separated hosts/domains)
    #[arg(long, value_delimiter = ',', env = "EDGE_RUNTIME_TCP_NO_PROXY")]
    tcp_no_proxy: Vec<String>,

    // ─────────────────────────────────────────────────────────────────────────
    // Body Size Limits
    // ─────────────────────────────────────────────────────────────────────────
    /// Maximum request body size in bytes (default: 5242880 = 5 MiB)
    #[arg(long, default_value_t = 5 * 1024 * 1024, env = "EDGE_RUNTIME_MAX_REQUEST_BODY_SIZE")]
    max_request_body_size: usize,

    /// Maximum response body size in bytes (default: 10485760 = 10 MiB)
    #[arg(long, default_value_t = 10 * 1024 * 1024, env = "EDGE_RUNTIME_MAX_RESPONSE_BODY_SIZE")]
    max_response_body_size: usize,

    // ─────────────────────────────────────────────────────────────────────────
    // Connection Limits
    // ─────────────────────────────────────────────────────────────────────────
    /// Maximum concurrent active connections across all listeners (default: 50000)
    #[arg(long, default_value_t = 50_000, env = "EDGE_RUNTIME_MAX_CONNECTIONS")]
    max_connections: usize,

    /// Enable isolate pooling in this process.
    #[arg(long, default_value_t = true, env = "EDGE_RUNTIME_POOL_ENABLED")]
    pool_enabled: bool,

    /// Global max isolates across all functions in this process.
    #[arg(
        long,
        default_value_t = 64,
        env = "EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES"
    )]
    pool_global_max_isolates: usize,

    /// Default minimum isolates kept warm per function pool.
    #[arg(long, default_value_t = 5, env = "EDGE_RUNTIME_POOL_MIN_ISOLATES")]
    pool_min_isolates: usize,

    /// Default maximum isolates allowed per function pool.
    #[arg(long, default_value_t = 10, env = "EDGE_RUNTIME_POOL_MAX_ISOLATES")]
    pool_max_isolates: usize,

    /// Minimum free memory required (MiB) to allow pool scale-up.
    #[arg(
        long,
        default_value_t = 256,
        env = "EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB"
    )]
    pool_min_free_memory_mib: u64,

    /// Wait time (ms) to queue requests under temporary pool saturation before returning 503.
    #[arg(
        long,
        default_value_t = 300,
        env = "EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS"
    )]
    pool_capacity_wait_timeout_ms: u64,

    /// Max number of concurrent waiting requests while pool is saturated.
    #[arg(
        long,
        default_value_t = 20_000,
        env = "EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS"
    )]
    pool_capacity_max_waiters: usize,

    /// Enable context-aware scheduler (context-first, isolate-next).
    #[arg(
        long,
        default_value_t = true,
        env = "EDGE_RUNTIME_CONTEXT_POOL_ENABLED"
    )]
    context_pool_enabled: bool,

    /// Max logical contexts tracked per isolate.
    #[arg(
        long,
        default_value_t = 64,
        env = "EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE"
    )]
    max_contexts_per_isolate: usize,

    /// Max active requests per logical context.
    #[arg(
        long,
        default_value_t = 8,
        env = "EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT"
    )]
    max_active_requests_per_context: usize,

    /// Default minimum logical contexts maintained per function.
    #[arg(
        long,
        default_value_t = 1,
        env = "EDGE_RUNTIME_CONTEXT_POOL_MIN_CONTEXTS"
    )]
    context_pool_min_contexts: usize,

    /// Default maximum logical contexts allowed per function.
    #[arg(
        long,
        default_value_t = 256,
        env = "EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS"
    )]
    context_pool_max_contexts: usize,

    // ─────────────────────────────────────────────────────────────────────────
    // Common Options
    // ─────────────────────────────────────────────────────────────────────────
    /// Rate limit (requests per second, 0 = unlimited)
    #[arg(long, default_value_t = 0, env = "EDGE_RUNTIME_RATE_LIMIT")]
    rate_limit: u64,

    /// Graceful shutdown deadline in seconds
    #[arg(long, default_value_t = 30)]
    graceful_exit_timeout: u64,

    /// Default max heap size per isolate in MiB (0 = unlimited)
    #[arg(long, default_value_t = 128, env = "EDGE_RUNTIME_MAX_HEAP_MIB")]
    max_heap_mib: u64,

    /// Default CPU time limit per request in ms (0 = unlimited)
    #[arg(long, default_value_t = 50000, env = "EDGE_RUNTIME_CPU_TIME_LIMIT_MS")]
    cpu_time_limit_ms: u64,

    /// Default wall clock timeout per request in ms (0 = unlimited)
    #[arg(
        long,
        default_value_t = 60000,
        env = "EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS"
    )]
    wall_clock_timeout_ms: u64,

    /// Print user function `console.*` logs to runtime stdout.
    /// If disabled, logs are captured only by the internal isolate collector.
    #[arg(long, default_value_t = true, env = "EDGE_RUNTIME_PRINT_ISOLATE_LOGS")]
    print_isolate_logs: bool,

    /// Default VFS total writable quota in bytes per isolate.
    #[arg(
        long,
        default_value_t = 10 * 1024 * 1024,
        env = "EDGE_RUNTIME_VFS_TOTAL_QUOTA_BYTES"
    )]
    vfs_total_quota_bytes: usize,

    /// Default VFS max writable file size in bytes per isolate.
    #[arg(
        long,
        default_value_t = 5 * 1024 * 1024,
        env = "EDGE_RUNTIME_VFS_MAX_FILE_BYTES"
    )]
    vfs_max_file_bytes: usize,

    /// DNS-over-HTTPS resolver endpoint used by node:dns compatibility layer.
    #[arg(
        long,
        default_value = "https://1.1.1.1/dns-query",
        env = "EDGE_RUNTIME_DNS_DOH_ENDPOINT"
    )]
    dns_doh_endpoint: String,

    /// Maximum DNS answers returned per query by node:dns compatibility layer.
    #[arg(long, default_value_t = 16, env = "EDGE_RUNTIME_DNS_MAX_ANSWERS")]
    dns_max_answers: usize,

    /// DNS resolver timeout in milliseconds for node:dns compatibility layer.
    #[arg(long, default_value_t = 2000, env = "EDGE_RUNTIME_DNS_TIMEOUT_MS")]
    dns_timeout_ms: u64,

    /// Default node:zlib max output length in bytes (hard-ceiling enforced by runtime).
    #[arg(
        long,
        default_value_t = 16 * 1024 * 1024,
        env = "EDGE_RUNTIME_ZLIB_MAX_OUTPUT_LENGTH"
    )]
    zlib_max_output_length: usize,

    /// Default node:zlib max input length in bytes (hard-ceiling enforced by runtime).
    #[arg(
        long,
        default_value_t = 8 * 1024 * 1024,
        env = "EDGE_RUNTIME_ZLIB_MAX_INPUT_LENGTH"
    )]
    zlib_max_input_length: usize,

    /// Default node:zlib operation timeout in milliseconds.
    #[arg(
        long,
        default_value_t = 250,
        env = "EDGE_RUNTIME_ZLIB_OPERATION_TIMEOUT_MS"
    )]
    zlib_operation_timeout_ms: u64,

    /// Maximum outbound network requests per execution (0 = unlimited).
    #[arg(
        long,
        default_value_t = 0,
        env = "EDGE_RUNTIME_EGRESS_MAX_REQUESTS_PER_EXECUTION"
    )]
    egress_max_requests_per_execution: usize,

    /// Source map handling for modules loaded from eszip
    #[arg(
        long,
        value_enum,
        default_value = "none",
        env = "EDGE_RUNTIME_SOURCE_MAP"
    )]
    sourcemap: SourceMapMode,
}

pub fn run(args: StartArgs) -> Result<(), anyhow::Error> {
    validate_start_args(&args)?;

    // Warn if TLS specified with Unix socket
    if args.unix_socket.is_some() && (args.tls_cert.is_some() || args.tls_key.is_some()) {
        tracing::warn!("TLS options (--tls-cert, --tls-key) ignored for Unix socket ingress");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("edge-rt")
        .build()?;

    runtime.block_on(async {
        let shutdown = CancellationToken::new();

        // Build SSRF config
        let ssrf_config = if args.disable_ssrf_protection {
            tracing::warn!(
                "SSRF protection disabled - fetch can access private IPs. \
                 This is NOT recommended for production."
            );
            SsrfConfig::disabled()
        } else if !args.allow_private_net.is_empty() {
            info!(
                "SSRF protection enabled with exceptions: {:?}",
                args.allow_private_net
            );
            SsrfConfig::with_exceptions(args.allow_private_net.clone())
        } else {
            info!("SSRF protection enabled (blocking private IP ranges)");
            SsrfConfig::new()
        };

        let default_config = IsolateConfig {
            max_heap_size_bytes: (args.max_heap_mib as usize) * 1024 * 1024,
            cpu_time_limit_ms: args.cpu_time_limit_ms,
            wall_clock_timeout_ms: args.wall_clock_timeout_ms,
            inspect_port: None,
            inspect_brk: false,
            inspect_allow_remote: false,
            enable_source_maps: matches!(args.sourcemap, SourceMapMode::Inline),
            ssrf_config,
            print_isolate_logs: args.print_isolate_logs,
            vfs_total_quota_bytes: args.vfs_total_quota_bytes,
            vfs_max_file_bytes: args.vfs_max_file_bytes,
            dns_doh_endpoint: args.dns_doh_endpoint,
            dns_max_answers: args.dns_max_answers,
            dns_timeout_ms: args.dns_timeout_ms,
            zlib_max_output_length: args.zlib_max_output_length,
            zlib_max_input_length: args.zlib_max_input_length,
            zlib_operation_timeout_ms: args.zlib_operation_timeout_ms,
            egress_max_requests_per_execution: args.egress_max_requests_per_execution,
            context_pool_enabled: if args.pool_enabled && !args.context_pool_enabled {
                tracing::warn!(
                    "pool enabled without context pool; enabling context-aware scheduler automatically"
                );
                true
            } else {
                args.context_pool_enabled
            },
            max_contexts_per_isolate: args.max_contexts_per_isolate,
            max_active_requests_per_context: args.max_active_requests_per_context,
        };

        if args.pool_min_isolates > args.pool_max_isolates {
            return Err(anyhow::anyhow!(
                "invalid pool isolate limits: --pool-min-isolates must be <= --pool-max-isolates"
            ));
        }

        if args.context_pool_min_contexts > args.context_pool_max_contexts {
            return Err(anyhow::anyhow!(
                "invalid context limits: --context-pool-min-contexts must be <= --context-pool-max-contexts"
            ));
        }

        let pool_config = PoolRuntimeConfig {
            enabled: args.pool_enabled,
            global_max_isolates: args.pool_global_max_isolates,
            min_free_memory_mib: args.pool_min_free_memory_mib,
            capacity_wait_timeout_ms: args.pool_capacity_wait_timeout_ms,
            capacity_wait_max_waiters: args.pool_capacity_max_waiters,
            outgoing_proxy: OutgoingProxyConfig {
                http_proxy: args.http_outgoing_proxy,
                https_proxy: args.https_outgoing_proxy,
                tcp_proxy: args.tcp_outgoing_proxy,
                http_no_proxy: args.http_no_proxy,
                https_no_proxy: args.https_no_proxy,
                tcp_no_proxy: args.tcp_no_proxy,
            },
        };

        let registry = Arc::new(FunctionRegistry::new_with_pool(
            shutdown.clone(),
            default_config,
            pool_config,
            PoolLimits {
                min: args.pool_min_isolates,
                max: args.pool_max_isolates,
            },
            ContextPoolLimits {
                min: args.context_pool_min_contexts,
                max: args.context_pool_max_contexts,
            },
        ));

        crate::telemetry::spawn_isolate_log_exporter(shutdown.clone(), args.print_isolate_logs);

        // Spawn signal handler
        let shutdown_signal = shutdown.clone();
        tokio::spawn(edge_server::graceful::wait_for_shutdown_signal(
            shutdown_signal,
        ));

        // Build body limits config
        let body_limits = edge_server::BodyLimitsConfig {
            max_request_body_bytes: args.max_request_body_size,
            max_response_body_bytes: args.max_response_body_size,
        };

        if args.require_bundle_signature && args.bundle_public_key_path.is_none() {
            return Err(anyhow::anyhow!(
                "--require-bundle-signature requires --bundle-public-key-path"
            ));
        }

        if let Some(path) = &args.bundle_public_key_path {
            edge_server::bundle_signature::ensure_public_key_path_exists(path)?;
        }

        // Build admin listener config
        let admin_listener_type = match args.admin_fd {
            Some(fd) => edge_server::TcpListenerSource::InheritedFd(fd),
            None => {
                let host = args.admin_host.as_deref().unwrap_or("0.0.0.0");
                let port = args.admin_port.unwrap_or(9000);
                let addr: SocketAddr = format!("{host}:{port}").parse()?;
                edge_server::TcpListenerSource::Bind(addr)
            }
        };
        let admin_tls = match (&args.admin_tls_cert, &args.admin_tls_key) {
            (Some(cert), Some(key)) => Some(edge_server::TlsConfig {
                cert_path: cert.clone(),
                key_path: key.clone(),
            }),
            _ => None,
        };

        // Build ingress listener config
        let ingress_type = match (args.ingress_fd, &args.unix_socket, args.port) {
            (Some(fd), _, _) => {
                edge_server::IngressListenerType::Tcp(edge_server::TcpListenerSource::InheritedFd(
                    fd,
                ))
            }
            (None, Some(path), _) => edge_server::IngressListenerType::Unix(path.clone()),
            (None, _, Some(port)) => {
                let host = args.host.as_deref().unwrap_or("0.0.0.0");
                let addr: SocketAddr = format!("{host}:{port}").parse()?;
                edge_server::IngressListenerType::Tcp(edge_server::TcpListenerSource::Bind(addr))
            }
            (None, None, None) => {
                // Default: TCP port 8080
                let host = args.host.as_deref().unwrap_or("0.0.0.0");
                let addr: SocketAddr = format!("{host}:8080").parse()?;
                edge_server::IngressListenerType::Tcp(edge_server::TcpListenerSource::Bind(addr))
            }
        };

        let ingress_tls = match (&args.tls_cert, &args.tls_key, &args.unix_socket) {
            (Some(cert), Some(key), None) => Some(edge_server::TlsConfig {
                cert_path: cert.clone(),
                key_path: key.clone(),
            }),
            _ => None,
        };

        let config = edge_server::DualServerConfig {
            admin: edge_server::AdminListenerConfig {
                listener_type: admin_listener_type,
                api_key: args.api_key,
                tls: admin_tls,
                body_limits,
                bundle_signature: edge_server::bundle_signature::config_from_flag(
                    args.require_bundle_signature,
                    args.bundle_public_key_path,
                ),
            },
            ingress: edge_server::IngressListenerConfig {
                listener_type: ingress_type,
                tls: ingress_tls,
                rate_limit_rps: if args.rate_limit > 0 {
                    Some(args.rate_limit)
                } else {
                    None
                },
                body_limits,
            },
            graceful_exit_deadline_secs: args.graceful_exit_timeout,
            max_connections: args.max_connections,
        };

        let ingress_target = match &config.ingress.listener_type {
            edge_server::IngressListenerType::Tcp(edge_server::TcpListenerSource::Bind(addr)) => {
                format!("tcp://{addr}")
            }
            edge_server::IngressListenerType::Tcp(
                edge_server::TcpListenerSource::InheritedFd(fd),
            ) => format!("inherited-fd:{fd}"),
            edge_server::IngressListenerType::Unix(path) => {
                format!("unix:{}", path.display())
            }
        };

        info!(
            admin_listener = ?config.admin.listener_type,
            ingress = ingress_target,
            "starting thunder dual-listener server"
        );

        // Run the dual-listener server (blocks until shutdown)
        edge_server::run_dual_server(config, registry.clone(), shutdown.clone()).await?;

        info!("thunder stopped");
        Ok(())
    })
}

fn validate_start_args(args: &StartArgs) -> Result<(), anyhow::Error> {
    if args.port.is_some() && args.unix_socket.is_some() {
        return Err(anyhow::anyhow!(
            "--port and --unix-socket are mutually exclusive"
        ));
    }

    if let Some(fd) = args.admin_fd {
        if fd < 0 {
            return Err(anyhow::anyhow!(
                "--admin-fd must be a non-negative file descriptor"
            ));
        }
    }
    if let Some(fd) = args.ingress_fd {
        if fd < 0 {
            return Err(anyhow::anyhow!(
                "--ingress-fd must be a non-negative file descriptor"
            ));
        }
    }
    if args.admin_fd.is_some() && args.admin_fd == args.ingress_fd {
        return Err(anyhow::anyhow!(
            "--admin-fd and --ingress-fd cannot use the same file descriptor"
        ));
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    if args.admin_fd.is_some() || args.ingress_fd.is_some() {
        return Err(anyhow::anyhow!(
            "--admin-fd and --ingress-fd are supported only on macOS and Linux"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Command, FromArgMatches};

    fn parse_start_args(args: &[&str]) -> Result<StartArgs, clap::Error> {
        let command = StartArgs::augment_args(Command::new("start"));
        let matches = command.try_get_matches_from(args)?;
        StartArgs::from_arg_matches(&matches)
    }

    #[test]
    fn inherited_listener_flags_parse_with_their_environment_names() {
        let args = parse_start_args(&["start", "--admin-fd", "3", "--ingress-fd", "4"])
            .expect("inherited listener flags should parse");

        assert_eq!(args.admin_fd, Some(3));
        assert_eq!(args.ingress_fd, Some(4));
        assert!(
            args.admin_host.is_none() && args.admin_port.is_none() && args.host.is_none(),
            "bind defaults must remain absent so FD conflicts only apply to explicit selectors"
        );
        validate_start_args(&args).expect("distinct non-negative FDs should validate");
    }

    #[test]
    fn inherited_listener_flags_conflict_with_bind_selectors() {
        for arguments in [
            &["start", "--admin-fd", "3", "--admin-host", "127.0.0.1"][..],
            &["start", "--admin-fd", "3", "--admin-port", "9001"][..],
            &["start", "--ingress-fd", "4", "--host", "127.0.0.1"][..],
            &["start", "--ingress-fd", "4", "--port", "8081"][..],
            &[
                "start",
                "--ingress-fd",
                "4",
                "--unix-socket",
                "/tmp/thunder.sock",
            ][..],
        ] {
            assert!(
                parse_start_args(arguments).is_err(),
                "expected conflicting arguments to fail: {arguments:?}"
            );
        }
    }

    #[test]
    fn inherited_listener_flags_reject_negative_and_duplicate_descriptors() {
        let negative = parse_start_args(&["start", "--admin-fd=-1"])
            .expect("negative descriptor syntax should parse for validation");
        let negative_error =
            validate_start_args(&negative).expect_err("negative descriptor must be rejected");
        assert!(negative_error.to_string().contains("--admin-fd"));

        let duplicate = parse_start_args(&["start", "--admin-fd", "3", "--ingress-fd", "3"])
            .expect("duplicate descriptor syntax should parse for validation");
        let duplicate_error =
            validate_start_args(&duplicate).expect_err("duplicate descriptor must be rejected");
        assert!(duplicate_error.to_string().contains("same file descriptor"));
    }

    #[cfg(windows)]
    #[test]
    fn inherited_listener_flags_are_explicitly_unsupported_on_windows() {
        let args = parse_start_args(&["start", "--admin-fd", "3"])
            .expect("inherited listener flag syntax should parse");
        let error =
            validate_start_args(&args).expect_err("non-Unix platforms must reject inherited FDs");
        assert!(error
            .to_string()
            .contains("supported only on macOS and Linux"));
    }
}
