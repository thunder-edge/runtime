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
