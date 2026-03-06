use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use clap::{ArgAction, Args};
use deno_graph::ast::CapturingModuleAnalyzer;
use deno_graph::source::{LoadError, LoadOptions, LoadResponse, Loader};
use deno_graph::{BuildOptions, GraphKind, ModuleGraph};
use glob::Pattern;
use url::Url;

#[derive(Args)]
pub struct CheckArgs {
    /// Path, directory or glob pattern (for example: ./functions/**/*.ts)
    #[arg(short, long, default_value = "./**/*.{ts,js,mts,mjs,tsx,jsx,cjs,cts}")]
    path: String,

    /// Ignore path/pattern (can be used multiple times)
    #[arg(short = 'i', long = "ignore", action = ArgAction::Append)]
    ignore: Vec<String>,
}

pub fn run(args: CheckArgs) -> Result<(), anyhow::Error> {
    let files = discover_source_files(&args.path, &args.ignore)?;
    if files.is_empty() {
        return Err(anyhow::anyhow!(
            "no source files found for '{}' (ignore: {:?})",
            args.path,
            args.ignore
        ));
    }

    run_deno_check_for_files(&files)
}

pub(crate) fn deno_binary_exists() -> bool {
    Command::new("deno").arg("--version").output().is_ok()
}

pub(crate) fn run_deno_check_for_files(files: &[PathBuf]) -> Result<(), anyhow::Error> {
    if !deno_binary_exists() {
        eprintln!(
            "warning: 'deno' binary not found in PATH. Falling back to syntax/module validation only (no TS semantic typecheck)."
        );
        return run_syntax_check_for_files(files);
    }

    let mut cmd = Command::new("deno");
    cmd.arg("check");
    for file in files {
        cmd.arg(file);
    }

    let output = cmd
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run deno check: {e}"))?;

    if output.status.success() {
        println!("Check passed for {} file(s)", files.len());
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Err(anyhow::anyhow!(
        "deno check failed\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    ))
}

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

fn run_syntax_check_for_files(files: &[PathBuf]) -> Result<(), anyhow::Error> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(run_syntax_check_for_files_async(files))
}

pub(crate) async fn run_syntax_check_for_files_async(
    files: &[PathBuf],
) -> Result<(), anyhow::Error> {
    let mut checked = 0usize;
    for file in files {
        let entrypoint = file
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("cannot resolve '{}': {e}", file.display()))?;
        let root_url = Url::from_file_path(&entrypoint).map_err(|()| {
            anyhow::anyhow!("cannot convert path to URL: {}", entrypoint.display())
        })?;

        let loader = FileLoader;
        let analyzer = CapturingModuleAnalyzer::default();

        let mut graph = ModuleGraph::new(GraphKind::CodeOnly);
        graph
            .build(
                vec![root_url],
                vec![],
                &loader,
                BuildOptions {
                    module_analyzer: &analyzer,
                    ..Default::default()
                },
            )
            .await;

        graph.valid().map_err(|e| {
            anyhow::anyhow!("syntax/module graph error for '{}': {e}", file.display())
        })?;

        checked += 1;
    }

    println!(
        "Syntax/module check passed for {} file(s) (TS semantic typecheck not available without deno)",
        checked
    );

    Ok(())
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
            return Ok(Some(b"export * from 'edge://assert/assert.ts';\n".to_vec()));
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

fn discover_source_files(
    path_or_pattern: &str,
    ignore_patterns: &[String],
) -> Result<Vec<PathBuf>, anyhow::Error> {
    let cwd = std::env::current_dir()?;
    let candidate = Path::new(path_or_pattern);

    let mut files = if is_glob_pattern(path_or_pattern) {
        collect_glob_matches(path_or_pattern)?
    } else if candidate.is_dir() {
        walk_directory_for_sources(candidate)
    } else if candidate.is_file() {
        vec![candidate.to_path_buf()]
    } else {
        return Err(anyhow::anyhow!(
            "path '{}' does not exist and is not a valid glob pattern",
            path_or_pattern
        ));
    };

    let ignore_matchers = compile_patterns(ignore_patterns)?;

    files.retain(|path| {
        is_supported_source_file(path) && !matches_ignore(path, &cwd, &ignore_matchers)
    });

    files.sort();
    files.dedup();

    Ok(files)
}

fn is_glob_pattern(input: &str) -> bool {
    input.contains('*') || input.contains('?') || input.contains('[') || input.contains('{')
}

fn collect_glob_matches(pattern: &str) -> Result<Vec<PathBuf>, anyhow::Error> {
    let mut matches = Vec::new();
    for entry in glob::glob(pattern)
        .map_err(|e| anyhow::anyhow!("invalid glob pattern '{}': {e}", pattern))?
    {
        let path = entry.map_err(|e| anyhow::anyhow!("glob read error: {e}"))?;
        if path.is_file() {
            matches.push(path);
        }
    }
    Ok(matches)
}

fn walk_directory_for_sources(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

fn is_supported_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ts")
            | Some("js")
            | Some("mts")
            | Some("mjs")
            | Some("tsx")
            | Some("jsx")
            | Some("cjs")
            | Some("cts")
    )
}

fn compile_patterns(patterns: &[String]) -> Result<Vec<Pattern>, anyhow::Error> {
    patterns
        .iter()
        .map(|p| {
            Pattern::new(p).map_err(|e| anyhow::anyhow!("invalid ignore pattern '{}': {e}", p))
        })
        .collect()
}

fn matches_ignore(path: &Path, cwd: &Path, ignore_patterns: &[Pattern]) -> bool {
    if ignore_patterns.is_empty() {
        return false;
    }

    let relative = path.strip_prefix(cwd).unwrap_or(path);
    let relative_str = relative.to_string_lossy();
    let relative_with_dot = format!("./{}", relative_str);

    ignore_patterns.iter().any(|pattern| {
        pattern.matches_path(path)
            || pattern.matches_path(relative)
            || pattern.matches(&relative_str)
            || pattern.matches(&relative_with_dot)
    })
}
