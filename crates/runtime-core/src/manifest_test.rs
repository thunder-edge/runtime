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
    assert!(
        err.to_string()
            .contains("flavor 'single' must not define routes")
            || err.to_string().contains("schema validation failed")
    );
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
    assert!(
        err.to_string()
            .contains("flavor 'routed-app' requires at least one route")
            || err.to_string().contains("schema validation failed")
    );
}

#[test]
fn rejects_manifest_with_denylisted_ip() {
    for target in [
        "127.0.0.1",
        "[::ffff:169.254.169.254]",
        "[fd00::1]",
        "[fe80::1]",
    ] {
        let json = format!(
            r#"{{
                "manifestVersion": 2,
                "name": "hello",
                "entrypoint": "./index.ts",
                "flavor": "single",
                "network": {{
                    "mode": "allowlist",
                    "allow": ["{target}"]
                }}
            }}"#
        );

        let err = validate_manifest_json(&json).expect_err("denylisted target must fail");
        assert!(
            err.to_string().contains("collides with internal denylist"),
            "target {target} returned unexpected error: {err}"
        );
    }
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

    let resolved = parse_validate_and_resolve_manifest(json, Some("prod")).expect("must resolve");
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

    let manifest = validate_routing_manifest_json(json).expect("routing manifest should validate");
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

    let err =
        validate_routing_manifest_json(json).expect_err("duplicate host+path should be rejected");
    assert!(err
        .to_string()
        .contains("routing manifest has duplicate host+path entry"));
}
