use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum ResidualKind {
    JavaScript,
    Esm,
}

fn collect_residual_sources(
    kind: ResidualKind,
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut sources = BTreeMap::new();

    for extension in runtime_core::extensions::get_extensions() {
        let files = match kind {
            ResidualKind::JavaScript => extension.lazy_loaded_js_files.as_ref(),
            ResidualKind::Esm => {
                let mut files = extension.esm_files.to_vec();
                files.extend_from_slice(extension.lazy_loaded_esm_files.as_ref());
                let owned_files = files;
                for file in owned_files {
                    let (source, _) = runtime_core::extensions::transpile_extension_source(
                        deno_core::ModuleName::from_static(file.specifier),
                        file.load()?,
                    )?;
                    let source_text: &str = &source;
                    if sources
                        .insert(file.specifier.to_owned(), source_text.to_owned())
                        .is_some()
                    {
                        return Err(format!(
                            "duplicate residual extension source: {}",
                            file.specifier
                        )
                        .into());
                    }
                }
                continue;
            }
        };

        for file in files {
            let (source, _) = runtime_core::extensions::transpile_extension_source(
                deno_core::ModuleName::from_static(file.specifier),
                file.load()?,
            )?;
            let source_text: &str = &source;
            let source = match kind {
                ResidualKind::JavaScript => deno_core::wrap_lazy_ext_script(source_text),
                ResidualKind::Esm => source_text.to_owned(),
            };

            if sources.insert(file.specifier.to_owned(), source).is_some() {
                return Err(
                    format!("duplicate residual extension source: {}", file.specifier).into(),
                );
            }
        }
    }

    Ok(sources)
}

fn write_residual_sources(
    path: &PathBuf,
    lazy_js_sources: BTreeMap<String, String>,
    lazy_esm_sources: BTreeMap<String, String>,
    consumed_sources: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let consumed_sources = consumed_sources
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut output = fs::File::create(path)?;

    writeln!(
        output,
        "pub static RUNTIME_BASE_RESIDUAL_LAZY_JS_SOURCES: &[(&str, &str)] = &["
    )?;
    for (specifier, source) in lazy_js_sources {
        if !consumed_sources.contains(&specifier) {
            writeln!(output, "    ({specifier:?}, {source:?}),")?;
        }
    }
    writeln!(output, "];")?;

    writeln!(
        output,
        "pub static RUNTIME_BASE_RESIDUAL_LAZY_ESM_SOURCES: &[(&str, &str)] = &["
    )?;
    for (specifier, source) in lazy_esm_sources {
        if !consumed_sources.contains(&specifier) {
            writeln!(output, "    ({specifier:?}, {source:?}),")?;
        }
    }
    writeln!(output, "];")?;

    Ok(())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../runtime-core/src/bootstrap.js");
    println!("cargo:rerun-if-changed=../runtime-core/src/extensions.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let snapshot_path = out_dir.join("runtime_base.snapshot.bin");
    let residual_sources_path = out_dir.join("runtime_base_residual_sources.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let manifest_dir_static: &'static str = Box::leak(manifest_dir.into_boxed_str());

    let lazy_js_sources =
        collect_residual_sources(ResidualKind::JavaScript).expect("collect lazy JS sources");
    let lazy_esm_sources =
        collect_residual_sources(ResidualKind::Esm).expect("collect lazy ESM sources");

    deno_core::JsRuntime::init_platform(None);
    let snapshot_output =
        runtime_core::extensions::create_runtime_base_snapshot(manifest_dir_static)
            .expect("failed to create runtime base snapshot");

    fs::write(&snapshot_path, &snapshot_output.output)
        .expect("failed to write runtime base snapshot");
    write_residual_sources(
        &residual_sources_path,
        lazy_js_sources,
        lazy_esm_sources,
        &snapshot_output.consumed_lazy_specifiers,
    )
    .expect("write residual extension sources");

    for file in snapshot_output.files_loaded_during_snapshot {
        println!("cargo:rerun-if-changed={}", file.display());
    }
}
