use std::collections::HashMap;
use std::net::IpAddr;

use anyhow::Error;
use ipnet::IpNet;
use jsonschema::{Draft, Resource, Validator};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ssrf::DEFAULT_DENY_RANGES;

const COMMON_SCHEMA_URI: &str = "https://thunder.dev/schemas/base/common.schema.json";
const NETWORK_SCHEMA_URI: &str = "https://thunder.dev/schemas/base/network.schema.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionManifest {
    pub manifest_version: u32,
    pub name: String,
    pub entrypoint: String,
    #[serde(default)]
    pub flavor: Option<ManifestFlavor>,
    #[serde(default)]
    pub routes: Vec<ManifestRoute>,
    #[serde(default)]
    pub env: Option<ManifestEnv>,
    pub network: ManifestNetwork,
    #[serde(default)]
    pub resources: Option<ManifestResources>,
    #[serde(default)]
    pub auth: Option<ManifestAuth>,
    #[serde(default)]
    pub observability: Option<ManifestObservability>,
    #[serde(default)]
    pub profiles: HashMap<String, ManifestProfile>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestFlavor {
    Single,
    RoutedApp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRoute {
    pub kind: ManifestRouteKind,
    pub path: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub asset_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ManifestRouteKind {
    Function,
    Asset,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEnv {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestNetwork {
    pub mode: String,
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestResources {
    pub max_heap_mi_b: Option<u64>,
    pub cpu_time_ms: Option<u64>,
    pub wall_clock_timeout_ms: Option<u64>,
    pub vfs_total_quota_bytes: Option<u64>,
    pub vfs_max_file_bytes: Option<u64>,
    pub egress_max_requests_per_execution: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestAuth {
    pub verify_jwt: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestObservability {
    pub log_level: Option<String>,
    pub trace_sample_percent: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProfile {
    #[serde(default)]
    pub env: Option<ManifestProfileEnv>,
    #[serde(default)]
    pub network: Option<ManifestProfileNetwork>,
    #[serde(default)]
    pub resources: Option<ManifestResources>,
    #[serde(default)]
    pub auth: Option<ManifestAuth>,
    #[serde(default)]
    pub observability: Option<ManifestObservability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProfileEnv {
    #[serde(default)]
    pub allow: Option<Vec<String>>,
    #[serde(default)]
    pub secret_refs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProfileNetwork {
    #[serde(default)]
    pub allow: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFunctionManifest {
    pub manifest_version: u32,
    pub name: String,
    pub entrypoint: String,
    pub selected_profile: Option<String>,
    pub env_allow: Vec<String>,
    pub env_secret_refs: Vec<String>,
    pub network_allow: Vec<String>,
    pub resources: ManifestResources,
    pub auth: ManifestAuth,
    pub observability: ManifestObservability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingManifest {
    pub manifest_version: u32,
    pub routes: Vec<RoutingRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRule {
    pub host: String,
    pub path: String,
    pub target_function: String,
}

#[derive(Debug, Clone)]
enum ParsedNetworkTarget {
    Host(String),
    Ip(IpAddr),
    Cidr(IpNet),
}

#[derive(Debug, Clone)]
enum DenyRule {
    Ip(IpAddr),
    Cidr(IpNet),
}

static DENY_RULES: Lazy<Vec<DenyRule>> = Lazy::new(|| {
    DEFAULT_DENY_RANGES
        .iter()
        .filter_map(|raw| {
            let normalized = raw.trim().trim_start_matches('[').trim_end_matches(']');
            if let Ok(ip) = normalized.parse::<IpAddr>() {
                return Some(DenyRule::Ip(ip));
            }
            if let Ok(cidr) = normalized.parse::<IpNet>() {
                return Some(DenyRule::Cidr(cidr));
            }
            None
        })
        .collect()
});

static MANIFEST_V2_VALIDATOR: Lazy<Validator> = Lazy::new(|| {
    let common_schema: Value =
        serde_json::from_str(include_str!("../../../schemas/base/common.schema.json"))
            .expect("valid common schema JSON");
    let network_schema: Value =
        serde_json::from_str(include_str!("../../../schemas/base/network.schema.json"))
            .expect("valid network schema JSON");
    let manifest_schema: Value =
        serde_json::from_str(include_str!("../../../schemas/function-manifest.v2.schema.json"))
            .expect("valid manifest schema JSON");

    let mut options = jsonschema::options().with_draft(Draft::Draft202012);
    options = options.with_resource(
        COMMON_SCHEMA_URI,
        Resource::from_contents(common_schema).expect("valid common schema resource"),
    );
    options = options.with_resource(
        NETWORK_SCHEMA_URI,
        Resource::from_contents(network_schema).expect("valid network schema resource"),
    );

    options
        .build(&manifest_schema)
        .expect("valid v2 manifest validator")
});

static ROUTING_MANIFEST_VALIDATOR: Lazy<Validator> = Lazy::new(|| {
    let common_schema: Value =
        serde_json::from_str(include_str!("../../../schemas/base/common.schema.json"))
            .expect("valid common schema JSON");
    let manifest_schema: Value = serde_json::from_str(include_str!(
        "../../../schemas/routing-manifest.v1.schema.json"
    ))
    .expect("valid routing manifest schema JSON");

    let mut options = jsonschema::options().with_draft(Draft::Draft202012);
    options = options.with_resource(
        COMMON_SCHEMA_URI,
        Resource::from_contents(common_schema).expect("valid common schema resource"),
    );

    options
        .build(&manifest_schema)
        .expect("valid routing manifest validator")
});

pub fn validate_manifest_json(manifest_json: &str) -> Result<FunctionManifest, Error> {
    let manifest_value: Value = serde_json::from_str(manifest_json)
        .map_err(|e| anyhow::anyhow!("manifest is not valid JSON: {e}"))?;

    let manifest_version = extract_manifest_version(&manifest_value)?;
    if manifest_version != 2 {
        return Err(anyhow::anyhow!(
            "manifestVersion {} is not supported; only manifestVersion 2 is accepted",
            manifest_version
        ));
    }

    let schema_errors: Vec<String> = MANIFEST_V2_VALIDATOR
        .iter_errors(&manifest_value)
        .map(|err| err.to_string())
        .collect();
    if !schema_errors.is_empty() {
        return Err(anyhow::anyhow!(
            "manifest schema validation failed: {}",
            schema_errors.join("; ")
        ));
    }

    let manifest: FunctionManifest = serde_json::from_value(manifest_value)
        .map_err(|e| anyhow::anyhow!("manifest parsing failed after schema validation: {e}"))?;

    validate_manifest_semantics(&manifest)?;

    Ok(manifest)
}

fn extract_manifest_version(manifest_value: &Value) -> Result<u32, Error> {
    manifest_value
        .get("manifestVersion")
        .and_then(Value::as_u64)
        .map(|value| value as u32)
        .ok_or_else(|| anyhow::anyhow!("manifestVersion must be an integer"))
}

pub fn parse_validate_and_resolve_manifest(
    manifest_json: &str,
    profile: Option<&str>,
) -> Result<ResolvedFunctionManifest, Error> {
    let manifest = validate_manifest_json(manifest_json)?;
    resolve_manifest_for_profile(&manifest, profile)
}

pub fn validate_routing_manifest_json(manifest_json: &str) -> Result<RoutingManifest, Error> {
    let manifest_value: Value = serde_json::from_str(manifest_json)
        .map_err(|e| anyhow::anyhow!("routing manifest is not valid JSON: {e}"))?;

    let schema_errors: Vec<String> = ROUTING_MANIFEST_VALIDATOR
        .iter_errors(&manifest_value)
        .map(|err| err.to_string())
        .collect();
    if !schema_errors.is_empty() {
        return Err(anyhow::anyhow!(
            "routing manifest schema validation failed: {}",
            schema_errors.join("; ")
        ));
    }

    let manifest: RoutingManifest = serde_json::from_value(manifest_value).map_err(|e| {
        anyhow::anyhow!("routing manifest parsing failed after schema validation: {e}")
    })?;

    validate_routing_manifest_semantics(&manifest)?;
    Ok(manifest)
}

pub fn resolve_manifest_for_profile(
    manifest: &FunctionManifest,
    profile: Option<&str>,
) -> Result<ResolvedFunctionManifest, Error> {
    let selected_profile = if let Some(profile_name) = profile {
        if !manifest.profiles.contains_key(profile_name) {
            return Err(anyhow::anyhow!(
                "manifest profile '{}' was not found",
                profile_name
            ));
        }
        Some(profile_name.to_string())
    } else {
        None
    };

    let profile = selected_profile
        .as_ref()
        .and_then(|profile_name| manifest.profiles.get(profile_name));

    let mut env = manifest.env.clone().unwrap_or_default();
    let mut network_allow = manifest.network.allow.clone();
    let mut resources = manifest.resources.clone().unwrap_or_default();
    let mut auth = manifest.auth.clone().unwrap_or_default();
    let mut observability = manifest.observability.clone().unwrap_or_default();

    if let Some(profile_overrides) = profile {
        if let Some(profile_env) = &profile_overrides.env {
            if let Some(allow) = &profile_env.allow {
                env.allow = allow.clone();
            }
            if let Some(secret_refs) = &profile_env.secret_refs {
                env.secret_refs = secret_refs.clone();
            }
        }

        if let Some(profile_network) = &profile_overrides.network {
            if let Some(allow) = &profile_network.allow {
                network_allow = allow.clone();
            }
        }

        if let Some(profile_resources) = &profile_overrides.resources {
            if let Some(max_heap_mi_b) = profile_resources.max_heap_mi_b {
                resources.max_heap_mi_b = Some(max_heap_mi_b);
            }
            if let Some(cpu_time_ms) = profile_resources.cpu_time_ms {
                resources.cpu_time_ms = Some(cpu_time_ms);
            }
            if let Some(wall_clock_timeout_ms) = profile_resources.wall_clock_timeout_ms {
                resources.wall_clock_timeout_ms = Some(wall_clock_timeout_ms);
            }
            if let Some(vfs_total_quota_bytes) = profile_resources.vfs_total_quota_bytes {
                resources.vfs_total_quota_bytes = Some(vfs_total_quota_bytes);
            }
            if let Some(vfs_max_file_bytes) = profile_resources.vfs_max_file_bytes {
                resources.vfs_max_file_bytes = Some(vfs_max_file_bytes);
            }
            if let Some(egress_max_requests_per_execution) =
                profile_resources.egress_max_requests_per_execution
            {
                resources.egress_max_requests_per_execution =
                    Some(egress_max_requests_per_execution);
            }
        }

        if let Some(profile_auth) = &profile_overrides.auth {
            if let Some(verify_jwt) = profile_auth.verify_jwt {
                auth.verify_jwt = Some(verify_jwt);
            }
        }

        if let Some(profile_observability) = &profile_overrides.observability {
            if let Some(log_level) = &profile_observability.log_level {
                observability.log_level = Some(log_level.clone());
            }
            if let Some(trace_sample_percent) = profile_observability.trace_sample_percent {
                observability.trace_sample_percent = Some(trace_sample_percent);
            }
        }
    }

    validate_network_targets(&network_allow)?;

    Ok(ResolvedFunctionManifest {
        manifest_version: manifest.manifest_version,
        name: manifest.name.clone(),
        entrypoint: manifest.entrypoint.clone(),
        selected_profile,
        env_allow: dedupe_preserve_order(&env.allow),
        env_secret_refs: dedupe_preserve_order(&env.secret_refs),
        network_allow: dedupe_preserve_order(&network_allow),
        resources,
        auth,
        observability,
    })
}

fn dedupe_preserve_order(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            result.push(value.clone());
        }
    }
    result
}

fn validate_manifest_semantics(manifest: &FunctionManifest) -> Result<(), Error> {
    validate_network_targets(&manifest.network.allow)?;
    if manifest.manifest_version != 2 {
        return Err(anyhow::anyhow!(
            "manifestVersion {} is not supported; only manifestVersion 2 is accepted",
            manifest.manifest_version
        ));
    }

    let flavor = manifest
        .flavor
        .ok_or_else(|| anyhow::anyhow!("manifestVersion 2 requires field 'flavor'"))?;

    match flavor {
        ManifestFlavor::Single => {
            if !manifest.routes.is_empty() {
                return Err(anyhow::anyhow!(
                    "manifest flavor 'single' must not define routes"
                ));
            }
        }
        ManifestFlavor::RoutedApp => {
            if manifest.routes.is_empty() {
                return Err(anyhow::anyhow!(
                    "manifest flavor 'routed-app' requires at least one route"
                ));
            }
        }
    }

    validate_manifest_routes(&manifest.routes)?;

    for (profile_name, profile) in &manifest.profiles {
        if let Some(profile_network) = &profile.network {
            if let Some(allow) = &profile_network.allow {
                validate_network_targets(allow).map_err(|e| {
                    anyhow::anyhow!(
                        "profile '{}' has invalid network.allow entry: {}",
                        profile_name,
                        e
                    )
                })?;
            }
        }
    }

    Ok(())
}

fn validate_routing_manifest_semantics(manifest: &RoutingManifest) -> Result<(), Error> {
    let mut seen = std::collections::HashSet::new();
    for route in &manifest.routes {
        let key = (route.host.to_ascii_lowercase(), route.path.clone());
        if !seen.insert(key) {
            return Err(anyhow::anyhow!(
                "routing manifest has duplicate host+path entry: {} {}",
                route.host,
                route.path
            ));
        }
    }

    Ok(())
}

fn validate_manifest_routes(routes: &[ManifestRoute]) -> Result<(), Error> {
    let mut seen = std::collections::HashSet::new();
    for route in routes {
        if !route.path.starts_with('/') {
            return Err(anyhow::anyhow!(
                "manifest route path '{}' must start with '/'",
                route.path
            ));
        }

        let mut normalized_methods = Vec::new();
        for method in &route.methods {
            let normalized = method.to_ascii_uppercase();
            let is_valid = matches!(
                normalized.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
            );
            if !is_valid {
                return Err(anyhow::anyhow!(
                    "manifest route '{}' has unsupported HTTP method '{}'",
                    route.path,
                    method
                ));
            }
            if !normalized_methods.contains(&normalized) {
                normalized_methods.push(normalized);
            }
        }
        normalized_methods.sort();

        match route.kind {
            ManifestRouteKind::Function => {
                if route
                    .entrypoint
                    .as_deref()
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(anyhow::anyhow!(
                        "function route '{}' requires non-empty entrypoint",
                        route.path
                    ));
                }
                if route.asset_dir.is_some() {
                    return Err(anyhow::anyhow!(
                        "function route '{}' cannot define assetDir",
                        route.path
                    ));
                }
            }
            ManifestRouteKind::Asset => {
                if route
                    .asset_dir
                    .as_deref()
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(anyhow::anyhow!(
                        "asset route '{}' requires non-empty assetDir",
                        route.path
                    ));
                }
                if route.entrypoint.is_some() {
                    return Err(anyhow::anyhow!(
                        "asset route '{}' cannot define entrypoint",
                        route.path
                    ));
                }
                if !route.methods.is_empty()
                    && route
                        .methods
                        .iter()
                        .any(|method| !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD"))
                {
                    return Err(anyhow::anyhow!(
                        "asset route '{}' only supports GET/HEAD methods",
                        route.path
                    ));
                }
            }
        }

        let methods_key = if normalized_methods.is_empty() {
            "*".to_string()
        } else {
            normalized_methods.join(",")
        };
        let dedupe_key = (route.path.clone(), methods_key, route.kind);
        if !seen.insert(dedupe_key) {
            return Err(anyhow::anyhow!(
                "manifest has duplicate route definition for path '{}'",
                route.path
            ));
        }
    }

    Ok(())
}

fn validate_network_targets(targets: &[String]) -> Result<(), Error> {
    for target in targets {
        if target == "*" {
            return Err(anyhow::anyhow!(
                "wildcard '*' is not allowed in network.allow"
            ));
        }

        let parsed = parse_network_target(target)?;
        if collides_with_denylist(&parsed) {
            return Err(anyhow::anyhow!(
                "network.allow target '{}' collides with internal denylist",
                target
            ));
        }
    }

    Ok(())
}

fn parse_network_target(raw_target: &str) -> Result<ParsedNetworkTarget, Error> {
    let target = raw_target.trim();
    if target.is_empty() {
        return Err(anyhow::anyhow!("empty network target"));
    }

    let core = strip_optional_port(target)?;

    if let Ok(cidr) = core.parse::<IpNet>() {
        return Ok(ParsedNetworkTarget::Cidr(cidr));
    }

    if let Ok(ip) = core.parse::<IpAddr>() {
        return Ok(ParsedNetworkTarget::Ip(ip));
    }

    let host = core.to_ascii_lowercase();
    if host.chars().any(char::is_whitespace) {
        return Err(anyhow::anyhow!("host cannot contain whitespace"));
    }

    Ok(ParsedNetworkTarget::Host(host))
}

fn strip_optional_port(target: &str) -> Result<String, Error> {
    if target.starts_with('[') {
        let close_idx = target
            .find(']')
            .ok_or_else(|| anyhow::anyhow!("invalid bracketed target '{}'", target))?;

        let core = &target[1..close_idx];
        let remainder = &target[close_idx + 1..];
        if !remainder.is_empty() {
            let port = remainder
                .strip_prefix(':')
                .ok_or_else(|| anyhow::anyhow!("invalid port syntax in '{}'", target))?;
            validate_port(port)?;
        }

        return Ok(core.to_string());
    }

    if let Some((host_or_ip, maybe_port)) = target.rsplit_once(':') {
        let has_only_one_colon = host_or_ip.find(':').is_none();
        if has_only_one_colon && maybe_port.chars().all(|c| c.is_ascii_digit()) {
            validate_port(maybe_port)?;
            return Ok(host_or_ip.to_string());
        }
    }

    Ok(target.to_string())
}

fn validate_port(port_str: &str) -> Result<(), Error> {
    let port = port_str
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("invalid port '{}'", port_str))?;
    if port == 0 {
        return Err(anyhow::anyhow!("port 0 is not allowed"));
    }
    Ok(())
}

fn collides_with_denylist(target: &ParsedNetworkTarget) -> bool {
    match target {
        ParsedNetworkTarget::Host(host) => host == "localhost" || host == "localhost.",
        ParsedNetworkTarget::Ip(ip) => DENY_RULES.iter().any(|rule| match rule {
            DenyRule::Ip(deny_ip) => deny_ip == ip,
            DenyRule::Cidr(deny_cidr) => deny_cidr.contains(ip),
        }),
        ParsedNetworkTarget::Cidr(cidr) => DENY_RULES.iter().any(|rule| match rule {
            DenyRule::Ip(deny_ip) => cidr.contains(deny_ip),
            DenyRule::Cidr(deny_cidr) => {
                deny_cidr.contains(&cidr.network()) || cidr.contains(&deny_cidr.network())
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_minimal_manifest() {
        let json = r#"{
                    "$schema": "https://thunder.dev/schemas/function-manifest.v2.schema.json",
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "single",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["api.example.com:443", "8.8.8.8"]
                    }
                }"#;

        let manifest = validate_manifest_json(json).expect("manifest should validate");
        assert_eq!(manifest.name, "hello");
    }

    #[test]
    fn validates_minimal_manifest_v2_single() {
        let json = r#"{
                    "$schema": "https://thunder.dev/schemas/function-manifest.v2.schema.json",
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "single",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["api.example.com:443"]
                    }
                }"#;

        let manifest = validate_manifest_json(json).expect("manifest should validate");
        assert_eq!(manifest.manifest_version, 2);
        assert_eq!(manifest.flavor, Some(ManifestFlavor::Single));
        assert!(manifest.routes.is_empty());
    }

    #[test]
    fn validates_manifest_v2_routed_app_with_function_and_asset_routes() {
        let json = r#"{
                    "manifestVersion": 2,
                    "name": "hello-routed",
                    "entrypoint": "./fallback.ts",
                    "flavor": "routed-app",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["api.example.com:443"]
                    },
                    "routes": [
                        {
                            "kind": "function",
                            "path": "/api/users/:id",
                            "methods": ["GET", "POST"],
                            "entrypoint": "./functions/users.ts"
                        },
                        {
                            "kind": "asset",
                            "path": "/assets/*",
                            "assetDir": "./public"
                        }
                    ]
                }"#;

        let manifest = validate_manifest_json(json).expect("manifest should validate");
        assert_eq!(manifest.flavor, Some(ManifestFlavor::RoutedApp));
        assert_eq!(manifest.routes.len(), 2);
    }

    #[test]
    fn rejects_manifest_v2_single_with_routes() {
        let json = r#"{
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "single",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["api.example.com:443"]
                    },
                    "routes": [
                        {
                            "kind": "function",
                            "path": "/api/hello",
                            "entrypoint": "./functions/hello.ts"
                        }
                    ]
                }"#;

        let err = validate_manifest_json(json).expect_err("single flavor with routes must fail");
        assert!(err
            .to_string()
            .contains("flavor 'single' must not define routes")
            || err.to_string().contains("schema validation failed"));
    }

    #[test]
    fn rejects_manifest_v2_routed_app_without_routes() {
        let json = r#"{
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "routed-app",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["api.example.com:443"]
                    }
                }"#;

        let err = validate_manifest_json(json).expect_err("routed-app without routes must fail");
        assert!(err
            .to_string()
            .contains("flavor 'routed-app' requires at least one route")
            || err.to_string().contains("schema validation failed"));
    }

    #[test]
    fn rejects_manifest_with_denylisted_ip() {
        let json = r#"{
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "single",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["127.0.0.1"]
                    }
                }"#;

        let err = validate_manifest_json(json).expect_err("denylisted target must fail");
        assert!(err.to_string().contains("collides with internal denylist"));
    }

    #[test]
    fn rejects_manifest_with_wildcard_allow() {
        let json = r#"{
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "single",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["*"]
                    }
                }"#;

        let err = validate_manifest_json(json).expect_err("wildcard target must fail");
        assert!(
            err.to_string().contains("wildcard '*' is not allowed")
                || err.to_string().contains("schema validation failed")
        );
    }

    #[test]
    fn rejects_manifest_missing_required_network() {
        let json = r#"{
                    "manifestVersion": 2,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "flavor": "single"
                }"#;

        let err = validate_manifest_json(json).expect_err("schema should reject");
        assert!(err.to_string().contains("schema validation failed"));
    }

    #[test]
    fn resolves_profile_overrides_for_network_and_resources() {
        let json = r#"{
            "manifestVersion": 2,
            "name": "hello",
            "entrypoint": "./index.ts",
            "flavor": "single",
            "env": {
                "allow": ["LOG_LEVEL"],
                "secretRefs": ["BASE_SECRET"]
            },
            "network": {
                "mode": "allowlist",
                "allow": ["api.example.com:443"]
            },
            "resources": {
                "maxHeapMiB": 64,
                "cpuTimeMs": 100,
                "wallClockTimeoutMs": 200
            },
            "profiles": {
                "prod": {
                    "env": {
                        "allow": ["LOG_LEVEL", "FEATURE_FLAG"],
                        "secretRefs": ["PROD_SECRET"]
                    },
                    "network": {
                        "allow": ["payments.example.com:443"]
                    },
                    "resources": {
                        "maxHeapMiB": 256
                    }
                }
            }
        }"#;

        let resolved =
            parse_validate_and_resolve_manifest(json, Some("prod")).expect("must resolve");
        assert_eq!(resolved.selected_profile.as_deref(), Some("prod"));
        assert_eq!(resolved.network_allow, vec!["payments.example.com:443"]);
        assert_eq!(resolved.env_allow, vec!["LOG_LEVEL", "FEATURE_FLAG"]);
        assert_eq!(resolved.env_secret_refs, vec!["PROD_SECRET"]);
        assert_eq!(resolved.resources.max_heap_mi_b, Some(256));
        assert_eq!(resolved.resources.cpu_time_ms, Some(100));
        assert_eq!(resolved.resources.vfs_total_quota_bytes, None);
        assert_eq!(resolved.resources.vfs_max_file_bytes, None);
        assert_eq!(resolved.resources.egress_max_requests_per_execution, None);
    }

    #[test]
    fn rejects_unknown_profile() {
        let json = r#"{
            "manifestVersion": 2,
            "name": "hello",
            "entrypoint": "./index.ts",
            "flavor": "single",
            "network": {
                "mode": "allowlist",
                "allow": ["api.example.com:443"]
            },
            "profiles": {
                "dev": {
                    "network": {
                        "allow": ["dev.example.com:443"]
                    }
                }
            }
        }"#;

        let err = parse_validate_and_resolve_manifest(json, Some("prod"))
            .expect_err("must reject unknown profile");
        assert!(err.to_string().contains("was not found"));
    }

    #[test]
    fn rejects_manifest_v1_as_no_longer_supported() {
        let json = r#"{
                    "manifestVersion": 1,
                    "name": "hello",
                    "entrypoint": "./index.ts",
                    "network": {
                        "mode": "allowlist",
                        "allow": ["api.example.com:443"]
                    }
                }"#;

        let err = validate_manifest_json(json).expect_err("v1 must be rejected");
        assert!(err
            .to_string()
            .contains("only manifestVersion 2 is accepted"));
    }

    #[test]
    fn validates_minimal_routing_manifest() {
        let json = r#"{
            "manifestVersion": 1,
            "routes": [
                {
                    "host": "api.example.com",
                    "path": "/users/:id",
                    "targetFunction": "users-api"
                }
            ]
        }"#;

        let manifest =
            validate_routing_manifest_json(json).expect("routing manifest should validate");
        assert_eq!(manifest.routes.len(), 1);
        assert_eq!(manifest.routes[0].target_function, "users-api");
    }

    #[test]
    fn rejects_routing_manifest_with_duplicate_host_and_path() {
        let json = r#"{
            "manifestVersion": 1,
            "routes": [
                {
                    "host": "api.example.com",
                    "path": "/users/:id",
                    "targetFunction": "users-api"
                },
                {
                    "host": "API.EXAMPLE.COM",
                    "path": "/users/:id",
                    "targetFunction": "users-api-v2"
                }
            ]
        }"#;

        let err = validate_routing_manifest_json(json)
            .expect_err("duplicate host+path should be rejected");
        assert!(err
            .to_string()
            .contains("routing manifest has duplicate host+path entry"));
    }
}
