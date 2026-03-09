use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use clap::Args;
use deno_ast::{EmitOptions, TranspileOptions};
use deno_graph::ast::CapturingModuleAnalyzer;
use deno_graph::source::{LoadError, LoadOptions, LoadResponse, Loader};
use deno_graph::{BuildOptions, GraphKind, ModuleGraph};
use functions::types::BundlePackage;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use url::Url;

use runtime_core::isolate::{IsolateConfig, OutgoingProxyConfig};

use super::embedded_assert;

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum WatchBundleFormat {
    /// Standard ESZIP package.
    Eszip,
    /// Snapshot-flavor envelope with ESZIP fallback.
    Snapshot,
}

#[derive(Args)]
pub struct WatchArgs {
    /// Directory to watch (defaults to current directory)
    #[arg(default_value = ".", long)]
    path: String,

    /// IP address to bind
    #[arg(long, default_value = "0.0.0.0", env = "EDGE_RUNTIME_HOST")]
    host: String,

    /// Port to listen on
    #[arg(short, long, default_value_t = 9000, env = "EDGE_RUNTIME_PORT")]
    port: u16,

    /// Watch interval in milliseconds (debounce for file changes)
    #[arg(long, default_value_t = 1000)]
    interval: u64,

    /// Bundle format used for hot deployments.
    #[arg(long, value_enum, default_value = "snapshot")]
    format: WatchBundleFormat,

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

    /// Enable V8 inspector protocol in watch mode (optional base port, default: 9229)
    ///
    /// When multiple functions are loaded, ports are assigned sequentially:
    /// base, base+1, base+2, ... in deployment order.
    #[arg(long, value_name = "PORT", num_args = 0..=1, default_missing_value = "9229")]
    inspect: Option<u16>,

    /// Wait for debugger attach and break on first statement (requires --inspect)
    #[arg(long, default_value_t = false)]
    inspect_brk: bool,

    /// Allow inspector to bind on all interfaces (0.0.0.0). Unsafe for production.
    #[arg(long, default_value_t = false)]
    inspect_allow_remote: bool,
}

/// A simple file-system loader for deno_graph.
struct FileLoader;

impl Loader for FileLoader {
    fn load(
        &self,
        specifier: &deno_graph::ModuleSpecifier,
        _options: LoadOptions,
    ) -> deno_graph::source::LoadFuture {
        let specifier = specifier.clone();
        Box::pin(async move {
            if specifier.scheme() == "edge" || specifier.scheme() == "ext" {
                if let Some(content) = load_edge_assert_module(&specifier)? {
                    return Ok(Some(LoadResponse::Module {
                        content: content.into(),
                        specifier,
                        maybe_headers: None,
                        mtime: None,
                    }));
                }
            }

            if specifier.scheme() != "file" {
                return Ok(None);
            }

            let path = specifier.to_file_path().map_err(|()| {
                LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
                    "invalid file URL: {specifier}"
                ))))
            })?;

            let content = std::fs::read(&path).map_err(|e| {
                LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
                    "failed to read '{}': {e}",
                    path.display()
                ))))
            })?;

            let content = rewrite_edge_assert_imports(content)?;

            Ok(Some(LoadResponse::Module {
                content: content.into(),
                specifier,
                maybe_headers: None,
                mtime: None,
            }))
        })
    }
}

fn rewrite_edge_assert_imports(content: Vec<u8>) -> Result<Vec<u8>, LoadError> {
    Ok(embedded_assert::rewrite_edge_assert_imports(content))
}

fn load_edge_assert_module(
    specifier: &deno_graph::ModuleSpecifier,
) -> Result<Option<Vec<u8>>, LoadError> {
    embedded_assert::load_module_bytes(specifier)
}

pub fn run(args: WatchArgs) -> Result<(), anyhow::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("edge-rt-watch")
        .build()?;

    runtime.block_on(async {
        let path = Path::new(&args.path);

        if !path.exists() {
            return Err(anyhow::anyhow!("path '{}' does not exist", args.path));
        }

        if args.inspect_allow_remote && args.inspect.is_none() {
            return Err(anyhow::anyhow!(
                "--inspect-allow-remote requires --inspect"
            ));
        }

        let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
        let shutdown = CancellationToken::new();

        let default_config = build_watch_default_config(&args);

        if let Some(base_port) = args.inspect {
            warn!(
                "V8 inspector is enabled in watch mode on base port {}. Do not use this in production.",
                base_port
            );
            if args.inspect_allow_remote {
                warn!(
                    "Inspector remote access is enabled (--inspect-allow-remote). Debug endpoints are exposed on all interfaces."
                );
            }
        }

        let registry = Arc::new(functions::registry::FunctionRegistry::new_with_pool(
            shutdown.clone(),
            default_config.clone(),
            build_watch_pool_config(&args),
            functions::types::PoolLimits::default(),
            functions::types::ContextPoolLimits::default(),
        ));

        crate::telemetry::spawn_isolate_log_exporter(
            shutdown.clone(),
            args.print_isolate_logs,
        );

        // Spawn signal handler for graceful shutdown
        let shutdown_signal = shutdown.clone();
        tokio::spawn(edge_server::graceful::wait_for_shutdown_signal(shutdown_signal));

        let server_config = edge_server::ServerConfig {
            addr,
            tls: None,
            rate_limit_rps: None,
            // Watch mode favors fast feedback and instant cancellation.
            graceful_exit_deadline_secs: 0,
            body_limits: edge_server::BodyLimitsConfig::default(),
            max_connections: 10_000,
        };

        info!("starting edge runtime in watch mode on {}", addr);
        info!("watching '{}' for TypeScript/JavaScript files", path.display());

        // Spawn the server
        let registry_clone = registry.clone();
        let shutdown_clone = shutdown.clone();
        let server_handle = tokio::spawn(async move {
            if let Err(e) = edge_server::run_server(server_config, registry_clone.clone(), shutdown_clone).await {
                tracing::error!("server error: {}", e);
            }
        });

        // Setup file watcher channel
        let (tx, mut rx) = mpsc::unbounded_channel();
        let watch_path = path.to_path_buf();

        std::thread::spawn(move || {
            use notify::{Watcher, RecursiveMode};

            let mut watcher = match notify::recommended_watcher(move |_res: notify::Result<_>| {
                let _ = tx.send(());
            }) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("Failed to create watcher: {}", e);
                    return;
                }
            };

            if let Err(e) = watcher.watch(&watch_path, RecursiveMode::Recursive) {
                eprintln!("Failed to watch directory: {}", e);
                return;
            }

            // Keep watcher alive
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        });

        // Initial load of functions
        load_and_deploy_functions(path, &registry, &default_config, args.inspect, args.format)
            .await?;

        let mut last_update = tokio::time::Instant::now();
        let debounce_duration = Duration::from_millis(args.interval);

        tokio::select! {
            _ = server_handle => {
                info!("server exited");
            }
            _ = async {
                loop {
                    if let Some(_) = rx.recv().await {
                        let now = tokio::time::Instant::now();
                        if now.duration_since(last_update) >= debounce_duration {
                            println!("\n{}", "─".repeat(80));
                            println!("🔄 Changes detected, reloading...");
                            if let Err(e) = load_and_deploy_functions(
                                path,
                                &registry,
                                &default_config,
                                args.inspect,
                                args.format,
                            )
                            .await
                            {
                                eprintln!("❌ Error loading functions: {}", e);
                            }
                            last_update = now;
                        }
                    }
                }
            } => {}
        }

        // In watch mode we prefer immediate cancellation over graceful draining.
        // Try a short shutdown window for isolates, then continue process exit.
        if tokio::time::timeout(Duration::from_millis(200), registry.shutdown_all())
            .await
            .is_err()
        {
            tracing::warn!("watch shutdown timeout reached; forcing immediate exit");
        }

        info!("edge runtime watch mode stopped");
        Ok(())
    })
}

async fn load_and_deploy_functions(
    path: &Path,
    registry: &Arc<functions::registry::FunctionRegistry>,
    default_config: &IsolateConfig,
    inspect_base_port: Option<u16>,
    format: WatchBundleFormat,
) -> anyhow::Result<()> {
    info!("scanning {}", path.display());

    let ts_js_pattern = regex::Regex::new(r"\.(ts|js)$")?;

    let mut deployed = 0;
    let mut skipped = 0;

    let mut source_files: Vec<std::path::PathBuf> = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    source_files.sort();

    let mut inspect_index: u16 = 0;
    for file_path in source_files.iter() {
        // Skip node_modules, dist, build, etc.
        if file_path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            matches!(
                s.as_ref(),
                "node_modules" | "dist" | "build" | ".next" | ".deno" | "target"
            )
        }) {
            continue;
        }

        if !ts_js_pattern.is_match(file_path.to_string_lossy().as_ref()) {
            continue;
        }

        // Generate function name from path.
        // If watch target is a single file, strip_prefix(path) becomes empty,
        // so we fallback to the filename to keep stable names like "hello".
        let relative_path = if path.is_file() {
            file_path
                .file_name()
                .map(Path::new)
                .unwrap_or(file_path.as_path())
        } else {
            file_path.strip_prefix(path).unwrap_or(file_path.as_path())
        };
        let func_name = path_to_function_name(relative_path);

        let inspect_port = if let Some(base) = inspect_base_port {
            let port = base
                .checked_add(inspect_index)
                .ok_or_else(|| anyhow::anyhow!("inspector port overflow for '{}'", func_name))?;
            inspect_index = inspect_index.saturating_add(1);
            Some(port)
        } else {
            None
        };

        let function_config = IsolateConfig {
            max_heap_size_bytes: default_config.max_heap_size_bytes,
            cpu_time_limit_ms: default_config.cpu_time_limit_ms,
            wall_clock_timeout_ms: default_config.wall_clock_timeout_ms,
            egress_max_requests_per_execution: default_config.egress_max_requests_per_execution,
            inspect_port,
            inspect_brk: default_config.inspect_brk,
            inspect_allow_remote: default_config.inspect_allow_remote,
            enable_source_maps: default_config.enable_source_maps,
            ssrf_config: default_config.ssrf_config.clone(),
            print_isolate_logs: default_config.print_isolate_logs,
            vfs_total_quota_bytes: default_config.vfs_total_quota_bytes,
            vfs_max_file_bytes: default_config.vfs_max_file_bytes,
            dns_doh_endpoint: default_config.dns_doh_endpoint.clone(),
            dns_max_answers: default_config.dns_max_answers,
            dns_timeout_ms: default_config.dns_timeout_ms,
            zlib_max_output_length: default_config.zlib_max_output_length,
            zlib_max_input_length: default_config.zlib_max_input_length,
            zlib_operation_timeout_ms: default_config.zlib_operation_timeout_ms,
            context_pool_enabled: default_config.context_pool_enabled,
            max_contexts_per_isolate: default_config.max_contexts_per_isolate,
            max_active_requests_per_context: default_config.max_active_requests_per_context,
        };

        match bundle_file(file_path, format, &function_config).await {
            Ok(bundle_bytes) => {
                let bytes = Bytes::from(bundle_bytes);

                // Try to deploy (or update if exists)
                match registry
                    .deploy(
                        func_name.clone(),
                        bytes.clone(),
                        Some(function_config.clone()),
                        None,
                    )
                    .await
                {
                    Ok(_info) => {
                        println!("✅ Deployed: {} ({} bytes)", func_name, bytes.len());
                        if let Some(port) = inspect_port {
                            let host = if function_config.inspect_allow_remote {
                                "0.0.0.0"
                            } else {
                                "127.0.0.1"
                            };
                            println!("   └─ inspector: ws://{}:{}/ws", host, port);
                        }
                        deployed += 1;
                    }
                    Err(e) if e.to_string().contains("already exists") => {
                        // Try to update instead
                        match registry
                            .update(
                                &func_name,
                                bytes.clone(),
                                Some(function_config.clone()),
                                None,
                            )
                            .await
                        {
                            Ok(_) => {
                                println!("🔄 Updated: {}", func_name);
                                if let Some(port) = inspect_port {
                                    let host = if function_config.inspect_allow_remote {
                                        "0.0.0.0"
                                    } else {
                                        "127.0.0.1"
                                    };
                                    println!("   └─ inspector: ws://{}:{}/ws", host, port);
                                }
                                deployed += 1;
                            }
                            Err(e) => {
                                eprintln!("❌ Failed to update '{}': {}", func_name, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to deploy '{}': {}", func_name, e);
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ Failed to bundle '{}': {}", file_path.display(), e);
                skipped += 1;
            }
        }
    }

    println!("\n{}", "─".repeat(80));
    println!("📊 Summary: {} deployed, {} skipped", deployed, skipped);

    Ok(())
}

async fn bundle_file(
    file_path: &Path,
    format: WatchBundleFormat,
    config: &IsolateConfig,
) -> anyhow::Result<Vec<u8>> {
    let entrypoint = file_path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve '{}': {e}", file_path.display()))?;

    let root_url = Url::from_file_path(&entrypoint)
        .map_err(|()| anyhow::anyhow!("cannot convert path to URL: {}", entrypoint.display()))?;

    // Build module graph
    let loader = FileLoader;
    let analyzer = CapturingModuleAnalyzer::default();

    let mut graph = ModuleGraph::new(GraphKind::CodeOnly);
    graph
        .build(
            vec![root_url.clone()],
            vec![], // referrer imports
            &loader,
            BuildOptions {
                module_analyzer: &analyzer,
                ..Default::default()
            },
        )
        .await;

    graph
        .valid()
        .map_err(|e| anyhow::anyhow!("module graph error: {e}"))?;

    // Create eszip from graph
    let eszip = eszip::EszipV2::from_graph(eszip::FromGraphOptions {
        graph,
        parser: analyzer.as_capturing_parser(),
        module_kind_resolver: Default::default(),
        transpile_options: TranspileOptions::default(),
        emit_options: EmitOptions::default(),
        relative_file_base: None,
        npm_packages: None,
        npm_snapshot: Default::default(),
    })?;

    let eszip_bytes = eszip.into_bytes();

    // Package the bundle
    let pkg = match format {
        WatchBundleFormat::Eszip => BundlePackage::eszip_only(eszip_bytes),
        WatchBundleFormat::Snapshot => {
            let bytecode_cache = functions::snapshot::create_function_bytecode_cache_from_eszip(
                eszip_bytes.clone(),
                config,
                &OutgoingProxyConfig::default(),
                None,
                &file_path.display().to_string(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to create function bytecode cache: {e}"))?;
            BundlePackage::snapshot_with_fallback(bytecode_cache, eszip_bytes)
        }
    };
    let bundle_data = bincode::serialize(&pkg)?;

    Ok(bundle_data)
}

fn path_to_function_name(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    // Remove file extension
    let path_str = if path_str.ends_with(".ts") {
        &path_str[..path_str.len() - 3]
    } else if path_str.ends_with(".js") {
        &path_str[..path_str.len() - 3]
    } else {
        &path_str
    };

    // Split by path separator
    let parts: Vec<&str> = path_str.split('/').filter(|p| !p.is_empty()).collect();

    if parts.is_empty() {
        return "unknown".to_string();
    }

    // If we have at least 2 parts and the last part equals the second-to-last part,
    // it means the directory and file have the same name (e.g., hello/hello.ts)
    // In that case, take only the last part to avoid duplication
    if parts.len() >= 2 && parts[parts.len() - 1] == parts[parts.len() - 2] {
        // Remove the duplicate and use only part of the path
        let relevant_parts = &parts[0..parts.len() - 1];
        relevant_parts.join("-")
    } else {
        // Use all parts, joining with dashes
        parts.join("-")
    }
}

fn build_watch_default_config(args: &WatchArgs) -> IsolateConfig {
    IsolateConfig {
        max_heap_size_bytes: (args.max_heap_mib as usize) * 1024 * 1024,
        cpu_time_limit_ms: args.cpu_time_limit_ms,
        wall_clock_timeout_ms: args.wall_clock_timeout_ms,
        inspect_port: None,
        inspect_brk: args.inspect_brk,
        inspect_allow_remote: args.inspect_allow_remote,
        enable_source_maps: true,
        // Watch mode is local-dev oriented: do not enforce SSRF network denylist.
        ssrf_config: runtime_core::ssrf::SsrfConfig::disabled(),
        print_isolate_logs: args.print_isolate_logs,
        vfs_total_quota_bytes: args.vfs_total_quota_bytes,
        vfs_max_file_bytes: args.vfs_max_file_bytes,
        dns_doh_endpoint: args.dns_doh_endpoint.clone(),
        dns_max_answers: args.dns_max_answers,
        dns_timeout_ms: args.dns_timeout_ms,
        zlib_max_output_length: args.zlib_max_output_length,
        zlib_max_input_length: args.zlib_max_input_length,
        zlib_operation_timeout_ms: args.zlib_operation_timeout_ms,
        egress_max_requests_per_execution: args.egress_max_requests_per_execution,
        context_pool_enabled: false,
        max_contexts_per_isolate: 8,
        max_active_requests_per_context: 1,
    }
}

fn build_watch_pool_config(args: &WatchArgs) -> functions::registry::PoolRuntimeConfig {
    functions::registry::PoolRuntimeConfig {
        enabled: false,
        global_max_isolates: 1024,
        min_free_memory_mib: 256,
        capacity_wait_timeout_ms: 300,
        capacity_wait_max_waiters: 20_000,
        outgoing_proxy: OutgoingProxyConfig {
            http_proxy: args.http_outgoing_proxy.clone(),
            https_proxy: args.https_outgoing_proxy.clone(),
            tcp_proxy: args.tcp_outgoing_proxy.clone(),
            http_no_proxy: args.http_no_proxy.clone(),
            https_no_proxy: args.https_no_proxy.clone(),
            tcp_no_proxy: args.tcp_no_proxy.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_default_config_disables_ssrf_protection() {
        let args = WatchArgs {
            path: ".".to_string(),
            host: "0.0.0.0".to_string(),
            port: 9000,
            interval: 1000,
            format: WatchBundleFormat::Snapshot,
            max_heap_mib: 128,
            cpu_time_limit_ms: 50_000,
            wall_clock_timeout_ms: 60_000,
            inspect: None,
            inspect_brk: false,
            inspect_allow_remote: false,
            print_isolate_logs: true,
            vfs_total_quota_bytes: 10 * 1024 * 1024,
            vfs_max_file_bytes: 5 * 1024 * 1024,
            dns_doh_endpoint: "https://1.1.1.1/dns-query".to_string(),
            dns_max_answers: 16,
            dns_timeout_ms: 2000,
            zlib_max_output_length: 16 * 1024 * 1024,
            zlib_max_input_length: 8 * 1024 * 1024,
            zlib_operation_timeout_ms: 250,
            egress_max_requests_per_execution: 0,
            http_outgoing_proxy: None,
            https_outgoing_proxy: None,
            tcp_outgoing_proxy: None,
            http_no_proxy: vec![],
            https_no_proxy: vec![],
            tcp_no_proxy: vec![],
        };

        let cfg = build_watch_default_config(&args);
        assert!(
            !cfg.ssrf_config.enabled,
            "watch mode must allow all network by default"
        );
        assert!(cfg.ssrf_config.allow_private_subnets.is_empty());
    }
}
