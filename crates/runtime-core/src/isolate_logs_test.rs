use super::*;

#[test]
fn collector_accepts_entries() {
    let before = collected_log_count();
    push_collected_log(IsolateConsoleLog {
        timestamp: Utc::now(),
        function_name: "test-fn".to_string(),
        request_id: "isolate-console".to_string(),
        level: 1,
        message: "hello".to_string(),
    });
    let after = collected_log_count();
    assert!(after >= before + 1);
}

#[test]
fn collector_drains_up_to_limit() {
    push_collected_log(IsolateConsoleLog {
        timestamp: Utc::now(),
        function_name: "test-fn".to_string(),
        request_id: "isolate-console".to_string(),
        level: 1,
        message: "one".to_string(),
    });
    push_collected_log(IsolateConsoleLog {
        timestamp: Utc::now(),
        function_name: "test-fn".to_string(),
        request_id: "isolate-console".to_string(),
        level: 1,
        message: "two".to_string(),
    });

    let drained = drain_collected_logs(1);
    assert_eq!(drained.len(), 1);
}
