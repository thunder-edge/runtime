use std::net::SocketAddr;
use std::sync::Arc;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use clap::Args;
use deno_ast::{EmitOptions, TranspileOptions};
use deno_graph::source::{LoadOptions, LoadResponse, Loader};
use deno_graph::{BuildOptions, CapturingModuleAnalyzer, GraphKind, ModuleGraph};
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;
use tracing::info;
use url::Url;

use runtime_core::isolate::IsolateConfig;

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

    /// Default max heap size per isolate in MiB (0 = unlimited)
    #[arg(long, default_value_t = 128, env = "EDGE_RUNTIME_MAX_HEAP_MIB")]
    max_heap_mib: u64,

    /// Default CPU time limit per request in ms (0 = unlimited)
    #[arg(long, default_value_t = 50000, env = "EDGE_RUNTIME_CPU_TIME_LIMIT_MS")]
    cpu_time_limit_ms: u64,

    /// Default wall clock timeout per request in ms (0 = unlimited)
    #[arg(long, default_value_t = 60000, env = "EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS")]
    wall_clock_timeout_ms: u64,
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
            if specifier.scheme() != "file" {
                return Ok(None);
            }

            let path = specifier
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("invalid file URL: {specifier}"))?;

            let content = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", path.display()))?;

            Ok(Some(LoadResponse::Module {
                content: content.into(),
                specifier,
                maybe_headers: None,
            }))
        })
    }
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

        let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
        let shutdown = CancellationToken::new();

        let default_config = IsolateConfig {
            max_heap_size_bytes: (args.max_heap_mib as usize) * 1024 * 1024,
            cpu_time_limit_ms: args.cpu_time_limit_ms,
            wall_clock_timeout_ms: args.wall_clock_timeout_ms,
        };

        let registry = Arc::new(functions::registry::FunctionRegistry::new(
            shutdown.clone(),
            default_config,
        ));

        // Spawn signal handler for graceful shutdown
        let shutdown_signal = shutdown.clone();
        tokio::spawn(edge_server::graceful::wait_for_shutdown_signal(shutdown_signal));

        let server_config = edge_server::ServerConfig {
            addr,
            tls: None,
            rate_limit_rps: None,
            graceful_exit_deadline_secs: 30,
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
        load_and_deploy_functions(path, &registry).await?;

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
                            if let Err(e) = load_and_deploy_functions(path, &registry).await {
                                eprintln!("❌ Error loading functions: {}", e);
                            }
                            last_update = now;
                        }
                    }
                }
            } => {}
        }

        // Shutdown all functions
        registry.shutdown_all().await;

        info!("edge runtime watch mode stopped");
        Ok(())
    })
}

async fn load_and_deploy_functions(
    path: &Path,
    registry: &Arc<functions::registry::FunctionRegistry>,
) -> anyhow::Result<()> {
    info!("scanning {}", path.display());

    let ts_js_pattern = regex::Regex::new(r"\.(ts|js)$")?;

    let mut deployed = 0;
    let mut skipped = 0;

    for entry in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
    {
        let file_path = entry.path();

        // Skip node_modules, dist, build, etc.
        if file_path
            .components()
            .any(|c| {
                let s = c.as_os_str().to_string_lossy();
                matches!(
                    s.as_ref(),
                    "node_modules" | "dist" | "build" | ".next" | ".deno" | "target"
                )
            })
        {
            continue;
        }

        if !ts_js_pattern.is_match(file_path.to_string_lossy().as_ref()) {
            continue;
        }

        // Generate function name from path
        let relative_path = file_path.strip_prefix(path).unwrap_or(file_path);
        let func_name = path_to_function_name(relative_path);

        match bundle_file(file_path).await {
            Ok(eszip_bytes) => {
                let bytes = Bytes::from(eszip_bytes);

                // Try to deploy (or update if exists)
                match registry.deploy(func_name.clone(), bytes.clone(), None).await {
                    Ok(_info) => {
                        println!(
                            "✅ Deployed: {} ({} bytes)",
                            func_name,
                            bytes.len()
                        );
                        deployed += 1;
                    }
                    Err(e) if e.to_string().contains("already exists") => {
                        // Try to update instead
                        match registry.update(&func_name, bytes.clone(), None).await {
                            Ok(_) => {
                                println!("🔄 Updated: {}", func_name);
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

async fn bundle_file(file_path: &Path) -> anyhow::Result<Vec<u8>> {
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
            &loader,
            BuildOptions {
                module_analyzer: &analyzer,
                ..Default::default()
            },
        )
        .await;

    graph.valid().map_err(|e| anyhow::anyhow!("module graph error: {e}"))?;

    // Create eszip from graph
    let eszip = eszip::EszipV2::from_graph(eszip::FromGraphOptions {
        graph,
        parser: analyzer.as_capturing_parser(),
        module_kind_resolver: Default::default(),
        transpile_options: TranspileOptions::default(),
        emit_options: EmitOptions::default(),
        relative_file_base: None,
        npm_packages: None,
    })?;

    let eszip_bytes = eszip.into_bytes();

    // Package the bundle
    let pkg = functions::types::BundlePackage::eszip_only(eszip_bytes);
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
