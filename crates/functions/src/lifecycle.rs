use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Error;
use bytes::Bytes;
use chrono::Utc;
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use runtime_core::extensions;
use runtime_core::isolate::{determine_root_specifier, IsolateConfig, IsolateHandle, IsolateRequest};
use runtime_core::module_loader::EszipModuleLoader;
use runtime_core::permissions::Permissions;

use crate::handler;
use crate::types::*;

/// Create a FunctionEntry: parse bundle (snapshot or eszip), boot isolate on a dedicated thread.
pub async fn create_function(
    name: String,
    bundle_data: Vec<u8>,
    config: IsolateConfig,
    parent_shutdown: CancellationToken,
) -> Result<FunctionEntry, Error> {
    // Parse the bundle package
    let bundle_package: BundlePackage = bincode::deserialize(&bundle_data)
        .map_err(|e| anyhow::anyhow!("failed to deserialize bundle package: {e}"))?;

    let now = Utc::now();
    let metrics = Arc::new(FunctionMetrics::default());

    // Extract eszip bytes for fallback/reload purposes
    let eszip_bytes_vec = match bundle_package.format {
        BundleFormat::Eszip => bundle_package.bundle.clone(),
        BundleFormat::Snapshot => {
            // For snapshots, use fallback eszip if available
            bundle_package
                .fallback_eszip
                .clone()
                .unwrap_or_else(|| bundle_package.bundle.clone())
        }
    };

    // Parse eszip asynchronously
    let reader = futures_util::io::BufReader::new(futures_util::io::Cursor::new(eszip_bytes_vec.clone()));
    let (eszip, loader_fut) = eszip::EszipV2::parse(reader)
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse eszip: {e}"))?;

    // Spawn the lazy loader future
    tokio::spawn(loader_fut);

    let eszip = Arc::new(eszip);
    let root_specifier = determine_root_specifier(&eszip)?;

    // Create the request channel
    let (request_tx, request_rx) = mpsc::unbounded_channel::<IsolateRequest>();

    // Build the IsolateHandle
    let shutdown = parent_shutdown.child_token();
    let handle = IsolateHandle {
        request_tx,
        shutdown: shutdown.clone(),
        id: uuid::Uuid::new_v4(),
    };

    // Spawn the isolate on a dedicated thread (JsRuntime is !Send)
    let isolate_name = name.clone();
    let isolate_config = config.clone();
    let isolate_metrics = metrics.clone();
    let bundle_format = bundle_package.format;
    let snapshot_bytes = if bundle_package.format == BundleFormat::Snapshot {
        Some(bundle_package.bundle.clone())
    } else {
        None
    };

    std::thread::Builder::new()
        .name(format!("fn-{}", name))
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for isolate");

            let local = tokio::task::LocalSet::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                local.block_on(&rt, run_isolate(
                    isolate_name.clone(),
                    eszip,
                    root_specifier,
                    isolate_config,
                    request_rx,
                    shutdown,
                    isolate_metrics,
                    bundle_format,
                    snapshot_bytes,
                    bundle_package.v8_version.clone(),
                ))
            }));
            match result {
                Ok(res) => match res {
                    Ok(()) => info!("isolate '{}' exited cleanly", isolate_name),
                    Err(e) => error!("isolate '{}' exited with error: {}", isolate_name, e),
                }
                Err(e) => error!("isolate '{}' panicked: {:?}", isolate_name, e),
            }
        })
        .map_err(|e| anyhow::anyhow!("failed to spawn isolate thread: {e}"))?;

    Ok(FunctionEntry {
        name,
        eszip_bytes: Bytes::from(eszip_bytes_vec),
        bundle_format,
        isolate_handle: Some(handle),
        status: FunctionStatus::Running,
        config,
        metrics,
        created_at: now,
        updated_at: now,
        last_error: None,
    })
}

/// The long-running isolate event loop.
async fn run_isolate(
    name: String,
    eszip: Arc<eszip::EszipV2>,
    root_specifier: deno_core::ModuleSpecifier,
    config: IsolateConfig,
    mut request_rx: mpsc::UnboundedReceiver<IsolateRequest>,
    shutdown: CancellationToken,
    metrics: Arc<FunctionMetrics>,
    bundle_format: BundleFormat,
    snapshot_bytes: Option<Vec<u8>>,
    v8_version: String,
) -> Result<(), Error> {
    // Track cold start timing
    let cold_start_timer = std::time::Instant::now();

    // Try to load from snapshot first, fall back to eszip if needed
    let mut js_runtime = match bundle_format {
        BundleFormat::Snapshot => {
            if v8_version == deno_core::v8::VERSION_STRING {
                if let Some(snapshot_data) = snapshot_bytes {
                    info!("loading '{}' from V8 snapshot", name);
                    match load_from_snapshot(&snapshot_data, &config).await {
                        Ok(rt) => rt,
                        Err(e) => {
                            warn!("failed to load snapshot: {}, trying fallback eszip", e);
                            load_from_eszip_with_init(&eszip, &root_specifier, &config).await?
                        }
                    }
                } else {
                    info!("snapshot data missing, loading from eszip");
                    load_from_eszip_with_init(&eszip, &root_specifier, &config).await?
                }
            } else {
                warn!(
                    "snapshot V8 version mismatch (snapshot: {}, current: {}), using eszip fallback",
                    v8_version,
                    deno_core::v8::VERSION_STRING
                );
                load_from_eszip_with_init(&eszip, &root_specifier, &config).await?
            }
        }
        BundleFormat::Eszip => load_from_eszip_with_init(&eszip, &root_specifier, &config).await?,
    };

    // Record cold start time
    let cold_start_duration_ms = cold_start_timer.elapsed().as_millis() as u64;
    metrics.cold_start_count.fetch_add(1, Ordering::Relaxed);
    metrics
        .total_cold_start_time_ms
        .fetch_add(cold_start_duration_ms, Ordering::Relaxed);
    info!(
        "function '{}' cold started in {}ms (format: {})",
        name, cold_start_duration_ms, bundle_format
    );

    info!("function '{}' isolate initialized, entering request loop", name);

    // Request handling loop
    loop {
        tokio::select! {
            Some(req) = request_rx.recv() => {
                metrics.active_requests.fetch_add(1, Ordering::Relaxed);
                metrics.total_requests.fetch_add(1, Ordering::Relaxed);

                // Track warm request timing
                let warm_start_timer = std::time::Instant::now();
                let result = handler::dispatch_request(&mut js_runtime, req.request).await;
                let warm_duration_ms = warm_start_timer.elapsed().as_millis() as u64;

                metrics.active_requests.fetch_sub(1, Ordering::Relaxed);
                metrics
                    .total_warm_start_time_ms
                    .fetch_add(warm_duration_ms, Ordering::Relaxed);
                if result.is_err() {
                    metrics.total_errors.fetch_add(1, Ordering::Relaxed);
                }

                // Send response back (ignore if receiver dropped)
                let _ = req.response_tx.send(result);

                // Pump the event loop to process any pending async work
                let _ = js_runtime
                    .run_event_loop(PollEventLoopOptions {
                        wait_for_inspector: false,
                        pump_v8_message_loop: true,
                    })
                    .await;
            }
            _ = shutdown.cancelled() => {
                info!("isolate '{}' received shutdown signal", name);
                break;
            }
        }
    }

    Ok(())
}

/// Load a JsRuntime from a V8 snapshot.
///
/// NOTE: V8 snapshot support requires compiling the snapshot data into a binary
/// at compile time or having it available as a Box<[u8]> with static lifetime.
/// For now, this falls back to eszip loading. In the future, we can optimize
/// this by pre-compiling snapshots and embedding them as static data.
async fn load_from_snapshot(
    _snapshot_data: &[u8],
    _config: &IsolateConfig,
) -> Result<JsRuntime, Error> {
    // TODO: Implement snapshot loading once deno_core supports dynamic snapshot loading
    // Currently, snapshots need 'static lifetime data which is incompatible with
    // runtime-created snapshots. Options:
    // 1. Pre-compile snapshots at build time
    // 2. Wait for deno_core to support owned snapshot data
    // 3. Store snapshots in mmap'd files
    Err(anyhow::anyhow!("snapshot loading not yet supported - using eszip fallback"))
}

/// Load a JsRuntime from an eszip bundle and initialize it completely.
async fn load_from_eszip_with_init(
    eszip: &Arc<eszip::EszipV2>,
    root_specifier: &deno_core::ModuleSpecifier,
    config: &IsolateConfig,
) -> Result<JsRuntime, Error> {
    // Set up V8 heap limits
    let create_params = if config.max_heap_size_bytes > 0 {
        Some(deno_core::v8::CreateParams::default().heap_limits(0, config.max_heap_size_bytes))
    } else {
        None
    };

    // Create JsRuntime with the eszip module loader
    let module_loader = std::rc::Rc::new(EszipModuleLoader::new(eszip.clone()));

    let mut runtime_opts = RuntimeOptions {
        module_loader: Some(module_loader),
        create_params,
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut runtime_opts);

    let mut js_runtime = JsRuntime::new(runtime_opts);

    // Put permissions into the op_state for extensions
    {
        let op_state = js_runtime.op_state();
        op_state.borrow_mut().put(Permissions);
    }

    // Register the request handler bridge in the JS global scope
    handler::inject_request_bridge(&mut js_runtime)?;

    // Load and evaluate the main module
    let module_id = js_runtime.load_main_es_module(root_specifier).await?;
    let eval_result = js_runtime.mod_evaluate(module_id);

    js_runtime
        .run_event_loop(PollEventLoopOptions {
            wait_for_inspector: false,
            pump_v8_message_loop: true,
        })
        .await?;

    eval_result.await?;

    Ok(js_runtime)
}

/// Destroy a function: cancel its isolate and wait for cleanup.
pub async fn destroy_function(entry: &FunctionEntry) {
    if let Some(handle) = &entry.isolate_handle {
        handle.shutdown.cancel();
        // Give the isolate a moment to drain
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
