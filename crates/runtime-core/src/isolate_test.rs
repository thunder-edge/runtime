use super::*;

#[test]
fn isolate_config_defaults() {
    let config = IsolateConfig::default();
    assert_eq!(config.max_heap_size_bytes, 128 * 1024 * 1024);
    assert_eq!(config.cpu_time_limit_ms, 50_000);
    assert_eq!(config.wall_clock_timeout_ms, 60_000);
    assert_eq!(config.inspect_port, None);
    assert!(!config.inspect_brk);
    assert!(config.enable_source_maps);
    assert!(config.ssrf_config.enabled);
    assert!(config.print_isolate_logs);
    assert_eq!(config.vfs_total_quota_bytes, 10 * 1024 * 1024);
    assert_eq!(config.vfs_max_file_bytes, 5 * 1024 * 1024);
    assert_eq!(config.dns_doh_endpoint, "https://1.1.1.1/dns-query");
    assert_eq!(config.dns_max_answers, 16);
    assert_eq!(config.dns_timeout_ms, 2000);
    assert_eq!(config.egress_max_requests_per_execution, 0);
    assert!(!config.context_pool_enabled);
    assert_eq!(config.max_contexts_per_isolate, 8);
    assert_eq!(config.max_active_requests_per_context, 1);
}

#[test]
fn isolate_config_serde_defaults() {
    let config: IsolateConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(config.max_heap_size_bytes, 128 * 1024 * 1024);
    assert_eq!(config.cpu_time_limit_ms, 50_000);
    assert_eq!(config.wall_clock_timeout_ms, 60_000);
    assert_eq!(config.inspect_port, None);
    assert!(!config.inspect_brk);
    assert!(config.enable_source_maps);
    assert!(config.ssrf_config.enabled);
    assert!(config.print_isolate_logs);
    assert_eq!(config.vfs_total_quota_bytes, 10 * 1024 * 1024);
    assert_eq!(config.vfs_max_file_bytes, 5 * 1024 * 1024);
    assert_eq!(config.dns_doh_endpoint, "https://1.1.1.1/dns-query");
    assert_eq!(config.dns_max_answers, 16);
    assert_eq!(config.dns_timeout_ms, 2000);
    assert_eq!(config.egress_max_requests_per_execution, 0);
    assert!(!config.context_pool_enabled);
    assert_eq!(config.max_contexts_per_isolate, 8);
    assert_eq!(config.max_active_requests_per_context, 1);
}

#[test]
fn isolate_config_serde_custom() {
    let json = r#"{"max_heap_size_bytes":999,"cpu_time_limit_ms":100,"wall_clock_timeout_ms":200,"inspect_port":9333,"inspect_brk":true,"enable_source_maps":false}"#;
    let config: IsolateConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.max_heap_size_bytes, 999);
    assert_eq!(config.cpu_time_limit_ms, 100);
    assert_eq!(config.wall_clock_timeout_ms, 200);
    assert_eq!(config.inspect_port, Some(9333));
    assert!(config.inspect_brk);
    assert!(!config.enable_source_maps);
}

#[test]
fn isolate_config_serializes() {
    let config = IsolateConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"max_heap_size_bytes\""));
    assert!(json.contains("\"cpu_time_limit_ms\""));
    assert!(json.contains("\"wall_clock_timeout_ms\""));
    assert!(json.contains("\"inspect_port\""));
    assert!(json.contains("\"inspect_brk\""));
    assert!(json.contains("\"enable_source_maps\""));
    assert!(json.contains("\"ssrf_config\""));
    assert!(json.contains("\"print_isolate_logs\""));
    assert!(json.contains("\"vfs_total_quota_bytes\""));
    assert!(json.contains("\"vfs_max_file_bytes\""));
    assert!(json.contains("\"dns_doh_endpoint\""));
    assert!(json.contains("\"dns_max_answers\""));
    assert!(json.contains("\"dns_timeout_ms\""));
    assert!(json.contains("\"egress_max_requests_per_execution\""));
    assert!(json.contains("\"context_pool_enabled\""));
    assert!(json.contains("\"max_contexts_per_isolate\""));
    assert!(json.contains("\"max_active_requests_per_context\""));
}

#[test]
fn default_entrypoint_is_index_ts() {
    let spec = default_entrypoint();
    assert_eq!(spec.as_str(), "file:///src/index.ts");
}

#[test]
fn response_stream_cancellation_wakes_waiters() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let completion = ResponseStreamCompletion::new();
    let waiter = completion.clone();

    runtime.block_on(async {
        let task = tokio::spawn(async move {
            waiter.wait_cancelled().await;
        });
        completion.cancel();
        tokio::time::timeout(std::time::Duration::from_millis(100), task)
            .await
            .expect("cancellation waiter did not wake")
            .expect("cancellation waiter task failed");
    });
}

#[test]
fn response_stream_completion_keeps_first_terminal_reason() {
    let completed = ResponseStreamCompletion::new();
    completed.complete();
    completed.cancel();
    assert!(completed.is_done());
    assert!(!completed.is_cancelled());

    let cancelled = ResponseStreamCompletion::new();
    cancelled.cancel();
    cancelled.complete();
    assert!(cancelled.is_done());
    assert!(cancelled.is_cancelled());
}

#[test]
fn response_stream_completion_handles_concurrent_terminal_race() {
    for _ in 0..256 {
        let completion = std::sync::Arc::new(ResponseStreamCompletion::new());
        let complete = completion.clone();
        let cancel = completion.clone();

        let complete_thread = std::thread::spawn(move || complete.complete());
        let cancel_thread = std::thread::spawn(move || cancel.cancel());

        complete_thread.join().expect("completion thread panicked");
        cancel_thread.join().expect("cancellation thread panicked");

        assert!(completion.is_done());
    }
}
