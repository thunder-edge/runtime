use super::*;

fn record(path: &str, methods: &[&str], rank: u32) -> BundleRouteRecord {
    BundleRouteRecord {
        kind: ManifestRouteKind::Function,
        path: path.to_string(),
        methods: methods.iter().map(|m| (*m).to_string()).collect(),
        entrypoint: Some("./functions/index.ts".to_string()),
        asset_dir: None,
        precedence_rank: rank,
    }
}

#[test]
fn matches_static_then_dynamic_by_rank() {
    let metadata = BundleRouteMetadata {
        generated_at_unix_ms: 0,
        routes: vec![record("/:slug", &[], 2), record("/about", &["GET"], 1)],
    };

    let decision = match_suffix_route(&metadata, "/about", &Method::GET);
    match decision {
        RouteMatchDecision::Matched(route) => assert_eq!(route.path, "/about"),
        _ => panic!("expected static route match"),
    }
}

#[test]
fn returns_method_not_allowed_when_path_matches() {
    let metadata = BundleRouteMetadata {
        generated_at_unix_ms: 0,
        routes: vec![record("/api/users", &["GET", "POST"], 1)],
    };

    let decision = match_suffix_route(&metadata, "/api/users", &Method::DELETE);
    match decision {
        RouteMatchDecision::MethodNotAllowed { allow } => {
            assert_eq!(allow, vec!["GET", "POST"])
        }
        _ => panic!("expected 405 decision"),
    }
}

#[test]
fn returns_not_found_when_no_path_matches() {
    let metadata = BundleRouteMetadata {
        generated_at_unix_ms: 0,
        routes: vec![record("/api/users", &["GET"], 1)],
    };

    let decision = match_suffix_route(&metadata, "/api/posts", &Method::GET);
    assert!(matches!(decision, RouteMatchDecision::NotFound));
}
