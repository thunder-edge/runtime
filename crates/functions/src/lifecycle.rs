use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::net::TcpListener;
use std::io::ErrorKind;
use std::thread;

use anyhow::Error;
use bytes::Bytes;
use chrono::Utc;
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use deno_core::{InspectorMsg, InspectorSessionChannels, InspectorSessionKind, InspectorSessionProxy};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tungstenite::{Message, WebSocket};

use runtime_core::extensions;
use runtime_core::isolate::{determine_root_specifier, IsolateConfig, IsolateHandle, IsolateRequest};
use runtime_core::module_loader::EszipModuleLoader;
use runtime_core::permissions::create_permissions_container;

use crate::handler;
use crate::types::*;

struct InspectorServerGuard {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
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

    // Create the inspector stop flag on the main thread so destroy_function
    // can signal the listener thread directly without waiting for the full
    // isolate shutdown chain.
    let inspector_stop = if config.inspect_port.is_some() {
        Some(Arc::new(AtomicBool::new(false)))
    } else {
        None
    };
    let inspector_stop_for_thread = inspector_stop.clone();

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
                    inspector_stop_for_thread,
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
        inspector_stop,
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
    inspector_stop: Option<Arc<AtomicBool>>,
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

    let _inspector_guard = if let Some(port) = config.inspect_port {
        // Inspector was already initialized inside load_from_eszip_with_init
        // (before module loading), so V8 tracks the script from compilation time.
        let inspector = js_runtime.inspector();
        let session_sender = inspector.get_session_sender();
        let stop = inspector_stop.clone().expect("inspector_stop must be Some when inspect_port is Some");
        let (guard, target_id) = start_inspector_server(session_sender, port, name.clone(), root_specifier.as_str().to_string(), stop)?;
        info!(
            "function '{}' inspector listening on ws://127.0.0.1:{}/{}",
            name,
            port,
            target_id
        );

        if config.inspect_brk {
            info!(
                "function '{}' waiting for debugger session (inspect-brk)",
                name
            );
            inspector.wait_for_session_and_break_on_next_statement();
        } else {
            inspector.wait_for_session();
        }

        // After the session is established, give VS Code ~150ms to send its
        // initialization messages (Runtime.enable, Debugger.enable, etc.).
        // Then flush them through the event loop so V8 processes Debugger.enable
        // and sends Debugger.scriptParsed to VS Code BEFORE any debugger; pause
        // occurs. Without this, scriptParsed only arrives after Debugger.paused
        // and VS Code shows "Unknown Source" instead of opening the file.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let _ = js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await;

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

    // Drop the runtime BEFORE the inspector guard so that the inspector
    // channels close first. This causes any active WebSocket pump to detect
    // the dead channel. The guard's Drop then sets stop=true and joins the
    // listener thread, which exits promptly (pump_websocket also checks the
    // stop flag on every iteration).
    drop(js_runtime);

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
    let module_loader = std::rc::Rc::new(EszipModuleLoader::new_with_source_maps(
        eszip.clone(),
        config.enable_source_maps,
    ));

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
        op_state.borrow_mut().put(create_permissions_container());
    }

    // Register the request handler bridge in the JS global scope
    handler::inject_request_bridge(&mut js_runtime)?;

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

    Ok(js_runtime)
}

fn start_inspector_server(
    session_sender: deno_core::futures::channel::mpsc::UnboundedSender<InspectorSessionProxy>,
    port: u16,
    target_name: String,
    root_url: String,
    stop: Arc<AtomicBool>,
) -> Result<(InspectorServerGuard, String), Error> {
    let target_id = uuid::Uuid::new_v4().to_string();
    let stop_for_thread = stop.clone();
    let target_id_for_thread = target_id.clone();
    let root_url_for_thread = root_url.clone();

    // On watch hot-reload, the previous isolate may still be tearing down and
    // releasing this port. Retry briefly to avoid spurious reload failures.
    let listener = bind_inspector_listener_with_retry(port, 30, std::time::Duration::from_millis(100))?;

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
                            "[{{\"description\":\"deno-edge-runtime\",\"id\":\"{id}\",\"title\":\"{title}\",\"type\":\"node\",\"url\":\"{url}\",\"webSocketDebuggerUrl\":\"ws://127.0.0.1:{port}/{id}\",\"devtoolsFrontendUrl\":\"devtools://devtools/bundled/inspector.html?ws=127.0.0.1:{port}/{id}\"}}]",
                            id = target_id,
                            title = target_name,
                            url = root_url,
                            port = port,
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

    Ok((InspectorServerGuard {
        stop,
        handle: Some(handle),
    }, target_id))
}

fn bind_inspector_listener_with_retry(
    port: u16,
    max_attempts: usize,
    retry_delay: std::time::Duration,
) -> Result<TcpListener, Error> {
    let addr = ("127.0.0.1", port);
    for attempt in 1..=max_attempts {
        match TcpListener::bind(addr) {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == ErrorKind::AddrInUse && attempt < max_attempts => {
                thread::sleep(retry_delay);
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "failed to bind inspector server on 127.0.0.1:{}: {}",
                    port,
                    e
                ));
            }
        }
    }

    Err(anyhow::anyhow!(
        "failed to bind inspector server on 127.0.0.1:{}: address still in use after {} attempts",
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
            Err(tungstenite::Error::ConnectionClosed)
            | Err(tungstenite::Error::AlreadyClosed) => return,
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
