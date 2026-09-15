use super::*;

#[test]
fn route_from_index_file_is_root() {
    let route = file_path_to_route(Path::new("index.ts")).expect("route");
    assert_eq!(route, "/");
}

#[test]
fn route_from_dynamic_segments_is_normalized() {
    let route = file_path_to_route(Path::new("api/users/[id]/posts.ts")).expect("route");
    assert_eq!(route, "/api/users/:id/posts");
}

#[test]
fn detects_collision_on_same_canonical_dynamic_path() {
    let manifest = FunctionManifest {
        manifest_version: 2,
        name: "app".to_string(),
        entrypoint: "./index.ts".to_string(),
        flavor: Some(ManifestFlavor::RoutedApp),
        routes: vec![
            ManifestRoute {
                kind: ManifestRouteKind::Function,
                path: "/users/:id".to_string(),
                methods: vec!["GET".to_string()],
                entrypoint: Some("./functions/users/[id].ts".to_string()),
                asset_dir: None,
            },
            ManifestRoute {
                kind: ManifestRouteKind::Function,
                path: "/users/:slug".to_string(),
                methods: vec!["GET".to_string()],
                entrypoint: Some("./functions/users/[slug].ts".to_string()),
                asset_dir: None,
            },
        ],
        env: None,
        network: runtime_core::manifest::ManifestNetwork {
            mode: "allowlist".to_string(),
            allow: vec!["api.example.com:443".to_string()],
        },
        resources: None,
        auth: None,
        observability: None,
        profiles: std::collections::HashMap::new(),
    };

    let err = detect_route_collisions(&manifest).expect_err("collision should fail");
    assert!(err.to_string().contains("route collision detected"));
}

#[test]
fn route_metadata_prioritizes_asset_before_dynamic_and_catchall() {
    let manifest = FunctionManifest {
        manifest_version: 2,
        name: "app".to_string(),
        entrypoint: "./index.ts".to_string(),
        flavor: Some(ManifestFlavor::RoutedApp),
        routes: vec![
            ManifestRoute {
                kind: ManifestRouteKind::Function,
                path: "/:slug".to_string(),
                methods: vec![],
                entrypoint: Some("./functions/[slug].ts".to_string()),
                asset_dir: None,
            },
            ManifestRoute {
                kind: ManifestRouteKind::Function,
                path: "/*".to_string(),
                methods: vec![],
                entrypoint: Some("./functions/[...all].ts".to_string()),
                asset_dir: None,
            },
            ManifestRoute {
                kind: ManifestRouteKind::Asset,
                path: "/logo.svg".to_string(),
                methods: vec!["GET".to_string(), "HEAD".to_string()],
                entrypoint: None,
                asset_dir: Some("./public".to_string()),
            },
        ],
        env: None,
        network: runtime_core::manifest::ManifestNetwork {
            mode: "allowlist".to_string(),
            allow: vec!["api.example.com:443".to_string()],
        },
        resources: None,
        auth: None,
        observability: None,
        profiles: std::collections::HashMap::new(),
    };

    let metadata = build_route_metadata(&manifest).expect("metadata should build");
    assert_eq!(metadata.routes.len(), 3);
    assert_eq!(metadata.routes[0].path, "/logo.svg");
    assert_eq!(metadata.routes[1].path, "/:slug");
    assert_eq!(metadata.routes[2].path, "/*");
}
