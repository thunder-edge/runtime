use std::path::Path;
use std::sync::Arc;

use clap::Args;
use deno_ast::{EmitOptions, TranspileOptions};
use deno_graph::ast::CapturingModuleAnalyzer;
use deno_graph::source::{LoadError, LoadOptions, LoadResponse, Loader};
use deno_graph::{BuildOptions, GraphKind, ModuleGraph};
use functions::types::BundlePackage;
use url::Url;

use super::check::{
    deno_binary_exists, run_deno_check_for_files, run_syntax_check_for_files_async,
};

#[derive(Args)]
pub struct BundleArgs {
    /// Entrypoint TypeScript/JavaScript file
    #[arg(short, long)]
    entrypoint: String,

    /// Output bundle file path
    #[arg(short, long)]
    output: String,

    /// Bundle format: eszip (default) or snapshot
    /// NOTE: Snapshot support requires deno_core improvements for dynamic snapshot loading
    #[arg(short, long, default_value = "eszip")]
    format: String,
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
            if specifier.scheme() == "edge" {
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
    let source = String::from_utf8_lossy(&content).to_string();
    if !source.contains("edge://assert/") {
        return Ok(content);
    }

    let cwd = std::env::current_dir().map_err(|e| {
        LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
            "failed to resolve current dir for edge:assert rewrite: {e}"
        ))))
    })?;

    let user_mod_path = cwd.join("crates/runtime-core/src/assert/user_mod.ts");
    let assert_path = cwd.join("crates/runtime-core/src/assert/assert.ts");

    let user_mod_url = Url::from_file_path(&user_mod_path).map_err(|()| {
        LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
            "failed to convert '{}' to file URL",
            user_mod_path.display()
        ))))
    })?;
    let assert_url = Url::from_file_path(&assert_path).map_err(|()| {
        LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
            "failed to convert '{}' to file URL",
            assert_path.display()
        ))))
    })?;

    let rewritten = source
        .replace("edge://assert/mod.ts", user_mod_url.as_str())
        .replace("edge://assert/assert.ts", assert_url.as_str());

    Ok(rewritten.into_bytes())
}

fn load_edge_assert_module(
    specifier: &deno_graph::ModuleSpecifier,
) -> Result<Option<Vec<u8>>, LoadError> {
    let relative_path = match specifier.as_str() {
        "edge://assert/mod.ts" => {
            return Ok(Some(
                b"export { AssertionError, assert, assertEquals, assertExists, assertNotEquals, assertRejects, assertThrows } from 'edge://assert/assert.ts';\n"
                    .to_vec(),
            ));
        }
        "edge://assert/assert.ts" => "crates/runtime-core/src/assert/assert.ts",
        _ => return Ok(None),
    };

    let cwd = std::env::current_dir().map_err(|e| {
        LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
            "failed to resolve current dir for edge:assert modules: {e}"
        ))))
    })?;

    let module_path = cwd.join(relative_path);
    let content = std::fs::read(&module_path).map_err(|e| {
        LoadError::Other(Arc::new(deno_error::JsErrorBox::generic(format!(
            "failed to read '{}': {e}",
            module_path.display()
        ))))
    })?;

    Ok(Some(content))
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

    // Validate format
    match args.format.as_str() {
        "eszip" => {}
        "snapshot" => {
            return Err(anyhow::anyhow!(
                "snapshot format not yet supported - deno_core needs improvements for dynamic snapshot loading. Use 'eszip' format instead."
            ));
        }
        _ => {
            return Err(anyhow::anyhow!(
                "invalid format '{}', must be 'eszip' or 'snapshot'",
                args.format
            ))
        }
    };

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
    let pkg = BundlePackage::eszip_only(eszip_bytes);
    let bundle_data = bincode::serialize(&pkg)?;

    std::fs::write(&args.output, &bundle_data)
        .map_err(|e| anyhow::anyhow!("failed to write bundle: {e}"))?;

    tracing::info!(
        "wrote {} bytes to '{}' (eszip format)",
        bundle_data.len(),
        args.output
    );

    Ok(())
}
