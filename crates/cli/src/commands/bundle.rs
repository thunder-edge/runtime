use std::path::Path;

use clap::Args;
use deno_ast::{EmitOptions, TranspileOptions};
use deno_graph::source::{LoadOptions, LoadResponse, Loader};
use deno_graph::{BuildOptions, CapturingModuleAnalyzer, GraphKind, ModuleGraph};
use functions::types::BundlePackage;
use url::Url;

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
            if specifier.scheme() != "file" {
                return Ok(None);
            }

            let path = specifier
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("invalid file URL: {specifier}"))?;

            let content = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", path.display()))?;

            Ok(Some(LoadResponse::Module {
                content: content.into(),
                specifier,
                maybe_headers: None,
            }))
        })
    }
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

    // Validate format
    match args.format.as_str() {
        "eszip" => {},
        "snapshot" => {
            return Err(anyhow::anyhow!(
                "snapshot format not yet supported - deno_core needs improvements for dynamic snapshot loading. Use 'eszip' format instead."
            ));
        },
        _ => return Err(anyhow::anyhow!("invalid format '{}', must be 'eszip' or 'snapshot'", args.format)),
    };

    // 1. Build module graph
    let loader = FileLoader;
    let analyzer = CapturingModuleAnalyzer::default();

    let mut graph = ModuleGraph::new(GraphKind::CodeOnly);
    graph
        .build(
            vec![root_url.clone()],
            &loader,
            BuildOptions {
                module_analyzer: &analyzer,
                ..Default::default()
            },
        )
        .await;

    graph.valid().map_err(|e| anyhow::anyhow!("module graph error: {e}"))?;

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
