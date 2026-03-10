use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Args;
use deno_ast::{EmitOptions, TranspileOptions};
use deno_graph::ast::CapturingModuleAnalyzer;
use deno_graph::source::{LoadError, LoadOptions, LoadResponse, Loader};
use deno_graph::{BuildOptions, GraphKind, ModuleGraph};
use functions::types::{BundlePackage, BundleRouteMetadata, BundleRouteRecord};
use runtime_core::manifest::{
    validate_manifest_json, FunctionManifest, ManifestFlavor, ManifestRoute,
    ManifestRouteKind,
};
use runtime_core::isolate::{IsolateConfig, OutgoingProxyConfig};
use url::Url;

use super::check::{
    deno_binary_exists, run_deno_check_for_files, run_syntax_check_for_files_async,
};
use super::embedded_assert;

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum BundleOutputFormat {
    /// Standard ESZIP package.
    Eszip,
    /// Snapshot-flavor envelope with ESZIP fallback.
    ///
    /// NOTE: currently packages per-module bytecode cache metadata that is
    /// consumed during ESZIP startup; static runtime startup snapshot remains
    /// in use.
    Snapshot,
}

#[derive(Args)]
pub struct BundleArgs {
    /// Entrypoint TypeScript/JavaScript file
    #[arg(short, long)]
    entrypoint: String,

    /// Output bundle file path
    #[arg(short, long)]
    output: String,

    /// Bundle output format.
    #[arg(long, value_enum, default_value = "eszip")]
    format: BundleOutputFormat,

    /// Optional function manifest (v2) file path.
    ///
    /// If provided and `flavor` is `routed-app` with empty `routes`, the CLI
    /// auto-scans a `functions/` directory and fills `routes[]` with function
    /// route entries.
    #[arg(long)]
    manifest: Option<String>,
}

/// A simple file-system loader for deno_graph.
///
/// Supports `file://` specifiers only — reads source files from local disk.
struct FileLoader;

impl Loader for FileLoader {
    fn load(
        &self,
        specifier: &deno_graph::ModuleSpecifier,
        _options: LoadOptions,
    ) -> deno_graph::source::LoadFuture {
        let specifier = specifier.clone();
        Box::pin(async move {
            if specifier.scheme() == "edge" || specifier.scheme() == "ext" {
                if let Some(content) = load_edge_assert_module(&specifier)? {
                    return Ok(Some(LoadResponse::Module {
                        content: content.into(),
                        specifier,
                        maybe_headers: None,
                        mtime: None,
                    }));
                }
            }

            if specifier.scheme() != "file" {
                return Ok(None);
            }

            let path = specifier.to_file_path().map_err(|()| {
                LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
                    "invalid file URL: {specifier}"
                ))))
            })?;

            let content = std::fs::read(&path).map_err(|e| {
                LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
                    "failed to read '{}': {e}",
                    path.display()
                ))))
            })?;

            let content = rewrite_edge_assert_imports(content)?;

            Ok(Some(LoadResponse::Module {
                content: content.into(),
                specifier,
                maybe_headers: None,
                mtime: None,
            }))
        })
    }
}

fn rewrite_edge_assert_imports(content: Vec<u8>) -> Result<Vec<u8>, LoadError> {
    Ok(embedded_assert::rewrite_edge_assert_imports(content))
}

fn load_edge_assert_module(
    specifier: &deno_graph::ModuleSpecifier,
) -> Result<Option<Vec<u8>>, LoadError> {
    embedded_assert::load_module_bytes(specifier)
}

pub fn run(args: BundleArgs) -> Result<(), anyhow::Error> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(run_async(args))
}

async fn run_async(args: BundleArgs) -> Result<(), anyhow::Error> {
    let entrypoint = Path::new(&args.entrypoint)
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve '{}': {e}", args.entrypoint))?;

    let root_url = Url::from_file_path(&entrypoint)
        .map_err(|()| anyhow::anyhow!("cannot convert path to URL: {}", entrypoint.display()))?;

    tracing::info!("bundling '{}' -> '{}'", root_url, args.output);

    let mut embedded_manifest_json: Option<String> = None;
    let mut embedded_route_metadata: Option<BundleRouteMetadata> = None;
    if let Some(manifest_path) = &args.manifest {
        let prepared = prepare_manifest_for_bundle(manifest_path, &entrypoint)?;
        embedded_manifest_json = Some(prepared.manifest_json);
        embedded_route_metadata = prepared.route_metadata;
    }

    // TS semantic typecheck (deno-check-like) before bundling.
    if matches!(
        entrypoint.extension().and_then(|e| e.to_str()),
        Some("ts") | Some("mts") | Some("cts") | Some("tsx")
    ) {
        if deno_binary_exists() {
            run_deno_check_for_files(&[entrypoint.clone()])?;
        } else {
            eprintln!(
                "warning: 'deno' binary not found in PATH. Falling back to syntax/module validation only (no TS semantic typecheck)."
            );
            run_syntax_check_for_files_async(&[entrypoint.clone()]).await?;
        }
    }

    // 1. Build module graph
    let loader = FileLoader;
    let analyzer = CapturingModuleAnalyzer::default();

    let mut graph = ModuleGraph::new(GraphKind::CodeOnly);
    graph
        .build(
            vec![root_url.clone()],
            vec![], // referrer imports
            &loader,
            BuildOptions {
                module_analyzer: &analyzer,
                ..Default::default()
            },
        )
        .await;

    graph
        .valid()
        .map_err(|e| anyhow::anyhow!("module graph error: {e}"))?;

    let module_count = graph.modules().count();
    tracing::info!("resolved {module_count} module(s)");

    // 2. Create eszip from graph
    let eszip = eszip::EszipV2::from_graph(eszip::FromGraphOptions {
        graph,
        parser: analyzer.as_capturing_parser(),
        module_kind_resolver: Default::default(),
        transpile_options: TranspileOptions::default(),
        emit_options: EmitOptions::default(),
        relative_file_base: None,
        npm_packages: None,
        npm_snapshot: Default::default(),
    })?;

    let eszip_bytes = eszip.into_bytes();

    // 3. Package and write bundle
    let mut pkg = match args.format {
        BundleOutputFormat::Eszip => BundlePackage::eszip_only(eszip_bytes),
        BundleOutputFormat::Snapshot => {
            let bytecode_cache = functions::snapshot::create_function_bytecode_cache_from_eszip(
                eszip_bytes.clone(),
                &IsolateConfig::default(),
                &OutgoingProxyConfig::default(),
                None,
                &args.entrypoint,
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to create function bytecode cache: {e}"))?;
            BundlePackage::snapshot_with_fallback(bytecode_cache, eszip_bytes)
        }
    };
    pkg.embedded_manifest_json = embedded_manifest_json;
    pkg.embedded_route_metadata = embedded_route_metadata;
    let bundle_data = bincode::serialize(&pkg)?;

    std::fs::write(&args.output, &bundle_data)
        .map_err(|e| anyhow::anyhow!("failed to write bundle: {e}"))?;

    tracing::info!(
        "wrote {} bytes to '{}' ({:?} format)",
        bundle_data.len(),
        args.output,
        args.format
    );

    Ok(())
}

struct PreparedManifest {
    manifest_json: String,
    route_metadata: Option<BundleRouteMetadata>,
}

fn prepare_manifest_for_bundle(
    manifest_path: &str,
    entrypoint: &Path,
) -> Result<PreparedManifest, anyhow::Error> {
    let manifest_path = Path::new(manifest_path);
    let raw = std::fs::read_to_string(manifest_path)
        .map_err(|e| anyhow::anyhow!("failed to read manifest '{}': {e}", manifest_path.display()))?;

    let mut manifest = validate_manifest_json(&raw)
        .map_err(|e| anyhow::anyhow!("invalid manifest '{}': {e}", manifest_path.display()))?;

    let mut manifest_changed = false;
    if manifest.flavor != Some(ManifestFlavor::RoutedApp) {
        return Ok(PreparedManifest {
            manifest_json: raw,
            route_metadata: None,
        });
    }

    if manifest.routes.is_empty() {
        let functions_dir = resolve_functions_dir(entrypoint)?;
        let routes = discover_function_routes(&functions_dir)?;
        if routes.is_empty() {
            return Err(anyhow::anyhow!(
                "manifest '{}' is routed-app, but no route files were found in '{}'.",
                manifest_path.display(),
                functions_dir.display()
            ));
        }
        manifest.routes = routes;
        manifest_changed = true;
    }

    let public_dir = resolve_public_dir(entrypoint);
    if let Some(public_dir) = public_dir {
        let public_routes = discover_public_asset_routes(&public_dir)?;
        if !public_routes.is_empty() {
            manifest.routes.extend(public_routes);
            manifest_changed = true;
        }
    }

    detect_route_collisions(&manifest)?;
    let route_metadata = build_route_metadata(&manifest)?;

    let serialized = serde_json::to_string_pretty(&manifest).map_err(|e| {
        anyhow::anyhow!(
            "failed to serialize manifest '{}': {e}",
            manifest_path.display()
        )
    })?;
    if manifest_changed {
        std::fs::write(manifest_path, &serialized).map_err(|e| {
            anyhow::anyhow!("failed to write manifest '{}': {e}", manifest_path.display())
        })?;
    }

    tracing::info!(
        "validated {} routed-app routes in '{}'",
        manifest.routes.len(),
        manifest_path.display()
    );

    Ok(PreparedManifest {
        manifest_json: serialized,
        route_metadata: Some(route_metadata),
    })
}

fn resolve_functions_dir(entrypoint: &Path) -> Result<PathBuf, anyhow::Error> {
    let parent = entrypoint.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot resolve parent directory for entrypoint '{}'",
            entrypoint.display()
        )
    })?;

    if parent.file_name().and_then(|n| n.to_str()) == Some("functions") {
        return Ok(parent.to_path_buf());
    }

    let candidate = parent.join("functions");
    if candidate.is_dir() {
        return Ok(candidate);
    }

    Err(anyhow::anyhow!(
        "routed-app manifest auto-scan requires a 'functions/' directory. Tried '{}'",
        candidate.display()
    ))
}

fn discover_function_routes(functions_dir: &Path) -> Result<Vec<ManifestRoute>, anyhow::Error> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_route_files(functions_dir, functions_dir, &mut files)?;
    files.sort();

    let mut routes = Vec::new();
    for file in files {
        let relative = file.strip_prefix(functions_dir).map_err(|e| {
            anyhow::anyhow!(
                "failed to build route path for '{}': {e}",
                file.display()
            )
        })?;

        let route_path = file_path_to_route(relative)?;
        routes.push(ManifestRoute {
            kind: ManifestRouteKind::Function,
            path: route_path,
            methods: Vec::new(),
            entrypoint: Some(format!("./{}", relative.to_string_lossy().replace('\\', "/"))),
            asset_dir: None,
        });
    }

    Ok(routes)
}

fn resolve_public_dir(entrypoint: &Path) -> Option<PathBuf> {
    let parent = entrypoint.parent()?;
    let candidate = if parent.file_name().and_then(|n| n.to_str()) == Some("functions") {
        parent.parent().map(|p| p.join("public"))
    } else {
        Some(parent.join("public"))
    }?;

    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

fn discover_public_asset_routes(public_dir: &Path) -> Result<Vec<ManifestRoute>, anyhow::Error> {
    let mut files = Vec::new();
    collect_all_files(public_dir, public_dir, &mut files)?;
    files.sort();

    let mut routes = Vec::new();
    for file in files {
        let relative = file.strip_prefix(public_dir).map_err(|e| {
            anyhow::anyhow!(
                "failed to build asset route path for '{}': {e}",
                file.display()
            )
        })?;
        let route_path = file_path_to_asset_route(relative)?;
        routes.push(ManifestRoute {
            kind: ManifestRouteKind::Asset,
            path: route_path,
            methods: vec!["GET".to_string(), "HEAD".to_string()],
            entrypoint: None,
            asset_dir: Some("./public".to_string()),
        });
    }

    Ok(routes)
}

fn collect_all_files(
    root: &Path,
    current: &Path,
    acc: &mut Vec<PathBuf>,
) -> Result<(), anyhow::Error> {
    for entry in std::fs::read_dir(current)
        .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", current.display()))?
    {
        let entry = entry
            .map_err(|e| anyhow::anyhow!("failed to read dir entry in '{}': {e}", current.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_all_files(root, &path, acc)?;
            continue;
        }

        if path.starts_with(root) {
            acc.push(path);
        }
    }

    Ok(())
}

fn file_path_to_asset_route(path: &Path) -> Result<String, anyhow::Error> {
    let as_str = path.to_string_lossy().replace('\\', "/");
    if as_str.is_empty() {
        return Err(anyhow::anyhow!("empty asset path"));
    }
    Ok(format!("/{}", as_str))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteClass {
    AssetExact,
    StaticExact,
    Dynamic,
    CatchAll,
}

#[derive(Clone)]
struct RouteShape {
    canonical: String,
    class: RouteClass,
    segment_count: usize,
    normalized_methods: Vec<String>,
}

fn detect_route_collisions(manifest: &FunctionManifest) -> Result<(), anyhow::Error> {
    for i in 0..manifest.routes.len() {
        for j in (i + 1)..manifest.routes.len() {
            let a = &manifest.routes[i];
            let b = &manifest.routes[j];
            let sa = route_shape(a)?;
            let sb = route_shape(b)?;

            if sa.canonical != sb.canonical {
                continue;
            }
            if !methods_overlap(&sa.normalized_methods, &sb.normalized_methods) {
                continue;
            }

            return Err(anyhow::anyhow!(
                "route collision detected between '{}' ({:?}) and '{}' ({:?})",
                a.path,
                a.kind,
                b.path,
                b.kind
            ));
        }
    }

    Ok(())
}

fn build_route_metadata(manifest: &FunctionManifest) -> Result<BundleRouteMetadata, anyhow::Error> {
    let mut ranked: Vec<(ManifestRoute, RouteShape)> = manifest
        .routes
        .iter()
        .cloned()
        .map(|route| {
            let shape = route_shape(&route)?;
            Ok((route, shape))
        })
        .collect::<Result<_, anyhow::Error>>()?;

    ranked.sort_by(|(ra, sa), (rb, sb)| {
        route_class_weight(sa.class)
            .cmp(&route_class_weight(sb.class))
            .then_with(|| sb.segment_count.cmp(&sa.segment_count))
            .then_with(|| sa.canonical.cmp(&sb.canonical))
            .then_with(|| format!("{:?}", ra.kind).cmp(&format!("{:?}", rb.kind)))
    });

    let routes = ranked
        .into_iter()
        .enumerate()
        .map(|(idx, (route, _shape))| BundleRouteRecord {
            kind: route.kind,
            path: route.path,
            methods: route.methods,
            entrypoint: route.entrypoint,
            asset_dir: route.asset_dir,
            precedence_rank: (idx as u32).saturating_add(1),
        })
        .collect();

    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system clock before unix epoch: {e}"))?
        .as_millis() as i64;

    Ok(BundleRouteMetadata {
        generated_at_unix_ms,
        routes,
    })
}

fn route_shape(route: &ManifestRoute) -> Result<RouteShape, anyhow::Error> {
    if !route.path.starts_with('/') {
        return Err(anyhow::anyhow!("route path '{}' must start with '/'", route.path));
    }

    let raw_segments: Vec<&str> = route
        .path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut normalized_segments = Vec::new();
    let mut has_dynamic = false;
    let mut has_catch_all = false;

    for seg in raw_segments.iter() {
        if *seg == "*" {
            has_catch_all = true;
            normalized_segments.push("*".to_string());
            continue;
        }

        if seg.starts_with(':') {
            has_dynamic = true;
            normalized_segments.push(":".to_string());
            continue;
        }

        normalized_segments.push((*seg).to_string());
    }

    let canonical = if normalized_segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", normalized_segments.join("/"))
    };

    let class = if route.kind == ManifestRouteKind::Asset && !has_dynamic && !has_catch_all {
        RouteClass::AssetExact
    } else if has_catch_all {
        RouteClass::CatchAll
    } else if has_dynamic {
        RouteClass::Dynamic
    } else {
        RouteClass::StaticExact
    };

    Ok(RouteShape {
        canonical,
        class,
        segment_count: raw_segments.len(),
        normalized_methods: normalize_methods(route),
    })
}

fn route_class_weight(class: RouteClass) -> u8 {
    match class {
        RouteClass::AssetExact => 0,
        RouteClass::StaticExact => 1,
        RouteClass::Dynamic => 2,
        RouteClass::CatchAll => 3,
    }
}

fn normalize_methods(route: &ManifestRoute) -> Vec<String> {
    if route.methods.is_empty() {
        return vec!["*".to_string()];
    }

    let mut methods: Vec<String> = route
        .methods
        .iter()
        .map(|m| m.trim().to_ascii_uppercase())
        .filter(|m| !m.is_empty())
        .collect();
    methods.sort();
    methods.dedup();
    methods
}

fn methods_overlap(a: &[String], b: &[String]) -> bool {
    if a.iter().any(|m| m == "*") || b.iter().any(|m| m == "*") {
        return true;
    }

    a.iter().any(|m| b.contains(m))
}

fn collect_route_files(
    root: &Path,
    current: &Path,
    acc: &mut Vec<PathBuf>,
) -> Result<(), anyhow::Error> {
    for entry in std::fs::read_dir(current)
        .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", current.display()))?
    {
        let entry = entry
            .map_err(|e| anyhow::anyhow!("failed to read dir entry in '{}': {e}", current.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_route_files(root, &path, acc)?;
            continue;
        }

        if !path.starts_with(root) {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        if matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mts" | "mjs" | "cts" | "cjs") {
            acc.push(path);
        }
    }

    Ok(())
}

fn file_path_to_route(path: &Path) -> Result<String, anyhow::Error> {
    let mut segments: Vec<String> = Vec::new();
    let mut comps: Vec<_> = path.components().collect();
    if comps.is_empty() {
        return Err(anyhow::anyhow!("empty route path"));
    }

    let last = comps
        .pop()
        .ok_or_else(|| anyhow::anyhow!("missing file component"))?;
    for c in comps {
        let part = c.as_os_str().to_string_lossy().to_string();
        segments.push(convert_route_segment(&part));
    }

    let stem = Path::new(last.as_os_str())
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid route file name"))?;

    if stem != "index" {
        segments.push(convert_route_segment(stem));
    }

    if segments.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", segments.join("/")))
    }
}

fn convert_route_segment(segment: &str) -> String {
    if let Some(rest) = segment
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
    {
        return format!(":{}", rest);
    }

    if let Some(rest) = segment
        .strip_prefix("[")
        .and_then(|s| s.strip_suffix("]"))
    {
        if rest.starts_with("...") {
            return "*".to_string();
        }
        return format!(":{}", rest);
    }

    segment.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_from_index_file_is_root() {
        let route = file_path_to_route(Path::new("index.ts")).expect("route");
        assert_eq!(route, "/");
    }

    #[test]
    fn route_from_dynamic_segments_is_normalized() {
        let route =
            file_path_to_route(Path::new("api/users/[id]/posts.ts")).expect("route");
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
}
