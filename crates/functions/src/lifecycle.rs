use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::Error;
use bytes::Bytes;
use chrono::Utc;
use deno_core::{
    InspectorMsg, InspectorSessionChannels, InspectorSessionKind, InspectorSessionProxy,
};
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use http::StatusCode;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tungstenite::{Message, WebSocket};

use runtime_core::cpu_timer::CpuTimer;
use runtime_core::extensions;
use runtime_core::isolate::{
    determine_root_specifier, IsolateConfig, IsolateHandle, IsolateRequest, IsolateResponse,
    OutgoingProxyConfig,
};
use runtime_core::isolate_logs::IsolateLogConfig;
use runtime_core::manifest::ResolvedFunctionManifest;
use runtime_core::mem_check::{near_heap_limit_callback, HeapLimitState};
use runtime_core::module_loader::EszipModuleLoader;
use runtime_core::permissions::create_permissions_with_policy;

use crate::handler;
use crate::types::*;

struct InspectorServerGuard {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

const MAX_ISOLATE_RESTARTS: u32 = 5;

fn fail_pending_requests(request_rx: &mut mpsc::UnboundedReceiver<IsolateRequest>, reason: &str) {
    request_rx.close();
    while let Ok(req) = request_rx.try_recv() {
        let _ = req
            .response_tx
            .send(Err(anyhow::anyhow!(reason.to_string())));
    }
}

fn timeout_response() -> IsolateResponse {
    let response = http::Response::builder()
        .status(StatusCode::GATEWAY_TIMEOUT)
        .header("content-type", "application/json")
        .body(bytes::Bytes::from_static(br#"{"error":"request timeout"}"#))
        .expect("failed to build timeout response");
    IsolateResponse::from_full_response(response)
}

fn set_env_var_pair(name_upper: &str, value: Option<&str>) {
    let name_lower = name_upper.to_ascii_lowercase();
    match value {
        Some(v) if !v.trim().is_empty() => {
            std::env::set_var(name_upper, v);
            std::env::set_var(name_lower, v);
        }
        _ => {
            std::env::remove_var(name_upper);
            std::env::remove_var(&name_lower);
        }
    }
}

fn apply_outgoing_proxy_env(config: &runtime_core::isolate::OutgoingProxyConfig) {
    static PROXY_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = PROXY_ENV_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().expect("proxy env lock poisoned");

    set_env_var_pair("HTTP_PROXY", config.http_proxy.as_deref());
    set_env_var_pair("HTTPS_PROXY", config.https_proxy.as_deref());
    set_env_var_pair("ALL_PROXY", config.tcp_proxy.as_deref());

    let mut merged_no_proxy: Vec<String> = Vec::new();
    for entry in config
        .http_no_proxy
        .iter()
        .chain(config.https_no_proxy.iter())
        .chain(config.tcp_no_proxy.iter())
    {
        let item = entry.trim();
        if item.is_empty() {
            continue;
        }
        if !merged_no_proxy.iter().any(|existing| existing == item) {
            merged_no_proxy.push(item.to_string());
        }
    }

    if merged_no_proxy.is_empty() {
        set_env_var_pair("NO_PROXY", None);
    } else {
        let joined = merged_no_proxy.join(",");
        set_env_var_pair("NO_PROXY", Some(&joined));
    }
}

async fn parse_eszip_bundle(
    eszip_bytes: Vec<u8>,
) -> Result<(Arc<eszip::EszipV2>, deno_core::ModuleSpecifier), Error> {
    let reader = futures_util::io::BufReader::new(futures_util::io::Cursor::new(eszip_bytes));
    let (eszip, loader_fut) = eszip::EszipV2::parse(reader)
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse eszip: {e}"))?;

    tokio::spawn(loader_fut);

    let eszip = Arc::new(eszip);
    let root_specifier = determine_root_specifier(&eszip)?;
    Ok((eszip, root_specifier))
}

impl Drop for InspectorServerGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Create a FunctionEntry: parse bundle (snapshot or eszip), boot isolate on a dedicated thread.
pub async fn create_function(
    name: String,
    bundle_data: Vec<u8>,
    config: IsolateConfig,
    outgoing_proxy: OutgoingProxyConfig,
    manifest: Option<ResolvedFunctionManifest>,
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

    // Validate that the bundle can be parsed before spawning isolate supervisor.
    let _ = parse_eszip_bundle(eszip_bytes_vec.clone()).await?;

    // Create the request channel
    let (request_tx, request_rx) = mpsc::unbounded_channel::<IsolateRequest>();

    // Create the alive flag - starts as true, set to false when isolate exits
    let alive = Arc::new(AtomicBool::new(true));
    let alive_for_thread = alive.clone();

    // Build the IsolateHandle
    let shutdown = parent_shutdown.child_token();
    let handle = IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: shutdown.clone(),
        id: uuid::Uuid::new_v4(),
        alive,
    };

    // Create the inspector stop flag on the main thread so destroy_function
    // can signal the listener thread directly without waiting for the full
    // isolate shutdown chain.
    let inspector_stop = if config.inspect_port.is_some() {
        Some(Arc::new(AtomicBool::new(false)))
    } else {
        None
    };
    let inspector_stop_for_thread = inspector_stop.clone();

    // Spawn the isolate supervisor on a dedicated thread (JsRuntime is !Send)
    let isolate_name = name.clone();
    let isolate_config = config.clone();
    let isolate_manifest = manifest.clone();
    let isolate_metrics = metrics.clone();
    let isolate_outgoing_proxy = outgoing_proxy.clone();
    let bundle_format = bundle_package.format;
    let snapshot_bytes = if bundle_package.format == BundleFormat::Snapshot {
        Some(bundle_package.bundle.clone())
    } else {
        None
    };
    let eszip_bytes_for_thread = eszip_bytes_vec.clone();
    let supervisor_handle = handle.clone();

    std::thread::Builder::new()
        .name(format!("fn-{}", name))
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for isolate");

            let mut request_rx = request_rx;
            let mut restart_count = 0_u32;
            let eszip_bytes_for_restart = eszip_bytes_for_thread;

            loop {
                let local = tokio::task::LocalSet::new();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    local.block_on(
                        &rt,
                        async {
                            // Re-parse eszip on each restart attempt because module sources
                            // are consumed by the loader during execution.
                            let (eszip, root_specifier) =
                                parse_eszip_bundle(eszip_bytes_for_restart.clone()).await?;

                            run_isolate(
                                isolate_name.clone(),
                                eszip,
                                root_specifier,
                                isolate_config.clone(),
                                isolate_outgoing_proxy.clone(),
                                isolate_manifest.clone(),
                                &mut request_rx,
                                shutdown.clone(),
                                isolate_metrics.clone(),
                                bundle_format,
                                snapshot_bytes.clone(),
                                bundle_package.v8_version.clone(),
                                inspector_stop_for_thread.clone(),
                                supervisor_handle.clone(),
                            )
                            .await
                        },
                    )
                }));

                match result {
                    Ok(Ok(())) => {
                        info!(function_name = %isolate_name, request_id = "system", "isolate '{}' exited cleanly", isolate_name);
                        break;
                    }
                    Ok(Err(e)) => {
                        if shutdown.is_cancelled() {
                            info!(function_name = %isolate_name, request_id = "system", "isolate '{}' stopped during shutdown", isolate_name);
                            break;
                        }
                        error!(function_name = %isolate_name, request_id = "system", "isolate '{}' exited with error: {}", isolate_name, e);
                        break;
                    }
                    Err(e) => {
                        error!(function_name = %isolate_name, request_id = "system", "isolate '{}' panicked: {:?}", isolate_name, e);

                        // Mark dead and close request channel so pending/new requests fail fast.
                        supervisor_handle.mark_dead();
                        supervisor_handle.close_request_tx();
                        fail_pending_requests(
                            &mut request_rx,
                            "isolate panicked; request channel closed",
                        );

                        if shutdown.is_cancelled() {
                            break;
                        }

                        if restart_count >= MAX_ISOLATE_RESTARTS {
                            error!(
                                function_name = %isolate_name,
                                request_id = "system",
                                "isolate '{}' exceeded max restart attempts ({}), giving up",
                                isolate_name, MAX_ISOLATE_RESTARTS
                            );
                            break;
                        }

                        restart_count += 1;
                        let backoff_secs = (1_u64 << (restart_count.saturating_sub(1))).min(60);
                        warn!(
                            function_name = %isolate_name,
                            request_id = "system",
                            "restarting isolate '{}' after panic (attempt {}/{}), backoff={}s",
                            isolate_name, restart_count, MAX_ISOLATE_RESTARTS, backoff_secs
                        );

                        std::thread::sleep(Duration::from_secs(backoff_secs));
                        if shutdown.is_cancelled() {
                            break;
                        }

                        // Re-open request channel for the new isolate instance.
                        let (new_tx, new_rx) = mpsc::unbounded_channel::<IsolateRequest>();
                        supervisor_handle.replace_request_tx(new_tx);
                        request_rx = new_rx;
                    }
                }
            }

            // Mark isolate as dead when supervisor exits.
            alive_for_thread.store(false, Ordering::SeqCst);
            supervisor_handle.mark_dead();
            supervisor_handle.close_request_tx();
        })
        .map_err(|e| anyhow::anyhow!("failed to spawn isolate thread: {e}"))?;

    Ok(FunctionEntry {
        name,
        eszip_bytes: Bytes::from(eszip_bytes_vec),
        bundle_format,
        isolate_handle: Some(handle),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop,
        status: FunctionStatus::Running,
        config,
        manifest,
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
    outgoing_proxy: OutgoingProxyConfig,
    manifest: Option<ResolvedFunctionManifest>,
    request_rx: &mut mpsc::UnboundedReceiver<IsolateRequest>,
    shutdown: CancellationToken,
    metrics: Arc<FunctionMetrics>,
    bundle_format: BundleFormat,
    snapshot_bytes: Option<Vec<u8>>,
    v8_version: String,
    inspector_stop: Option<Arc<AtomicBool>>,
    liveness_handle: IsolateHandle,
) -> Result<(), Error> {
    // Track cold start timing
    let cold_start_timer = std::time::Instant::now();

    // Try to load from snapshot first, fall back to eszip if needed
    let (mut js_runtime, heap_limit_state_ptr) = match bundle_format {
        BundleFormat::Snapshot => {
            if v8_version == deno_core::v8::VERSION_STRING {
                if let Some(snapshot_data) = snapshot_bytes {
                    info!("loading '{}' from V8 snapshot", name);
                    match load_from_snapshot(&snapshot_data, &config).await {
                        Ok(rt) => (rt, None),
                        Err(e) => {
                            warn!("failed to load snapshot: {}, trying fallback eszip", e);
                            load_from_eszip_with_init(
                                &eszip,
                                &root_specifier,
                                &config,
                                &outgoing_proxy,
                                manifest.as_ref(),
                                &name,
                            )
                            .await?
                        }
                    }
                } else {
                    info!("snapshot data missing, loading from eszip");
                    load_from_eszip_with_init(
                        &eszip,
                        &root_specifier,
                        &config,
                        &outgoing_proxy,
                        manifest.as_ref(),
                        &name,
                    )
                    .await?
                }
            } else {
                warn!(
                    "snapshot V8 version mismatch (snapshot: {}, current: {}), using eszip fallback",
                    v8_version,
                    deno_core::v8::VERSION_STRING
                );
                load_from_eszip_with_init(
                    &eszip,
                    &root_specifier,
                    &config,
                    &outgoing_proxy,
                    manifest.as_ref(),
                    &name,
                )
                .await?
            }
        }
        BundleFormat::Eszip => {
            load_from_eszip_with_init(
                &eszip,
                &root_specifier,
                &config,
                &outgoing_proxy,
                manifest.as_ref(),
                &name,
            )
            .await?
        }
    };

    let _inspector_guard = if let Some(port) = config.inspect_port {
        // Inspector was already initialized inside load_from_eszip_with_init
        // (before module loading), so V8 tracks the script from compilation time.
        let inspector = js_runtime.inspector();
        let session_sender = inspector.get_session_sender();
        let stop = inspector_stop
            .clone()
            .expect("inspector_stop must be Some when inspect_port is Some");
        let (guard, target_id) = start_inspector_server(
            session_sender,
            port,
            name.clone(),
            root_specifier.as_str().to_string(),
            stop,
            config.inspect_allow_remote,
        )?;
        let inspector_host = if config.inspect_allow_remote {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        warn!(
            "function '{}' inspector is enabled on {}:{} (debug-only; do not use in production)",
            name, inspector_host, port
        );
        if config.inspect_allow_remote {
            warn!(
                "function '{}' inspector remote access is enabled; debugger endpoint is exposed on all network interfaces",
                name
            );
        }
        info!(
            "function '{}' inspector listening on ws://{}:{}/{}",
            name, inspector_host, port, target_id
        );

        if config.inspect_brk {
            info!(
                "function '{}' waiting for debugger session (inspect-brk)",
                name
            );
            inspector.wait_for_session_and_break_on_next_statement();
            // After the session is established, give VS Code ~150ms to send its
            // initialization messages (Runtime.enable, Debugger.enable, etc.).
            // Then flush them through the event loop so V8 processes
            // Debugger.enable before user code resumes.
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = js_runtime
                .run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                    pump_v8_message_loop: true,
                })
                .await;
        }

        Some(guard)
    } else {
        None
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

    info!(
        "function '{}' isolate initialized, entering request loop",
        name
    );
    liveness_handle.mark_alive();

    // Keep one CPU timer per isolate and reset it for each incoming request.
    let mut cpu_timer = CpuTimer::new(config.cpu_time_limit_ms);
    let mut runtime_tick = tokio::time::interval(Duration::from_millis(10));
    runtime_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Request handling loop
    loop {
        tokio::select! {
            Some(req) = request_rx.recv() => {
                #[cfg(debug_assertions)]
                {
                    if let Some(path) = std::env::var_os("EDGE_RUNTIME_TEST_PANIC_ON_PATH") {
                        let path = path.to_string_lossy();
                        if req.request.uri().path() == path {
                            panic!("injected panic for testing on path '{}': function='{}'", path, name);
                        }
                    }
                }

                metrics.active_requests.fetch_add(1, Ordering::Relaxed);
                metrics.total_requests.fetch_add(1, Ordering::Relaxed);

                // Track warm request timing
                let warm_start_timer = std::time::Instant::now();

                cpu_timer.reset();
                cpu_timer.start();

                // Generate unique execution ID for this request (for timer tracking)
                let execution_id = uuid::Uuid::new_v4().to_string();

                // Start execution context (track timers/intervals for this request)
                if let Err(e) = js_runtime.execute_script(
                    "edge-internal:///start_execution.js",
                    deno_core::FastString::from(format!(
                        r#"globalThis.__edgeRuntime.startExecution("{}");"#,
                        execution_id
                    )),
                ) {
                    warn!("failed to start execution context: {}", e);
                }

                let request_context_id = req.context_id;
                let request_function_name = req.function_name;
                let request_payload = req.request;

                let result = if config.wall_clock_timeout_ms > 0 {
                    // Get thread-safe handle for the watchdog thread
                    let v8_handle = js_runtime.v8_isolate().thread_safe_handle();
                    let timeout_ms = config.wall_clock_timeout_ms;
                    let terminated = Arc::new(AtomicBool::new(false));
                    let watchdog_terminated = terminated.clone();
                    let request_completed = Arc::new(AtomicBool::new(false));
                    let watchdog_completed = request_completed.clone();

                    // Spawn watchdog thread that will forcefully terminate V8 execution on timeout
                    let watchdog = std::thread::spawn(move || {
                        let deadline = std::time::Instant::now()
                            + std::time::Duration::from_millis(timeout_ms);

                        // Poll periodically until deadline or request completes
                        while std::time::Instant::now() < deadline {
                            if watchdog_completed.load(Ordering::SeqCst) {
                                return; // Request completed before timeout
                            }
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }

                        // Deadline reached - terminate V8 execution if request still running
                        if !watchdog_completed.load(Ordering::SeqCst) {
                            if v8_handle.terminate_execution() {
                                watchdog_terminated.store(true, Ordering::SeqCst);
                            }
                        }
                    });

                    // Execute the request with an explicit async timeout.
                    // This complements V8 terminate_execution in case JS blocks the isolate.
                    let dispatch_result = tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        handler::dispatch_request_for_context(
                            &mut js_runtime,
                            request_payload,
                            request_context_id.as_deref(),
                            request_function_name.as_deref(),
                        ),
                    )
                    .await;

                    // Signal watchdog to stop and wait for it
                    request_completed.store(true, Ordering::SeqCst);
                    let _ = watchdog.join();

                    // Check if timeout happened (either async timeout or forced V8 termination)
                    if terminated.load(Ordering::SeqCst) || dispatch_result.is_err() {
                        // Reset termination state so isolate can be reused
                        js_runtime.v8_isolate().cancel_terminate_execution();

                        // Now clear timers/intervals created by this execution
                        // (must be after cancel_terminate_execution so we can execute JS)
                        if let Err(e) = js_runtime.execute_script(
                            "edge-internal:///clear_execution.js",
                            deno_core::FastString::from(format!(
                                r#"globalThis.__edgeRuntime.clearExecutionTimers("{}");"#,
                                execution_id
                            )),
                        ) {
                            warn!("failed to clear execution timers: {}", e);
                        }

                        warn!(
                            "function '{}' request forcefully terminated after {}ms",
                            name, config.wall_clock_timeout_ms
                        );
                        metrics.total_errors.fetch_add(1, Ordering::Relaxed);
                        Ok(timeout_response())
                    } else {
                        // End execution context normally (cleanup tracking, timers keep running)
                        if let Err(e) = js_runtime.execute_script(
                            "edge-internal:///end_execution.js",
                            deno_core::FastString::from(format!(
                                r#"globalThis.__edgeRuntime.endExecution("{}");"#,
                                execution_id
                            )),
                        ) {
                            warn!("failed to end execution context: {}", e);
                        }
                        dispatch_result.expect("dispatch_result should be Ok here")
                    }
                } else {
                    // No timeout configured - execute directly
                    let dispatch_result = handler::dispatch_request_for_context(
                        &mut js_runtime,
                        request_payload,
                        request_context_id.as_deref(),
                        request_function_name.as_deref(),
                    )
                    .await;

                    // End execution context
                    if let Err(e) = js_runtime.execute_script(
                        "edge-internal:///end_execution.js",
                        deno_core::FastString::from(format!(
                            r#"globalThis.__edgeRuntime.endExecution("{}");"#,
                            execution_id
                        )),
                    ) {
                        warn!("failed to end execution context: {}", e);
                    }

                    dispatch_result
                };
                let warm_duration_ms = warm_start_timer.elapsed().as_millis() as u64;
                let cpu_duration_ms = cpu_timer.stop();

                metrics.active_requests.fetch_sub(1, Ordering::Relaxed);
                metrics
                    .total_warm_start_time_ms
                    .fetch_add(warm_duration_ms, Ordering::Relaxed);
                metrics
                    .total_cpu_time_ms
                    .fetch_add(cpu_duration_ms, Ordering::Relaxed);
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

                // Check if heap limit was exceeded (via near-heap-limit callback)
                if let Some(state_ptr) = heap_limit_state_ptr {
                    // Safety: we created this pointer and it's valid until we drop it
                    let state = unsafe { &*state_ptr };
                    if state.should_terminate.load(Ordering::SeqCst) {
                        error!(
                            "function '{}' exceeded heap memory limit, terminating isolate",
                            name
                        );
                        metrics.total_errors.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
            }
            _ = runtime_tick.tick() => {
                // Keep runtime tasks moving between requests so ReadableStream/SSE
                // producers can continue pushing chunks.
                let _ = tokio::time::timeout(
                    Duration::from_millis(5),
                    js_runtime.run_event_loop(PollEventLoopOptions {
                        wait_for_inspector: false,
                        pump_v8_message_loop: true,
                    }),
                )
                .await;
            }
            _ = shutdown.cancelled() => {
                info!("isolate '{}' received shutdown signal", name);
                break;
            }
        }
    }

    // Drop the runtime BEFORE the inspector guard so that the inspector
    // channels close first. This causes any active WebSocket pump to detect
    // the dead channel. The guard's Drop then sets stop=true and joins the
    // listener thread, which exits promptly (pump_websocket also checks the
    // stop flag on every iteration).
    drop(js_runtime);

    // Clean up heap limit state if allocated
    if let Some(state_ptr) = heap_limit_state_ptr {
        // Safety: we allocated this with Box::into_raw, now reclaim it
        let _ = unsafe { Box::from_raw(state_ptr) };
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
    Err(anyhow::anyhow!(
        "snapshot loading not yet supported - using eszip fallback"
    ))
}

/// Load a JsRuntime from an eszip bundle and initialize it completely.
/// Returns the runtime and an optional pointer to HeapLimitState (if heap limit is configured).
async fn load_from_eszip_with_init(
    eszip: &Arc<eszip::EszipV2>,
    root_specifier: &deno_core::ModuleSpecifier,
    config: &IsolateConfig,
    outgoing_proxy: &OutgoingProxyConfig,
    manifest: Option<&ResolvedFunctionManifest>,
    function_name: &str,
) -> Result<(JsRuntime, Option<*mut HeapLimitState>), Error> {
    apply_outgoing_proxy_env(outgoing_proxy);

    // Set up V8 heap limits
    let create_params = if config.max_heap_size_bytes > 0 {
        Some(deno_core::v8::CreateParams::default().heap_limits(0, config.max_heap_size_bytes))
    } else {
        None
    };

    // Create JsRuntime with the eszip module loader
    let module_loader = std::rc::Rc::new(EszipModuleLoader::new_with_source_maps(
        eszip.clone(),
        config.enable_source_maps,
    ));

    let mut runtime_extensions = extensions::get_extensions();
    runtime_extensions.push(handler::response_stream_extension());

    let mut runtime_opts = RuntimeOptions {
        module_loader: Some(module_loader),
        create_params,
        extensions: runtime_extensions,
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut runtime_opts);

    let mut js_runtime = JsRuntime::new(runtime_opts);
    handler::ensure_response_stream_registry(&mut js_runtime);

    // Register near-heap-limit callback if heap limit is configured
    let heap_limit_state_ptr = if config.max_heap_size_bytes > 0 {
        let v8_handle = js_runtime.v8_isolate().thread_safe_handle();
        let state = Box::new(HeapLimitState::new(
            config.max_heap_size_bytes,
            function_name.to_string(),
            v8_handle,
        ));
        let state_ptr = Box::into_raw(state);

        js_runtime.v8_isolate().add_near_heap_limit_callback(
            near_heap_limit_callback,
            state_ptr as *mut std::ffi::c_void,
        );

        Some(state_ptr)
    } else {
        None
    };

    // Put permissions into the op_state for extensions
    {
        let mut env_allow = None;
        let mut net_allow = None;
        if let Some(policy) = manifest {
            let mut merged_env = policy.env_allow.clone();
            for secret_name in &policy.env_secret_refs {
                if !merged_env.iter().any(|name| name == secret_name) {
                    merged_env.push(secret_name.clone());
                }
            }
            env_allow = Some(merged_env);
            net_allow = Some(policy.network_allow.clone());
        }

        let op_state = js_runtime.op_state();
        let mut state = op_state.borrow_mut();
        state.put(create_permissions_with_policy(
            &config.ssrf_config,
            net_allow,
            env_allow,
        ));
        state.put(IsolateLogConfig {
            function_name: function_name.to_string(),
            emit_to_stdout: config.print_isolate_logs,
        });
    }

    // Register the request handler bridge in the JS global scope
    handler::inject_request_bridge_with_proxy_and_config(
        &mut js_runtime,
        &OutgoingProxyConfig::default(),
        config,
    )?;

    // Initialize the inspector BEFORE loading user modules so V8 is in debug
    // mode during script compilation. This guarantees that when a debugger
    // session later sends Debugger.enable, V8 can retroactively send
    // Debugger.scriptParsed for all compiled scripts (including the user module).
    if config.inspect_port.is_some() {
        js_runtime.maybe_init_inspector();
    }

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

    Ok((js_runtime, heap_limit_state_ptr))
}

fn start_inspector_server(
    session_sender: deno_core::futures::channel::mpsc::UnboundedSender<InspectorSessionProxy>,
    port: u16,
    target_name: String,
    root_url: String,
    stop: Arc<AtomicBool>,
    allow_remote: bool,
) -> Result<(InspectorServerGuard, String), Error> {
    let target_id = uuid::Uuid::new_v4().to_string();
    let stop_for_thread = stop.clone();
    let target_id_for_thread = target_id.clone();
    let root_url_for_thread = root_url.clone();
    let inspector_host = if allow_remote { "0.0.0.0" } else { "127.0.0.1" };

    // On watch hot-reload, the previous isolate may still be tearing down and
    // releasing this port. Retry briefly to avoid spurious reload failures.
    let listener = bind_inspector_listener_with_retry(
        inspector_host,
        port,
        30,
        std::time::Duration::from_millis(100),
    )?;

    listener
        .set_nonblocking(true)
        .map_err(|e| anyhow::anyhow!("failed to configure inspector listener: {}", e))?;

    let handle = thread::spawn(move || {
        let target_id = target_id_for_thread;
        let root_url = root_url_for_thread;
        let ws_path = format!("/{}", target_id);

        while !stop_for_thread.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let mut peek_buf = [0u8; 2048];
                    let peek_len = match stream.peek(&mut peek_buf) {
                        Ok(n) => n,
                        Err(_) => continue,
                    };

                    let req = String::from_utf8_lossy(&peek_buf[..peek_len]).to_string();
                    let first_line = req.lines().next().unwrap_or_default();
                    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                    let is_upgrade = req.to_ascii_lowercase().contains("upgrade: websocket");

                    // Accept WebSocket upgrade on the UUID path or the legacy /ws path
                    if is_upgrade && (path == ws_path.as_str() || path == "/ws") {
                        handle_websocket_session(&mut stream, &session_sender, &stop_for_thread);
                        continue;
                    }

                    if path == "/json" || path == "/json/list" {
                        let body = format!(
                            "[{{\"description\":\"thunder\",\"id\":\"{id}\",\"title\":\"{title}\",\"type\":\"node\",\"url\":\"{url}\",\"webSocketDebuggerUrl\":\"ws://{host}:{port}/{id}\",\"devtoolsFrontendUrl\":\"devtools://devtools/bundled/inspector.html?ws={host}:{port}/{id}\"}}]",
                            id = target_id,
                            title = target_name,
                            url = root_url,
                            port = port,
                            host = inspector_host,
                        );
                        let _ = write_http_json_response(&mut stream, &body);
                        continue;
                    }

                    if path == "/json/version" {
                        let body = "{\"Browser\":\"node.js/v18.0.0\",\"Protocol-Version\":\"1.1\"}";
                        let _ = write_http_json_response(&mut stream, body);
                        continue;
                    }

                    let _ = write_http_not_found(&mut stream);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => {
                    thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    });

    Ok((
        InspectorServerGuard {
            stop,
            handle: Some(handle),
        },
        target_id,
    ))
}

fn bind_inspector_listener_with_retry(
    host: &str,
    port: u16,
    max_attempts: usize,
    retry_delay: std::time::Duration,
) -> Result<TcpListener, Error> {
    let addr = (host, port);
    for attempt in 1..=max_attempts {
        match TcpListener::bind(addr) {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == ErrorKind::AddrInUse && attempt < max_attempts => {
                thread::sleep(retry_delay);
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "failed to bind inspector server on {}:{}: {}",
                    host,
                    port,
                    e
                ));
            }
        }
    }

    Err(anyhow::anyhow!(
        "failed to bind inspector server on {}:{}: address still in use after {} attempts",
        host,
        port,
        max_attempts
    ))
}

fn write_http_json_response(stream: &mut std::net::TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    std::io::Write::write_all(stream, response.as_bytes())
}

fn write_http_not_found(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    let body = "not found";
    let response = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    std::io::Write::write_all(stream, response.as_bytes())
}

fn handle_websocket_session(
    stream: &mut std::net::TcpStream,
    session_sender: &deno_core::futures::channel::mpsc::UnboundedSender<InspectorSessionProxy>,
    stop: &AtomicBool,
) {
    let cloned = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut ws = match tungstenite::accept(cloned) {
        Ok(socket) => socket,
        Err(_) => return,
    };

    if ws.get_mut().set_nonblocking(true).is_err() {
        return;
    }

    let (to_runtime_tx, to_runtime_rx) = deno_core::futures::channel::mpsc::unbounded::<String>();
    let (from_runtime_tx, mut from_runtime_rx) =
        deno_core::futures::channel::mpsc::unbounded::<InspectorMsg>();

    let proxy = InspectorSessionProxy {
        channels: InspectorSessionChannels::Regular {
            tx: from_runtime_tx,
            rx: to_runtime_rx,
        },
        kind: InspectorSessionKind::NonBlocking {
            wait_for_disconnect: false,
        },
    };

    if session_sender.unbounded_send(proxy).is_err() {
        return;
    }

    pump_websocket(&mut ws, to_runtime_tx, &mut from_runtime_rx, stop);
}

fn pump_websocket(
    ws: &mut WebSocket<std::net::TcpStream>,
    to_runtime_tx: deno_core::futures::channel::mpsc::UnboundedSender<String>,
    from_runtime_rx: &mut deno_core::futures::channel::mpsc::UnboundedReceiver<InspectorMsg>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        loop {
            match from_runtime_rx.try_recv() {
                Ok(msg) => {
                    if ws.send(Message::Text(msg.content.into())).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }

        match ws.read() {
            Ok(msg) => {
                if msg.is_close() {
                    return;
                }
                if let Message::Text(text) = msg {
                    if to_runtime_tx.unbounded_send(text.to_string()).is_err() {
                        return;
                    }
                }
            }
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
            Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                return
            }
            Err(_) => return,
        }

        thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Destroy a function: cancel its isolate and wait for cleanup.
pub async fn destroy_function(entry: &FunctionEntry) {
    // Signal the inspector listener thread to stop FIRST. This releases the
    // TCP port immediately (~50ms) without waiting for the entire isolate
    // shutdown chain (which involves dropping V8's JsRuntime).
    if let Some(stop) = &entry.inspector_stop {
        stop.store(true, Ordering::Relaxed);
    }

    if let Some(handle) = &entry.isolate_handle {
        handle.shutdown.cancel();
        // Give the isolate a moment to drain
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
