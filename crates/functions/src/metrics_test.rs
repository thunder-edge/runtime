use super::*;

#[test]
fn global_metrics_default_zeros() {
    let m = GlobalMetrics::default();
    let snap = m.snapshot();
    assert_eq!(snap.total_requests, 0);
    assert_eq!(snap.total_errors, 0);
}

#[test]
fn global_metrics_snapshot_reflects_updates() {
    let m = GlobalMetrics::default();
    m.total_requests.fetch_add(42, Ordering::Relaxed);
    m.total_errors.fetch_add(7, Ordering::Relaxed);
    let snap = m.snapshot();
    assert_eq!(snap.total_requests, 42);
    assert_eq!(snap.total_errors, 7);
}

#[test]
fn global_metrics_snapshot_serializes() {
    let snap = GlobalMetricsSnapshot {
        total_requests: 100,
        total_errors: 5,
    };
    let json = serde_json::to_string(&snap).unwrap();
    assert!(json.contains("\"total_requests\":100"));
    assert!(json.contains("\"total_errors\":5"));
}
