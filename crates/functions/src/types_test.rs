use super::*;

#[test]
fn function_status_serde_round_trip() {
    let statuses = vec![
        FunctionStatus::Loading,
        FunctionStatus::Running,
        FunctionStatus::Error,
        FunctionStatus::ShuttingDown,
    ];
    for status in statuses {
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: FunctionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, deserialized);
    }
}

#[test]
fn function_status_snake_case() {
    assert_eq!(
        serde_json::to_string(&FunctionStatus::Loading).unwrap(),
        "\"loading\""
    );
    assert_eq!(
        serde_json::to_string(&FunctionStatus::Running).unwrap(),
        "\"running\""
    );
    assert_eq!(
        serde_json::to_string(&FunctionStatus::Error).unwrap(),
        "\"error\""
    );
    assert_eq!(
        serde_json::to_string(&FunctionStatus::ShuttingDown).unwrap(),
        "\"shutting_down\""
    );
}

#[test]
fn function_metrics_default_zeros() {
    let m = FunctionMetrics::default();
    assert_eq!(m.total_requests.load(Ordering::Relaxed), 0);
    assert_eq!(m.active_requests.load(Ordering::Relaxed), 0);
    assert_eq!(m.total_errors.load(Ordering::Relaxed), 0);
    assert_eq!(m.total_cpu_time_ms.load(Ordering::Relaxed), 0);
}

#[test]
fn function_metrics_atomic_counters() {
    let m = FunctionMetrics::default();
    m.total_requests.fetch_add(5, Ordering::Relaxed);
    m.active_requests.fetch_add(2, Ordering::Relaxed);
    m.total_errors.fetch_add(1, Ordering::Relaxed);
    m.total_cpu_time_ms.fetch_add(100, Ordering::Relaxed);
    let snap = m.snapshot();
    assert_eq!(snap.total_requests, 5);
    assert_eq!(snap.active_requests, 2);
    assert_eq!(snap.total_errors, 1);
    assert_eq!(snap.total_cpu_time_ms, 100);
}

#[test]
fn function_metrics_snapshot_serializes() {
    let m = FunctionMetrics::default();
    m.total_requests.fetch_add(10, Ordering::Relaxed);
    let snap = m.snapshot();
    let json = serde_json::to_string(&snap).unwrap();
    assert!(json.contains("\"total_requests\":10"));
}

#[test]
fn function_entry_to_info() {
    let metrics = Arc::new(FunctionMetrics::default());
    metrics.total_requests.fetch_add(3, Ordering::Relaxed);
    let now = Utc::now();
    let entry = FunctionEntry {
        name: "test-fn".to_string(),
        bundle_package_bytes: Bytes::new(),
        eszip_bytes: Bytes::new(),
        bundle_format: BundleFormat::Eszip,
        package_v8_version: get_v8_version().to_string(),
        isolate_handle: None,
        extra_isolate_handles: Vec::new(),
        pool_limits: PoolLimits::default(),
        next_handle_index: 0,
        inspector_stop: None,
        status: FunctionStatus::Running,
        config: IsolateConfig::default(),
        manifest: None,
        route_metadata: None,
        metrics,
        created_at: now,
        updated_at: now,
        last_error: None,
    };
    let info = entry.to_info();
    assert_eq!(info.name, "test-fn");
    assert_eq!(info.status, FunctionStatus::Running);
    assert_eq!(info.metrics.total_requests, 3);
    assert_eq!(info.bundle_format, BundleFormat::Eszip);
    assert_eq!(info.package_v8_version, get_v8_version());
    assert_eq!(info.runtime_v8_version, get_v8_version());
    assert!(info.snapshot_compatible_with_runtime);
    assert!(!info.requires_snapshot_regeneration);
    assert_eq!(info.stored_eszip_size_bytes, 0);
    assert!(!info.can_regenerate_snapshot_from_stored_eszip);
    assert!(info.last_error.is_none());
}

#[test]
fn deploy_request_deserialization() {
    let json = r#"{"name":"my-func"}"#;
    let req: DeployRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "my-func");
    assert!(req.eszip_bytes.is_empty());
}
