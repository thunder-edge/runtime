use super::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::Utc;

use crate::types::{BundleFormat, FunctionEntry, FunctionMetrics, FunctionStatus};

fn make_registry() -> FunctionRegistry {
    let shutdown = CancellationToken::new();
    FunctionRegistry::new(shutdown, IsolateConfig::default())
}

fn make_registry_with_pool(enabled: bool) -> FunctionRegistry {
    let shutdown = CancellationToken::new();
    FunctionRegistry::new_with_pool(
        shutdown,
        IsolateConfig::default(),
        PoolRuntimeConfig {
            enabled,
            global_max_isolates: 16,
            min_free_memory_mib: 0,
            capacity_wait_timeout_ms: 75,
            capacity_wait_max_waiters: 20_000,
            outgoing_proxy: OutgoingProxyConfig::default(),
        },
        PoolLimits::default(),
        ContextPoolLimits::default(),
    )
}

#[test]
fn empty_registry_count_zero() {
    let reg = make_registry();
    assert_eq!(reg.count(), 0);
}

#[test]
fn empty_registry_list_empty() {
    let reg = make_registry();
    assert!(reg.list().is_empty());
}

#[test]
fn get_handle_none_for_missing() {
    let reg = make_registry();
    assert!(reg.get_handle("nonexistent").is_none());
}

#[test]
fn get_info_none_for_missing() {
    let reg = make_registry();
    assert!(reg.get_info("nonexistent").is_none());
}

#[test]
fn delete_missing_returns_error() {
    let reg = make_registry();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(reg.delete("nonexistent"));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn dead_isolate_is_marked_as_error_in_registry() {
    let reg = make_registry();

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let alive = Arc::new(AtomicBool::new(false));
    let handle = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive,
    };

    let entry = FunctionEntry {
        name: "dead-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(handle),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("dead-fn".to_string(), entry);

    assert!(reg.get_handle("dead-fn").is_none());

    let info = reg.get_info("dead-fn").expect("missing function info");
    assert_eq!(info.status, FunctionStatus::Error);
    assert!(info.last_error.is_some());
}

#[test]
fn dead_primary_handle_can_transition_back_to_running() {
    let reg = make_registry();

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let alive = Arc::new(AtomicBool::new(false));
    let handle = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: alive.clone(),
    };

    let entry = FunctionEntry {
        name: "recover-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(handle.clone()),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("recover-fn".to_string(), entry);

    assert!(reg.get_handle("recover-fn").is_none());
    assert_eq!(
        reg.get_info("recover-fn").expect("missing info").status,
        FunctionStatus::Error
    );

    alive.store(true, Ordering::SeqCst);

    let recovered_handle = reg
        .get_handle("recover-fn")
        .expect("expected handle after recovery");
    assert_eq!(recovered_handle.id, handle.id);
    assert_eq!(
        reg.get_info("recover-fn").expect("missing info").status,
        FunctionStatus::Running
    );
}

#[test]
fn shutdown_all_with_deadline_closes_registry_entries() {
    let reg = make_registry();

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let alive = Arc::new(AtomicBool::new(true));
    let handle = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive,
    };

    let entry = FunctionEntry {
        name: "shutdown-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(handle),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };
    reg.functions.insert("shutdown-fn".to_string(), entry);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(reg.shutdown_all_with_deadline(std::time::Duration::from_millis(20)));

    assert_eq!(reg.count(), 0);
}

#[test]
fn get_handle_round_robin_across_replicas() {
    let reg = make_registry_with_pool(true);

    let (primary_tx, _primary_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(primary_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let (replica_tx, _replica_rx) = tokio::sync::mpsc::unbounded_channel();
    let replica = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(replica_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "rr-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary.clone()),
        extra_isolate_handles: vec![replica.clone()],
        pool_limits: PoolLimits { min: 1, max: 2 },
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("rr-fn".to_string(), entry);

    let h1 = reg.get_handle("rr-fn").expect("missing first handle");
    let h2 = reg.get_handle("rr-fn").expect("missing second handle");
    let h3 = reg.get_handle("rr-fn").expect("missing third handle");

    assert_eq!(h1.id, primary.id);
    assert_eq!(h2.id, replica.id);
    assert_eq!(h3.id, primary.id);
}

#[test]
fn get_route_target_none_for_missing_function() {
    let reg = make_registry();
    assert!(reg.get_route_target("missing").is_none());
}

#[test]
fn context_first_scheduler_creates_new_context_before_new_isolate() {
    let reg = make_registry();

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "ctx-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary.clone()),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig {
            context_pool_enabled: true,
            max_contexts_per_isolate: 2,
            max_active_requests_per_context: 1,
            ..IsolateConfig::default()
        },
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("ctx-fn".to_string(), entry);

    let route_a = reg
        .get_route_target("ctx-fn")
        .expect("first route target should exist");
    let route_b = reg
        .get_route_target("ctx-fn")
        .expect("second route target should exist");

    assert_eq!(route_a.isolate_id, primary.id);
    assert_eq!(route_b.isolate_id, primary.id);
    assert_ne!(route_a.context_id, route_b.context_id);

    reg.release_route_target(&route_a);
    reg.release_route_target(&route_b);

    let route_c = reg
        .get_route_target("ctx-fn")
        .expect("route target after release should exist");
    assert_eq!(route_c.isolate_id, primary.id);
}

#[test]
fn route_target_with_status_returns_unavailable_for_missing_function() {
    let reg = make_registry();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let result = rt.block_on(reg.get_route_target_with_status("missing"));
    assert!(matches!(result, Err(RouteTargetError::FunctionUnavailable)));
}

#[test]
fn route_target_with_status_returns_capacity_exhausted_when_context_is_saturated() {
    let reg = make_registry();

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "ctx-saturated".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary.clone()),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig {
            context_pool_enabled: true,
            max_contexts_per_isolate: 1,
            max_active_requests_per_context: 1,
            ..IsolateConfig::default()
        },
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("ctx-saturated".to_string(), entry);

    let route = reg
        .get_route_target("ctx-saturated")
        .expect("first route target should exist");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let saturated = rt.block_on(reg.get_route_target_with_status("ctx-saturated"));
    assert!(matches!(
        saturated,
        Err(RouteTargetError::CapacityExhausted)
    ));

    reg.release_route_target(&route);
}

#[test]
fn routing_metrics_snapshot_reports_context_and_isolate_saturation() {
    let reg = make_registry();

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "ctx-metrics".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary.clone()),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig {
            context_pool_enabled: true,
            max_contexts_per_isolate: 1,
            max_active_requests_per_context: 1,
            ..IsolateConfig::default()
        },
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("ctx-metrics".to_string(), entry);

    let route = reg
        .get_route_target("ctx-metrics")
        .expect("first route target should exist");

    let snapshot = reg.routing_metrics_snapshot();
    assert_eq!(snapshot.total_contexts, 1);
    assert_eq!(snapshot.total_isolates, 1);
    assert_eq!(snapshot.total_active_requests, 1);
    assert_eq!(snapshot.saturated_contexts, 1);
    assert_eq!(snapshot.saturated_isolates, 1);

    reg.release_route_target(&route);

    let after_release = reg.routing_metrics_snapshot();
    assert_eq!(after_release.saturated_contexts, 0);
    assert_eq!(after_release.saturated_isolates, 0);
}

#[test]
fn get_route_target_skips_dead_isolate_entries() {
    let reg = make_registry();

    let (dead_tx, _dead_rx) = tokio::sync::mpsc::unbounded_channel();
    let dead = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(dead_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(false)),
    };

    let (alive_tx, _alive_rx) = tokio::sync::mpsc::unbounded_channel();
    let alive = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(alive_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "ctx-dead".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(dead),
        extra_isolate_handles: vec![alive.clone()],
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig {
            context_pool_enabled: true,
            max_contexts_per_isolate: 2,
            max_active_requests_per_context: 1,
            ..IsolateConfig::default()
        },
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("ctx-dead".to_string(), entry);

    let route = reg
        .get_route_target("ctx-dead")
        .expect("route target should exist on alive isolate");
    assert_eq!(route.isolate_id, alive.id);
}

#[test]
fn set_pool_limits_updates_entry_when_pool_enabled() {
    let reg = make_registry_with_pool(true);

    let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "pool-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(handle),
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };
    reg.functions.insert("pool-fn".to_string(), entry);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(reg.set_pool_limits("pool-fn", 1, 3));

    assert!(result.is_ok(), "set_pool_limits should succeed");
    let limits = reg.get_pool_limits("pool-fn").expect("pool limits missing");
    assert_eq!(limits.min, 1);
    assert_eq!(limits.max, 3);
}

#[test]
fn evict_lru_replica_prefers_oldest_extra_handle() {
    let reg = make_registry_with_pool(true);

    let (primary_a_tx, _primary_a_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary_a = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(primary_a_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };
    let (extra_a_tx, _extra_a_rx) = tokio::sync::mpsc::unbounded_channel();
    let extra_a = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(extra_a_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry_a = FunctionEntry {
        name: "fn-a".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary_a),
        extra_isolate_handles: vec![extra_a.clone()],
        pool_limits: PoolLimits { min: 1, max: 2 },
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    let (primary_b_tx, _primary_b_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary_b = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(primary_b_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };
    let (extra_b_tx, _extra_b_rx) = tokio::sync::mpsc::unbounded_channel();
    let extra_b = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(extra_b_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry_b = FunctionEntry {
        name: "fn-b".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary_b),
        extra_isolate_handles: vec![extra_b.clone()],
        pool_limits: PoolLimits { min: 1, max: 2 },
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };

    reg.functions.insert("fn-a".to_string(), entry_a);
    reg.functions.insert("fn-b".to_string(), entry_b);

    // Lower tick means older usage and should be evicted first.
    reg.handle_last_used.insert(extra_a.id, 10);
    reg.handle_last_used.insert(extra_b.id, 20);

    assert!(reg.evict_lru_replica_for_capacity("fn-caller"));

    {
        let fn_a = reg.functions.get("fn-a").expect("missing fn-a");
        assert!(fn_a.extra_isolate_handles.is_empty());
    }
    {
        let fn_b = reg.functions.get("fn-b").expect("missing fn-b");
        assert_eq!(fn_b.extra_isolate_handles.len(), 1);
    }
}

#[test]
fn evict_lru_replica_respects_min_pool_size() {
    let reg = make_registry_with_pool(true);

    let (primary_tx, _primary_rx) = tokio::sync::mpsc::unbounded_channel();
    let primary = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(primary_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };
    let (extra_tx, _extra_rx) = tokio::sync::mpsc::unbounded_channel();
    let extra = runtime_core::isolate::IsolateHandle {
        request_tx: Arc::new(std::sync::Mutex::new(Some(extra_tx))),
        shutdown: CancellationToken::new(),
        id: uuid::Uuid::new_v4(),
        alive: Arc::new(AtomicBool::new(true)),
    };

    let entry = FunctionEntry {
        name: "fn-min".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
        isolate_handle: Some(primary),
        extra_isolate_handles: vec![extra],
        pool_limits: PoolLimits { min: 2, max: 2 },
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics: Arc::new(FunctionMetrics::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_error: None,
    };
    reg.functions.insert("fn-min".to_string(), entry);

    assert!(!reg.evict_lru_replica_for_capacity("fn-min"));
    let current = reg.functions.get("fn-min").expect("missing fn-min");
    assert_eq!(current.extra_isolate_handles.len(), 1);
}
