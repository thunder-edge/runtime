use std::collections::BTreeSet;

use functions::types::{BundleRouteMetadata, BundleRouteRecord};
use http::Method;
use runtime_core::manifest::ManifestRouteKind;

#[derive(Debug, Clone)]
pub enum RouteMatchDecision {
    Matched(BundleRouteRecord),
    MethodNotAllowed { allow: Vec<String> },
    NotFound,
}

pub fn match_suffix_route(
    metadata: &BundleRouteMetadata,
    suffix_path: &str,
    method: &Method,
) -> RouteMatchDecision {
    let mut sorted = metadata.routes.clone();
    sorted.sort_by_key(|route| route.precedence_rank);

    let mut allow = BTreeSet::new();
    let method_upper = method.as_str().to_ascii_uppercase();
    let mut had_path_match = false;

    for route in sorted {
        if !path_matches(&route.path, suffix_path) {
            continue;
        }
        had_path_match = true;

        if method_allowed(&route, &method_upper) {
            return RouteMatchDecision::Matched(route);
        }

        for m in route.methods {
            let normalized = m.trim().to_ascii_uppercase();
            if !normalized.is_empty() {
                allow.insert(normalized);
            }
        }
    }

    if had_path_match {
        return RouteMatchDecision::MethodNotAllowed {
            allow: allow.into_iter().collect(),
        };
    }

    RouteMatchDecision::NotFound
}

pub fn is_asset_route(route: &BundleRouteRecord) -> bool {
    route.kind == ManifestRouteKind::Asset
}

fn method_allowed(route: &BundleRouteRecord, request_method: &str) -> bool {
    if route.methods.is_empty() {
        return true;
    }

    route
        .methods
        .iter()
        .any(|m| m.trim().eq_ignore_ascii_case(request_method))
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments = split_segments(pattern);
    let path_segments = split_segments(path);

    let mut i = 0usize;
    let mut j = 0usize;

    while i < pattern_segments.len() {
        let seg = pattern_segments[i];
        if seg == "*" {
            return true;
        }

        let Some(actual) = path_segments.get(j) else {
            return false;
        };

        if seg.starts_with(':') {
            if actual.is_empty() {
                return false;
            }
            i += 1;
            j += 1;
            continue;
        }

        if seg != *actual {
            return false;
        }

        i += 1;
        j += 1;
    }

    j == path_segments.len()
}

fn split_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
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
}
