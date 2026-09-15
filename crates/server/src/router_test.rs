use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn extract_simple() {
    let (name, path) = extract_function_and_path("/my-func/hello/world");
    assert_eq!(name, "my-func");
    assert_eq!(path, "/hello/world");
}

#[test]
fn extract_no_sub_path() {
    let (name, path) = extract_function_and_path("/my-func");
    assert_eq!(name, "my-func");
    assert_eq!(path, "/");
}

#[test]
fn extract_root() {
    let (name, path) = extract_function_and_path("/");
    assert_eq!(name, "");
    assert_eq!(path, "/");
}

#[test]
fn extract_deep() {
    let (name, path) = extract_function_and_path("/api/v1/users/123");
    assert_eq!(name, "api");
    assert_eq!(path, "/v1/users/123");
}

#[test]
fn json_response_content_type() {
    let resp = json_response(StatusCode::OK, r#"{"ok":true}"#);
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
}

#[test]
fn json_response_status() {
    let resp = json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#);
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[test]
fn sanitized_internal_error_contains_request_id() {
    let body = client_error_json(ClientError::InternalError, "req-123");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["error"], "internal_error");
    assert_eq!(parsed["request_id"], "req-123");
}

#[test]
fn truncate_for_log_keeps_short_message() {
    let msg = "short error";
    assert_eq!(truncate_for_log(msg, 1024), msg);
}

#[test]
fn truncate_for_log_limits_to_1kib() {
    let msg = "x".repeat(2000);
    let truncated = truncate_for_log(&msg, 1024);
    assert!(truncated.len() <= 1024);
    assert!(truncated.ends_with("... [truncated]"));
}

#[test]
fn function_name_validation_accepts_slug() {
    assert!(is_valid_function_name("my-function-01"));
}

#[test]
fn function_name_validation_rejects_invalid() {
    assert!(!is_valid_function_name(""));
    assert!(!is_valid_function_name("UpperCase"));
    assert!(!is_valid_function_name("name..dots"));
    assert!(!is_valid_function_name("with/slash"));
    assert!(!is_valid_function_name("função"));
    let too_long = "a".repeat(64);
    assert!(!is_valid_function_name(&too_long));
}

#[test]
fn slugify_normalizes_to_url_safe_slug() {
    assert_eq!(slugify_function_name(" My Func_v2 "), "my-func-v2");
    assert_eq!(
        slugify_function_name("api..gateway///edge"),
        "api-gateway-edge"
    );
    assert_eq!(
        normalize_function_name("___hello___"),
        Some("hello".to_string())
    );
}

#[tokio::test]
async fn metrics_cache_reuses_until_ttl_then_refreshes() {
    let cache = MetricsCache::new(Duration::from_millis(30));
    let calls = Arc::new(AtomicUsize::new(0));

    let c1 = calls.clone();
    let first = cache
        .get_or_compute(move || {
            let n = c1.fetch_add(1, Ordering::SeqCst) + 1;
            format!("payload-{n}")
        })
        .await;
    assert_eq!(first, "payload-1");

    let c2 = calls.clone();
    let second = cache
        .get_or_compute(move || {
            let n = c2.fetch_add(1, Ordering::SeqCst) + 1;
            format!("payload-{n}")
        })
        .await;
    assert_eq!(second, "payload-1");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    tokio::time::sleep(Duration::from_millis(40)).await;

    let c3 = calls.clone();
    let third = cache
        .get_or_compute(move || {
            let n = c3.fetch_add(1, Ordering::SeqCst) + 1;
            format!("payload-{n}")
        })
        .await;
    assert_eq!(third, "payload-2");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
