use super::*;
use crate::global_routing::GlobalRoutingTable;
use functions::registry::FunctionRegistry;
use runtime_core::isolate::IsolateConfig;
use tokio_util::sync::CancellationToken;

fn test_registry() -> Arc<FunctionRegistry> {
    Arc::new(FunctionRegistry::new(
        CancellationToken::new(),
        IsolateConfig::default(),
    ))
}

#[test]
fn test_internal_path_detection() {
    // Test the path detection logic
    let path = "/_internal/health";
    assert!(path.starts_with("/_internal"));

    let path = "/_internal/functions";
    assert!(path.starts_with("/_internal"));

    let path = "/my-function/hello";
    assert!(!path.starts_with("/_internal"));
}

#[test]
fn test_function_name_extraction() {
    // Test path segment extraction logic
    let path = "/my-func/hello/world";
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0], "");
    assert_eq!(segments[1], "my-func");
    assert_eq!(segments[2], "hello/world");
}

#[test]
fn test_function_name_extraction_no_subpath() {
    let path = "/my-func";
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1], "my-func");
}

#[test]
fn test_function_name_extraction_root() {
    let path = "/";
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    let function_name = if segments.len() >= 2 { segments[1] } else { "" };
    assert_eq!(function_name, "");
}

#[test]
fn test_path_rewrite() {
    let path = "/my-func/v1/users/123";
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    let forwarded_path = if segments.len() >= 3 {
        format!("/{}", segments[2])
    } else {
        "/".to_string()
    };
    assert_eq!(forwarded_path, "/v1/users/123");
}

#[test]
fn test_path_rewrite_no_subpath() {
    let path = "/my-func";
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    let forwarded_path = if segments.len() >= 3 {
        format!("/{}", segments[2])
    } else {
        "/".to_string()
    };
    assert_eq!(forwarded_path, "/");
}

#[test]
fn test_reject_invalid_function_name() {
    assert!(!crate::router::is_valid_function_name("Bad_Name"));
    assert!(!crate::router::is_valid_function_name("../admin"));
    assert!(!crate::router::is_valid_function_name(""));
}

#[test]
fn test_rate_limited_response_shape() {
    let resp = crate::middleware::rate_limited_response(1);
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "1");
}

#[test]
fn test_resolve_route_target_uses_global_stage0_when_matched() {
    let manifest = r#"{
        "manifestVersion": 1,
        "routes": [
            {
                "host": "api.example.com",
                "path": "/users/:id",
                "targetFunction": "users-api"
            }
        ]
    }"#;
    let table = GlobalRoutingTable::from_manifest_json(manifest, "test")
        .expect("global routing manifest should parse");

    let router = IngressRouter::new_with_global_routing(
        test_registry(),
        BodyLimitsConfig::default(),
        None,
        Some(table),
    );

    let resolved = router
        .resolve_route_target("/users/123", Some("api.example.com"))
        .expect("stage0 should resolve route");

    assert_eq!(resolved.0, "users-api");
    assert_eq!(resolved.1, "/users/123");
}

#[test]
fn test_resolve_route_target_falls_back_to_prefix_mode() {
    let router = IngressRouter::new_with_global_routing(
        test_registry(),
        BodyLimitsConfig::default(),
        None,
        None,
    );

    let resolved = router
        .resolve_route_target("/my-function/v1/ping", Some("api.example.com"))
        .expect("fallback routing should resolve");

    assert_eq!(resolved.0, "my-function");
    assert_eq!(resolved.1, "/v1/ping");
}
