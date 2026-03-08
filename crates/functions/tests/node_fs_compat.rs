use std::rc::Rc;
use std::sync::Arc;

use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::module_loader::EszipModuleLoader;

fn make_runtime_with_eszip(eszip: Arc<eszip::EszipV2>) -> JsRuntime {
    let module_loader = Rc::new(EszipModuleLoader::new(eszip));

    let mut opts = RuntimeOptions {
        module_loader: Some(module_loader),
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);

    JsRuntime::new(opts)
}

fn build_eszip(specifier: &str, source: &str) -> Vec<u8> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(build_eszip_async(specifier, source))
}

async fn build_eszip_async(specifier: &str, source: &str) -> Vec<u8> {
    use deno_ast::{EmitOptions, TranspileOptions};
    use deno_graph::ast::CapturingModuleAnalyzer;
    use deno_graph::source::{LoadOptions, LoadResponse, Loader};
    use deno_graph::{BuildOptions, GraphKind, ModuleGraph};

    struct InlineLoader {
        specifier: String,
        source: String,
    }

    impl Loader for InlineLoader {
        fn load(
            &self,
            specifier: &deno_graph::ModuleSpecifier,
            _options: LoadOptions,
        ) -> deno_graph::source::LoadFuture {
            let spec = specifier.clone();
            let expected = self.specifier.clone();
            let source = self.source.clone();
            Box::pin(async move {
                if spec.as_str() == expected {
                    Ok(Some(LoadResponse::Module {
                        content: source.into_bytes().into(),
                        specifier: spec,
                        maybe_headers: None,
                        mtime: None,
                    }))
                } else {
                    Ok(None)
                }
            })
        }
    }

    let loader = InlineLoader {
        specifier: specifier.to_string(),
        source: source.to_string(),
    };
    let analyzer = CapturingModuleAnalyzer::default();
    let root = deno_graph::ModuleSpecifier::parse(specifier).unwrap();

    let mut graph = ModuleGraph::new(GraphKind::CodeOnly);
    graph
        .build(
            vec![root],
            vec![],
            &loader,
            BuildOptions {
                module_analyzer: &analyzer,
                ..Default::default()
            },
        )
        .await;

    let eszip = eszip::EszipV2::from_graph(eszip::FromGraphOptions {
        graph,
        parser: analyzer.as_capturing_parser(),
        module_kind_resolver: Default::default(),
        transpile_options: TranspileOptions::default(),
        emit_options: EmitOptions::default(),
        relative_file_base: None,
        npm_packages: None,
        npm_snapshot: Default::default(),
    })
    .expect("from_graph failed");

    eszip.into_bytes()
}

async fn parse_eszip(bytes: &[u8]) -> eszip::EszipV2 {
    let reader = futures_util::io::BufReader::new(futures_util::io::Cursor::new(bytes.to_vec()));
    let (eszip, loader_fut) = eszip::EszipV2::parse(reader).await.unwrap();
    tokio::spawn(loader_fut);
    eszip
}

fn run_module_and_check(source: &str, check_expr: &'static str, what: &str) {
    deno_core::JsRuntime::init_platform(None);

    let eszip_bytes = build_eszip("file:///node_fs_compat_test.ts", source);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    let result: Result<(), String> = local.block_on(&rt, async {
        let eszip = Arc::new(parse_eszip(&eszip_bytes).await);
        let root = runtime_core::isolate::determine_root_specifier(&eszip)
            .map_err(|e| format!("determine_root_specifier: {e}"))?;

        let mut js_runtime = make_runtime_with_eszip(eszip);

        let module_id = js_runtime
            .load_main_es_module(&root)
            .await
            .map_err(|e| format!("load_main_es_module: {e}"))?;

        let eval = js_runtime.mod_evaluate(module_id);

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop: {e}"))?;

        eval.await.map_err(|e| format!("mod_evaluate: {e}"))?;

        let val = js_runtime
            .execute_script("<check>", deno_core::FastString::from_static(check_expr))
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err(format!("{what} failed"))
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_fs_stub_exposes_expected_surface() {
    let source = r#"
      import fs, { constants } from "node:fs";
      import { createRequire } from "node:module";

      const require = createRequire(import.meta.url);
      const fsByRequire = require("fs");

      globalThis.__nodeFsSurfaceOk =
        typeof fs.readFileSync === "function" &&
        typeof fs.writeFileSync === "function" &&
        typeof fs.readFile === "function" &&
        typeof fs.watch === "function" &&
        constants.F_OK === 0 && constants.R_OK === 4 && constants.W_OK === 2 && constants.X_OK === 1 &&
        typeof fsByRequire.readFileSync === "function";
    "#;

    run_module_and_check(
        source,
        "globalThis.__nodeFsSurfaceOk === true",
        "node:fs surface check",
    );
}

#[test]
fn node_fs_sync_supports_tmp_and_blocks_bundle_writes() {
    let source = r#"
      import fs from "node:fs";

            fs.mkdirSync("/tmp/cache", { recursive: true });
            fs.writeFileSync("/tmp/cache/test.txt", "hello-vfs");
            const roundTrip = fs.readFileSync("/tmp/cache/test.txt", "utf8") === "hello-vfs";

            let bundleReadOnly = false;
      try {
                fs.writeFileSync("/bundle/blocked.txt", "x");
      } catch (err) {
                bundleReadOnly =
                    err?.code === "EROFS" &&
                    err?.errno === 30 &&
                    err?.syscall === "writeFile" &&
                    err?.path === "/bundle/blocked.txt";
      }

            const entries = fs.readdirSync("/tmp/cache");
            globalThis.__nodeFsSyncErrOk = roundTrip && bundleReadOnly && entries.includes("test.txt");
    "#;

    run_module_and_check(
        source,
        "globalThis.__nodeFsSyncErrOk === true",
        "node:fs deterministic sync error check",
    );
}

#[test]
fn node_fs_callback_api_supports_vfs_io() {
    let source = r#"
      import fs from "node:fs";

            let callbackOk = false;
            fs.writeFile("/tmp/callback.txt", "cb-ok", null, (err) => {
                if (err) return;
                fs.readFile("/tmp/callback.txt", "utf8", (err2, value) => {
                    callbackOk = !err2 && value === "cb-ok";
                });
      });

      globalThis.__nodeFsCallbackOk = callbackOk;
    "#;

    run_module_and_check(
        source,
        "globalThis.__nodeFsCallbackOk === true",
        "node:fs callback error check",
    );
}

#[test]
fn node_fs_promises_honor_vfs_quota_limits() {
    let source = r#"
      import fsp from "node:fs/promises";

            const fourMiB = "a".repeat(4 * 1024 * 1024);
            await fsp.writeFile("/tmp/a.txt", fourMiB);
            await fsp.writeFile("/tmp/b.txt", fourMiB);

            let totalQuotaErr = false;
            await fsp.writeFile("/tmp/c.txt", "b".repeat(3 * 1024 * 1024)).catch((err) => {
                totalQuotaErr = err?.code === "ENOSPC" && err?.syscall === "writeFile";
      });

            let perFileErr = false;
            await fsp.writeFile("/tmp/too-large.txt", "c".repeat((5 * 1024 * 1024) + 1)).catch((err) => {
                perFileErr = err?.code === "ENOSPC" && err?.syscall === "writeFile";
            });

            globalThis.__nodeFsPromisesOk = totalQuotaErr && perFileErr;
    "#;

    run_module_and_check(
        source,
        "globalThis.__nodeFsPromisesOk === true",
        "node:fs/promises deterministic rejection check",
    );
}

#[test]
fn node_fs_callbacks_preserve_async_local_storage_context() {
        let source = r#"
            import fs from "node:fs";
            import { AsyncLocalStorage } from "node:async_hooks";

            const als = new AsyncLocalStorage();
            let callbackStore = null;

            als.run("request-context", () => {
                fs.writeFile("/tmp/als-callback.txt", "ok", null, (err) => {
                    if (!err) {
                        callbackStore = als.getStore();
                    }
                });

                // Mutate current context after callback registration; callback should keep captured one.
                als.enterWith("mutated-context");
            });

            globalThis.__nodeFsAlsCallbackOk = callbackStore === "request-context";
        "#;

        run_module_and_check(
                source,
                "globalThis.__nodeFsAlsCallbackOk === true",
                "node:fs callback should preserve ALS context",
        );
}

#[test]
fn node_fs_streams_support_vfs_roundtrip() {
        let source = r#"
            import fs from "node:fs";

            let readResult = "";

            const writeDone = new Promise((resolve, reject) => {
                const writer = fs.createWriteStream("/tmp/stream-roundtrip.txt");
                writer.once("error", reject);
                writer.once("close", resolve);
                writer.write("hello-");
                writer.end("stream");
            });

            await writeDone;

            const readDone = new Promise((resolve, reject) => {
                const reader = fs.createReadStream("/tmp/stream-roundtrip.txt", { encoding: "utf8", highWaterMark: 3 });
                reader.once("error", reject);
                reader.on("data", (chunk) => {
                    readResult += chunk;
                });
                reader.once("close", resolve);
            });

            await readDone;

            globalThis.__nodeFsStreamsRoundtripOk =
                readResult === "hello-stream";
        "#;

        run_module_and_check(
                source,
                "globalThis.__nodeFsStreamsRoundtripOk === true",
                "node:fs streams should roundtrip data in VFS",
        );
}

#[test]
fn node_fs_write_stream_rejects_non_writable_mount() {
        let source = r#"
            import fs from "node:fs";

            let sawExpectedError = false;
            await new Promise((resolve) => {
                const writer = fs.createWriteStream("/bundle/not-allowed.txt");
                writer.on("error", (err) => {
                    sawExpectedError =
                        err?.code === "EROFS" &&
                        err?.errno === 30 &&
                        err?.syscall === "createWriteStream";
                    resolve();
                });
            });

            globalThis.__nodeFsWriteStreamErrOk = sawExpectedError;
        "#;

        run_module_and_check(
                source,
                "globalThis.__nodeFsWriteStreamErrOk === true",
                "node:fs createWriteStream should fail on read-only mount",
        );
}
