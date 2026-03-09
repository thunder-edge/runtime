use std::cmp::Ordering;
use std::sync::{Arc, RwLock};

use base64::Engine;
use serde::Serialize;
use tracing::warn;

const ROUTING_MANIFEST_B64_ENV: &str = "EDGE_RUNTIME_ROUTING_MANIFEST_B64";
const ROUTING_MANIFEST_PATH_ENV: &str = "EDGE_RUNTIME_ROUTING_MANIFEST_PATH";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalRouteView {
    pub host: String,
    pub path: String,
    pub target_function: String,
}

#[derive(Debug, Clone)]
pub struct GlobalRoutingTable {
    source: String,
    rules: Vec<CompiledRule>,
    view_rules: Vec<GlobalRouteView>,
}

#[derive(Clone, Debug)]
pub struct GlobalRoutingState {
    inner: Arc<RwLock<Option<Arc<GlobalRoutingTable>>>>,
}

#[derive(Debug, Clone)]
pub struct ResolvedGlobalRoute {
    pub target_function: String,
}

#[derive(Debug, Clone)]
struct CompiledRule {
    host_raw: String,
    path_raw: String,
    host: HostPattern,
    path: PathPattern,
    target_function: String,
}

#[derive(Debug, Clone)]
enum HostPattern {
    Exact(String),
    WildcardSuffix(String),
}

#[derive(Debug, Clone)]
struct PathPattern {
    segments: Vec<PathSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSegment {
    Static(String),
    Param,
    CatchAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MatchRank {
    host_rank: u8,
    static_count: usize,
    dynamic_penalty: usize,
    catch_all_penalty: usize,
    segment_count: usize,
}

impl GlobalRoutingTable {
    pub fn from_manifest_json(raw: &str, source: &str) -> anyhow::Result<Self> {
        parse_global_routing_manifest(raw, source)
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn routes(&self) -> &[GlobalRouteView] {
        &self.view_rules
    }

    pub fn resolve(&self, host_header: Option<&str>, path: &str) -> Option<ResolvedGlobalRoute> {
        let host = normalize_host(host_header?)?;
        let path_segments = split_path(path);

        let mut best: Option<(&CompiledRule, MatchRank)> = None;

        for rule in &self.rules {
            if !rule.host.matches(&host) {
                continue;
            }
            if !rule.path.matches(&path_segments) {
                continue;
            }

            let rank = rule.rank();
            match &best {
                Some((_, best_rank)) if rank <= *best_rank => {}
                _ => best = Some((rule, rank)),
            }
        }

        best.map(|(rule, _)| ResolvedGlobalRoute {
            target_function: rule.target_function.clone(),
        })
    }
}

impl GlobalRoutingState {
    pub fn new(initial: Option<GlobalRoutingTable>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(initial.map(Arc::new))),
        }
    }

    pub fn get(&self) -> Option<Arc<GlobalRoutingTable>> {
        self.inner.read().ok().and_then(|guard| guard.clone())
    }

    pub fn replace_from_manifest_json(
        &self,
        raw: &str,
        source: &str,
    ) -> anyhow::Result<Arc<GlobalRoutingTable>> {
        let table = Arc::new(GlobalRoutingTable::from_manifest_json(raw, source)?);
        if let Ok(mut write) = self.inner.write() {
            *write = Some(table.clone());
        }
        Ok(table)
    }
}

impl CompiledRule {
    fn rank(&self) -> MatchRank {
        MatchRank {
            host_rank: self.host.rank(),
            static_count: self.path.static_count(),
            dynamic_penalty: self.path.dynamic_count(),
            catch_all_penalty: usize::from(self.path.has_catch_all()),
            segment_count: self.path.segments.len(),
        }
    }
}

impl HostPattern {
    fn parse(raw: &str) -> Option<Self> {
        let lowered = raw.trim().to_ascii_lowercase();
        if lowered.is_empty() {
            return None;
        }

        if let Some(suffix) = lowered.strip_prefix("*.") {
            if suffix.is_empty() {
                return None;
            }
            return Some(HostPattern::WildcardSuffix(suffix.to_string()));
        }

        Some(HostPattern::Exact(lowered))
    }

    fn matches(&self, host: &str) -> bool {
        match self {
            HostPattern::Exact(expected) => host == expected,
            HostPattern::WildcardSuffix(suffix) => {
                host == suffix || host.ends_with(&format!(".{suffix}"))
            }
        }
    }

    fn rank(&self) -> u8 {
        match self {
            HostPattern::Exact(_) => 2,
            HostPattern::WildcardSuffix(_) => 1,
        }
    }
}

impl PathPattern {
    fn parse(raw: &str) -> Option<Self> {
        if !raw.starts_with('/') {
            return None;
        }

        let segments = split_path(raw)
            .into_iter()
            .map(|segment| {
                if segment == "*" {
                    PathSegment::CatchAll
                } else if segment.starts_with(':') && segment.len() > 1 {
                    PathSegment::Param
                } else {
                    PathSegment::Static(segment.to_string())
                }
            })
            .collect::<Vec<_>>();

        if has_invalid_catch_all(&segments) {
            return None;
        }

        Some(Self { segments })
    }

    fn matches(&self, path_segments: &[&str]) -> bool {
        let mut i = 0usize;
        let mut j = 0usize;

        while i < self.segments.len() {
            match &self.segments[i] {
                PathSegment::CatchAll => return true,
                PathSegment::Static(expected) => {
                    let Some(actual) = path_segments.get(j) else {
                        return false;
                    };
                    if actual != expected {
                        return false;
                    }
                    i += 1;
                    j += 1;
                }
                PathSegment::Param => {
                    let Some(actual) = path_segments.get(j) else {
                        return false;
                    };
                    if actual.is_empty() {
                        return false;
                    }
                    i += 1;
                    j += 1;
                }
            }
        }

        j == path_segments.len()
    }

    fn static_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|s| matches!(s, PathSegment::Static(_)))
            .count()
    }

    fn dynamic_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|s| matches!(s, PathSegment::Param))
            .count()
    }

    fn has_catch_all(&self) -> bool {
        self.segments.iter().any(|s| matches!(s, PathSegment::CatchAll))
    }
}

fn has_invalid_catch_all(segments: &[PathSegment]) -> bool {
    let Some(pos) = segments.iter().position(|s| matches!(s, PathSegment::CatchAll)) else {
        return false;
    };
    pos + 1 != segments.len()
}

fn split_path(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
}

fn normalize_host(host_header: &str) -> Option<String> {
    let trimmed = host_header.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Drop optional port from Host header.
    let host = match trimmed.rsplit_once(':') {
        Some((left, right)) if !left.contains(':') && right.chars().all(|c| c.is_ascii_digit()) => {
            left
        }
        _ => trimmed,
    };

    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(host)
}

pub fn load_global_routing_table_from_env() -> Option<GlobalRoutingTable> {
    match try_load_global_routing_table_from_env() {
        Ok(value) => value,
        Err(err) => {
            warn!(
                function_name = "runtime",
                request_id = "system",
                "failed to load global routing manifest from env: {}",
                err
            );
            None
        }
    }
}

fn try_load_global_routing_table_from_env() -> anyhow::Result<Option<GlobalRoutingTable>> {
    if let Ok(encoded) = std::env::var(ROUTING_MANIFEST_B64_ENV) {
        if encoded.trim().is_empty() {
            return Ok(None);
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim())?;
        let raw = String::from_utf8(bytes)?;
        return parse_global_routing_manifest(&raw, &format!("env:{ROUTING_MANIFEST_B64_ENV}"))
            .map(Some);
    }

    if let Ok(path) = std::env::var(ROUTING_MANIFEST_PATH_ENV) {
        let path = path.trim();
        if path.is_empty() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)?;
        return parse_global_routing_manifest(&raw, &format!("file:{path}")).map(Some);
    }

    Ok(None)
}

fn parse_global_routing_manifest(raw: &str, source: &str) -> anyhow::Result<GlobalRoutingTable> {
    let parsed = runtime_core::manifest::validate_routing_manifest_json(raw)?;

    let mut rules = Vec::with_capacity(parsed.routes.len());
    let mut view_rules = Vec::with_capacity(parsed.routes.len());

    for route in parsed.routes {
        let host = HostPattern::parse(&route.host).ok_or_else(|| {
            anyhow::anyhow!("invalid host pattern in global routing manifest: {}", route.host)
        })?;
        let path = PathPattern::parse(&route.path).ok_or_else(|| {
            anyhow::anyhow!("invalid path pattern in global routing manifest: {}", route.path)
        })?;

        view_rules.push(GlobalRouteView {
            host: route.host.clone(),
            path: route.path.clone(),
            target_function: route.target_function.clone(),
        });

        rules.push(CompiledRule {
            host_raw: route.host,
            path_raw: route.path,
            host,
            path,
            target_function: route.target_function,
        });
    }

    validate_no_ambiguous_precedence(&rules)?;

    rules.sort_by(|a, b| compare_rank_desc(a.rank(), b.rank()));

    Ok(GlobalRoutingTable {
        source: source.to_string(),
        rules,
        view_rules,
    })
}

fn compare_rank_desc(a: MatchRank, b: MatchRank) -> Ordering {
    b.cmp(&a)
}

fn validate_no_ambiguous_precedence(rules: &[CompiledRule]) -> anyhow::Result<()> {
    for i in 0..rules.len() {
        for j in (i + 1)..rules.len() {
            let a = &rules[i];
            let b = &rules[j];

            if a.rank() != b.rank() {
                continue;
            }
            if !host_patterns_overlap(&a.host, &b.host) {
                continue;
            }
            if !path_patterns_overlap(&a.path, &b.path) {
                continue;
            }

            return Err(anyhow::anyhow!(
                "ambiguous routing rules with same precedence: ('{}' '{}') and ('{}' '{}')",
                a.host_raw,
                a.path_raw,
                b.host_raw,
                b.path_raw
            ));
        }
    }

    Ok(())
}

fn host_patterns_overlap(a: &HostPattern, b: &HostPattern) -> bool {
    match (a, b) {
        (HostPattern::Exact(x), HostPattern::Exact(y)) => x == y,
        (HostPattern::Exact(exact), HostPattern::WildcardSuffix(suffix))
        | (HostPattern::WildcardSuffix(suffix), HostPattern::Exact(exact)) => {
            exact == suffix || exact.ends_with(&format!(".{suffix}"))
        }
        (HostPattern::WildcardSuffix(a_suffix), HostPattern::WildcardSuffix(b_suffix)) => {
            a_suffix == b_suffix
                || a_suffix.ends_with(&format!(".{b_suffix}"))
                || b_suffix.ends_with(&format!(".{a_suffix}"))
        }
    }
}

fn path_patterns_overlap(a: &PathPattern, b: &PathPattern) -> bool {
    path_overlap_from(&a.segments, 0, &b.segments, 0)
}

fn path_overlap_from(a: &[PathSegment], ai: usize, b: &[PathSegment], bi: usize) -> bool {
    if ai < a.len() && matches!(a[ai], PathSegment::CatchAll) {
        return true;
    }
    if bi < b.len() && matches!(b[bi], PathSegment::CatchAll) {
        return true;
    }

    if ai == a.len() && bi == b.len() {
        return true;
    }
    if ai == a.len() || bi == b.len() {
        return false;
    }

    let compatible = match (&a[ai], &b[bi]) {
        (PathSegment::Static(x), PathSegment::Static(y)) => x == y,
        (PathSegment::Static(_), PathSegment::Param)
        | (PathSegment::Param, PathSegment::Static(_))
        | (PathSegment::Param, PathSegment::Param) => true,
        (PathSegment::CatchAll, _) | (_, PathSegment::CatchAll) => true,
    };

    compatible && path_overlap_from(a, ai + 1, b, bi + 1)
}

#[cfg(test)]
mod tests {
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
        assert!(err.to_string().contains("ambiguous routing rules with same precedence"));
    }
}
