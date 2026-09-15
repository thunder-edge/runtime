use super::*;

#[test]
fn global_routing_prefers_exact_host_and_static_path() {
    let raw = r#"{
        "manifestVersion": 1,
        "routes": [
            {"host": "*.example.com", "path": "/users/:id", "targetFunction": "wild"},
            {"host": "api.example.com", "path": "/users/:id", "targetFunction": "exact"},
            {"host": "api.example.com", "path": "/users/me", "targetFunction": "static"}
        ]
    }"#;

    let table = parse_global_routing_manifest(raw, "test").expect("manifest should parse");

    let matched = table
        .resolve(Some("api.example.com"), "/users/me")
        .expect("route should resolve");
    assert_eq!(matched.target_function, "static");
}

#[test]
fn global_routing_matches_wildcard_host() {
    let raw = r#"{
        "manifestVersion": 1,
        "routes": [
            {"host": "*.apps.example.com", "path": "/*", "targetFunction": "frontend"}
        ]
    }"#;

    let table = parse_global_routing_manifest(raw, "test").expect("manifest should parse");

    let matched = table
        .resolve(Some("team-a.apps.example.com:443"), "/assets/logo.svg")
        .expect("route should resolve");
    assert_eq!(matched.target_function, "frontend");
}

#[test]
fn global_routing_rejects_ambiguous_rules_same_precedence() {
    let raw = r#"{
        "manifestVersion": 1,
        "routes": [
            {"host": "api.example.com", "path": "/users/:id", "targetFunction": "a"},
            {"host": "api.example.com", "path": "/users/:slug", "targetFunction": "b"}
        ]
    }"#;

    let err = GlobalRoutingTable::from_manifest_json(raw, "test")
        .expect_err("ambiguous same-precedence rules must fail");
    assert!(err
        .to_string()
        .contains("ambiguous routing rules with same precedence"));
}
