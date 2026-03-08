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

fn run_module_and_expect_true(specifier: &str, source: &str, check_expr: &'static str) -> Result<(), String> {
    let eszip_bytes = build_eszip(specifier, source);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
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
            .execute_script("<check>", check_expr)
            .map_err(|e| format!("check script failed: {e}"))?;

        let passed = {
            deno_core::scope!(scope, js_runtime);
            let local_val = val.open(scope);
            local_val.is_true()
        };

        if passed {
            Ok(())
        } else {
            Err("check expression evaluated to false".to_string())
        }
    })
}

#[test]
fn node_process_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import process, { version, versions } from "node:process";
      globalThis.__nodeProcessCompatOk =
        typeof process === "object" &&
        typeof version === "string" &&
        typeof versions?.node === "string" &&
        typeof process.nextTick === "function";
    "#;

    let eszip_bytes = build_eszip("file:///node_process_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeProcessCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:process import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_buffer_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import bufferMod, { Buffer } from "node:buffer";
      const a = Buffer.from("hello", "utf8");
      const b = bufferMod.Buffer.from("68656c6c6f", "hex");
      globalThis.__nodeBufferCompatOk =
        Buffer.isBuffer(a) &&
        a.toString("utf8") === "hello" &&
        b.toString("utf8") === "hello";
    "#;

    let eszip_bytes = build_eszip("file:///node_buffer_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeBufferCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:buffer import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_events_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import events, { EventEmitter } from "node:events";
      const emitter = new EventEmitter();
      let called = 0;

      emitter.on("a", () => { called += 1; });
      emitter.once("a", () => { called += 10; });

      emitter.emit("a");
      emitter.emit("a");

      globalThis.__nodeEventsCompatOk =
        typeof events.EventEmitter === "function" &&
        called === 12 &&
        emitter.listenerCount("a") === 1;
    "#;

    let eszip_bytes = build_eszip("file:///node_events_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeEventsCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:events import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_util_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
    import util, { format, promisify, types, MIMEType } from "node:util";

      const formatted = format("hello %s %d", "edge", 42);

      const cbApi = (value, cb) => cb(null, value + 1);
      const promised = promisify(cbApi);

      const typeChecks =
        types.isDate(new Date()) &&
        types.isRegExp(/x/) &&
        types.isUint8Array(new Uint8Array(1)) &&
        typeof util.inspect({ a: 1 }) === "string";

            const mime = new MIMEType("text/html; charset=utf-8");
            mime.params.set("q", "0.9");
            const mimeChecks =
                mime.type === "text" &&
                mime.subtype === "html" &&
                mime.essence === "text/html" &&
                mime.params.get("charset") === "utf-8" &&
                mime.toString().includes("q=0.9");

      globalThis.__nodeUtilCompatOk = false;
      promised(1).then((value) => {
                globalThis.__nodeUtilCompatOk = formatted === "hello edge 42" && value === 2 && typeChecks && mimeChecks;
      });
    "#;

    let eszip_bytes = build_eszip("file:///node_util_test.ts", source);

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

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop (promises): {e}"))?;

        let val = js_runtime
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeUtilCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:util import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_path_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import path, { join, dirname, basename, extname, relative } from "node:path";

      const p1 = join("/a", "b", "..", "c.txt");
      const p2 = dirname("/a/c.txt");
      const p3 = basename("/a/c.txt");
      const p4 = extname("/a/c.txt");
      const p5 = relative("/a/b", "/a/c/d");

      globalThis.__nodePathCompatOk =
        p1 === "/a/c.txt" &&
        p2 === "/a" &&
        p3 === "c.txt" &&
        p4 === ".txt" &&
        p5 === "../c/d" &&
        path.sep === "/" &&
        typeof path.posix.join === "function" &&
        typeof path.win32.join === "function";
    "#;

    let eszip_bytes = build_eszip("file:///node_path_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodePathCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:path import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_stream_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import stream, { Readable, Writable, Transform, pipeline } from "node:stream";

      const source = Readable.from(["a", "b"]);
      let output = "";

      const upper = new Transform({
        transform(chunk, _encoding, cb) {
          cb(undefined, String(chunk).toUpperCase());
        },
      });

      const sink = new Writable({
        write(chunk, _encoding, cb) {
          output += String(chunk);
          cb();
        },
      });

      globalThis.__nodeStreamCompatOk = false;
      pipeline(source, upper, sink, (err) => {
        globalThis.__nodeStreamCompatOk =
          !err &&
          output === "AB" &&
          typeof stream.Readable === "function" &&
          typeof stream.promises.pipeline === "function";
      });
    "#;

    let eszip_bytes = build_eszip("file:///node_stream_test.ts", source);

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

        js_runtime
            .run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
                pump_v8_message_loop: true,
            })
            .await
            .map_err(|e| format!("run_event_loop (pipeline): {e}"))?;

        let val = js_runtime
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeStreamCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:stream import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_stream_pipeline_handles_backpressure_on_long_flow() {
    deno_core::JsRuntime::init_platform(None);

        let source = r#"
            import { Writable } from "node:stream";

            let output = "";
            let drained = false;
            let finishErr = null;

            const sink = new Writable({
                highWaterMark: 4,
                write(chunk, _encoding, cb) {
                    setTimeout(() => {
                        output += String(chunk);
                        cb();
                    }, 5);
                },
            });

            sink.once("drain", () => {
                drained = true;
            });

            globalThis.__nodeStreamBackpressureOk = false;
            sink.once("finish", () => {
                globalThis.__nodeStreamBackpressureDebug = JSON.stringify({
                    drained,
                    output,
                    finishErr: finishErr ? String(finishErr) : null,
                });
                globalThis.__nodeStreamBackpressureOk =
                    finishErr === null && drained && output === "AAAABBBBCCCC";
            });
            sink.once("error", (err) => {
                finishErr = err;
            });

            const first = sink.write("AAAA");
            const second = sink.write("BBBB");
            setTimeout(() => sink.end("CCCC"), 20);
            globalThis.__nodeStreamBackpressureDebug = JSON.stringify({ first, second });

            if (!(first === true && second === false)) {
                globalThis.__nodeStreamBackpressureOk = false;
            }
        "#;

    let eszip_bytes = build_eszip("file:///node_stream_backpressure_test.ts", source);

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

        // Allow delayed writable callbacks and finish event to settle.
        for _ in 0..8 {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            js_runtime
                .run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                    pump_v8_message_loop: true,
                })
                .await
                .map_err(|e| format!("run_event_loop (drain): {e}"))?;
        }

        let val = js_runtime
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeStreamBackpressureOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        let is_ok = {
            deno_core::scope!(scope, js_runtime);
            let local_val = val.open(scope);
            local_val.is_true()
        };

        if is_ok {
            Ok(())
        } else {
            let dbg = js_runtime
                .execute_script(
                    "<debug>",
                    deno_core::ascii_str!("String(globalThis.__nodeStreamBackpressureDebug || 'no-debug')"),
                )
                .map_err(|e| format!("debug script failed: {e}"))?;
            let dbg_text = {
                deno_core::scope!(scope2, js_runtime);
                let dbg_local = dbg.open(scope2);
                dbg_local.to_rust_string_lossy(scope2)
            };
            Err(format!("node:stream long-flow backpressure check failed: {dbg_text}"))
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_stream_readable_web_bridge_roundtrip_works() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import { Readable } from "node:stream";

      const encoder = new TextEncoder();
      const webReadable = new ReadableStream({
        start(controller) {
          controller.enqueue(encoder.encode("hello-"));
          controller.enqueue(encoder.encode("bridge"));
          controller.close();
        },
      });

      const nodeReadable = Readable.fromWeb(webReadable);
      const webRoundtrip = Readable.toWeb(nodeReadable);
      const reader = webRoundtrip.getReader();

      let out = "";
      globalThis.__nodeStreamWebReadableBridgeOk = false;

      (async () => {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          out += typeof value === "string" ? value : new TextDecoder().decode(value);
        }
        globalThis.__nodeStreamWebReadableBridgeOk = out === "hello-bridge";
      })().catch(() => {
        globalThis.__nodeStreamWebReadableBridgeOk = false;
      });
    "#;

    let eszip_bytes = build_eszip("file:///node_stream_web_readable_bridge_test.ts", source);

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

        for _ in 0..4 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            js_runtime
                .run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                    pump_v8_message_loop: true,
                })
                .await
                .map_err(|e| format!("run_event_loop (bridge): {e}"))?;
        }

        let val = js_runtime
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeStreamWebReadableBridgeOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:stream readable web bridge check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_stream_writable_web_bridge_roundtrip_works() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import { Writable } from "node:stream";

      let webSinkOut = "";
      const webWritable = new WritableStream({
        write(chunk) {
          webSinkOut += typeof chunk === "string" ? chunk : new TextDecoder().decode(chunk);
        },
      });

      const nodeWritable = Writable.fromWeb(webWritable);
      nodeWritable.write("A");
      nodeWritable.write("B");
      nodeWritable.end("C");

      let nodeSinkOut = "";
      const nodeSink = new Writable({
        write(chunk, _encoding, cb) {
          nodeSinkOut += String(chunk);
          cb();
        },
      });
      const webFromNode = Writable.toWeb(nodeSink);
      const writer = webFromNode.getWriter();

      globalThis.__nodeStreamWebWritableBridgeOk = false;
      (async () => {
        await writer.write("X");
        await writer.write("Y");
        await writer.close();

                // Allow node->web close propagation to flush final chunk from end("C").
                for (let i = 0; i < 20 && webSinkOut !== "ABC"; i++) {
                    await new Promise((resolve) => setTimeout(resolve, 5));
                }

        globalThis.__nodeStreamWebWritableBridgeOk =
          webSinkOut === "ABC" && nodeSinkOut === "XY";
      })().catch(() => {
        globalThis.__nodeStreamWebWritableBridgeOk = false;
      });
    "#;

    let eszip_bytes = build_eszip("file:///node_stream_web_writable_bridge_test.ts", source);

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

        for _ in 0..4 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            js_runtime
                .run_event_loop(PollEventLoopOptions {
                    wait_for_inspector: false,
                    pump_v8_message_loop: true,
                })
                .await
                .map_err(|e| format!("run_event_loop (bridge): {e}"))?;
        }

        let val = js_runtime
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeStreamWebWritableBridgeOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        let is_ok = {
            deno_core::scope!(scope, js_runtime);
            let local_val = val.open(scope);
            local_val.is_true()
        };

        if is_ok {
            Ok(())
        } else {
            Err("node:stream writable web bridge check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_os_module_can_be_imported() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import os, { platform, arch, tmpdir, EOL } from "node:os";

      const p = platform();
      const a = arch();
      const t = tmpdir();
      const eol = EOL;

      const info = os.userInfo();
      const cpus = os.cpus();
      const ni = os.networkInterfaces();

      let setPriorityThrows = false;
      try {
        os.setPriority(0, 0);
      } catch (err) {
                setPriorityThrows = String(err?.message || "").includes("[thunder] os.setPriority is not implemented in this runtime profile");
      }

      globalThis.__nodeOsCompatOk =
        p === "linux" &&
        a === "x64" &&
        t === "/tmp" &&
        eol === "\n" &&
        info?.username === "edge" &&
        Array.isArray(cpus) &&
        typeof ni === "object" &&
        setPriorityThrows;
    "#;

    let eszip_bytes = build_eszip("file:///node_os_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeOsCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:os import check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_module_create_require_supports_builtins_only() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import { createRequire } from "node:module";
      import * as utilNs from "node:util";

      const require = createRequire(import.meta.url);
      const utilCjs = require("node:util");
      const pathCjs = require("path");
            const requestCjs = require("request");

            const previousFetchHook = globalThis.__edgeMockFetchHandler;
            globalThis.__edgeMockFetchHandler = async () => new Response("request-mock-body", { status: 201 });

            globalThis.__requestCompatOk = false;
            await new Promise((resolve) => {
                requestCjs.get("https://example.com", (err, res, body) => {
                    globalThis.__requestCompatOk =
                        !err &&
                        res?.statusCode === 201 &&
                        body === "request-mock-body";
                    resolve(undefined);
                });
            });

            globalThis.__edgeMockFetchHandler = async (_input, init) => {
                const body = String(init?.body ?? "");
                return new Response(body === "manual-body" ? "request-manual-ok" : "request-manual-bad", {
                    status: body === "manual-body" ? 202 : 500,
                });
            };

            globalThis.__requestWriteEndOk = false;
            await new Promise((resolve) => {
                const req = requestCjs.default("https://example.com", { method: "POST" }, (err, res, responseBody) => {
                    globalThis.__requestWriteEndOk =
                        !err &&
                        res?.statusCode === 202 &&
                        responseBody === "request-manual-ok";
                    resolve(undefined);
                });
                req.write("manual-body");
                req.end();
            });

            globalThis.__edgeMockFetchHandler = previousFetchHook;

            let unsupportedOk = false;
            try {
                                require("definitely_not_real_builtin");
      } catch (err) {
        unsupportedOk = String(err?.message || "").includes("Only built-in modules are supported");
      }

      globalThis.__nodeModuleCompatOk =
        typeof utilCjs.format === "function" &&
        utilCjs.default === utilNs.default &&
        pathCjs.join("/a", "b") === "/a/b" &&
                typeof requestCjs.get === "function" &&
                typeof requestCjs.delete === "function" &&
                typeof requestCjs.default === "function" &&
                globalThis.__requestCompatOk === true &&
                globalThis.__requestWriteEndOk === true &&
        require.resolve("node:path") === "node:path" &&
        unsupportedOk;
    "#;

    let eszip_bytes = build_eszip("file:///node_module_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeModuleCompatOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("node:module createRequire check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn additional_node_stub_modules_import_and_behave_predictably() {
    deno_core::JsRuntime::init_platform(None);

    let source = r#"
      import asyncHooks from "node:async_hooks";
    import assertMod from "node:assert";
      import childProcess from "node:child_process";
      import cluster from "node:cluster";
      import consoleMod from "node:console";
      import diagnostics from "node:diagnostics_channel";
      import dns from "node:dns";
      import dgram from "node:dgram";
      import http from "node:http";
      import https from "node:https";
      import http2 from "node:http2";
      import inspector from "node:inspector";
      import net from "node:net";
      import perfHooks from "node:perf_hooks";
      import punycode from "node:punycode";
      import querystring from "node:querystring";
    import urlMod from "node:url";
      import readline from "node:readline";
      import repl from "node:repl";
            import sqlite from "node:sqlite";
      import stringDecoder from "node:string_decoder";
            import nodeTest from "node:test";
    import timers from "node:timers";
    import timersPromises from "node:timers/promises";
      import tls from "node:tls";
      import v8 from "node:v8";
      import vm from "node:vm";
      import zlib from "node:zlib";

            function isThunderNotImplemented(err, api) {
                const message = String(err?.message || "");
                return message.includes(`[thunder] ${api} is not implemented in this runtime profile`);
            }

      let deterministicErrors = 0;
            try { childProcess.exec('echo hi'); } catch (err) { if (isThunderNotImplemented(err, 'child_process.exec')) deterministicErrors++; }
            try { cluster.fork(); } catch (err) { if (isThunderNotImplemented(err, 'cluster.fork')) deterministicErrors++; }
            try { dns.lookupService('1.1.1.1', 53, () => {}); } catch (err) { if (isThunderNotImplemented(err, 'dns.lookupService')) deterministicErrors++; }
            try { dgram.createSocket('udp4'); } catch (err) { if (isThunderNotImplemented(err, 'dgram.createSocket')) deterministicErrors++; }
            try { net.createServer().listen(80); } catch (err) { if (isThunderNotImplemented(err, 'net.Server.listen')) deterministicErrors++; }
            try { tls.createServer(); } catch (err) { if (isThunderNotImplemented(err, 'tls.createServer')) deterministicErrors++; }
            try { repl.start(); } catch (err) { if (isThunderNotImplemented(err, 'repl.start')) deterministicErrors++; }
            try { new vm.Script('1+1').runInThisContext(); } catch (err) { if (isThunderNotImplemented(err, 'vm.Script.runInThisContext')) deterministicErrors++; }
            try { new sqlite.Database(':memory:'); } catch (err) { if (isThunderNotImplemented(err, 'sqlite.Database')) deterministicErrors++; }
            try { nodeTest.test('stub', () => {}); } catch (err) { if (isThunderNotImplemented(err, 'node:test')) deterministicErrors++; }

            const zlibSyncCompat = (() => {
                try {
                    const gz = zlib.gzipSync('hello-zlib-sync');
                    const plain = zlib.gunzipSync(gz);
                    const text = typeof plain === 'string' ? plain : new TextDecoder().decode(plain);
                    return text === 'hello-zlib-sync';
                } catch (_) {
                    return false;
                }
            })();

            const zlibCompat = await new Promise((resolve) => {
                zlib.gzip('hello-zlib', (gzipErr, gz) => {
                    if (gzipErr || !gz) {
                        resolve(false);
                        return;
                    }
                    zlib.gunzip(gz, (gunzipErr, plain) => {
                        if (gunzipErr || !plain) {
                            resolve(false);
                            return;
                        }
                        const text = typeof plain === 'string' ? plain : new TextDecoder().decode(plain);
                        resolve(text === 'hello-zlib');
                    });
                });
            });

            let zlibLimitEnforced = false;
            try {
                const gz = await zlib.gzip('limit-check-payload');
                await zlib.gunzip(gz, { maxOutputLength: 1 });
            } catch (err) {
                zlibLimitEnforced = String(err?.code || '').includes('ERR_BUFFER_TOO_LARGE');
            }

            let zlibInputLimitEnforced = false;
            try {
                const big = new Uint8Array(8 * 1024 * 1024 + 1);
                await zlib.gzip(big);
            } catch (err) {
                zlibInputLimitEnforced = String(err?.code || '').includes('ERR_ZLIB_INPUT_TOO_LARGE');
            }

            let zlibInvalidTimeoutRejected = false;
            try {
                await zlib.gzip('timeout-check', { operationTimeoutMs: 0 });
            } catch (err) {
                zlibInvalidTimeoutRejected = String(err?.code || '').includes('ERR_INVALID_ARG_VALUE');
            }

            const als = new asyncHooks.AsyncLocalStorage();
            const hook = asyncHooks.createHook({
                init: () => {},
            }).enable();

            let alsPromiseValue;
            await new Promise((resolve) => {
                als.run('ctx:promise', () => {
                    Promise.resolve().then(() => {
                        alsPromiseValue = als.getStore();
                        resolve(undefined);
                    });
                });
            });

            let alsMicrotaskValue;
            await new Promise((resolve) => {
                als.run('ctx:microtask', () => {
                    queueMicrotask(() => {
                        alsMicrotaskValue = als.getStore();
                        resolve(undefined);
                    });
                });
            });
            hook.disable();

            const previousFetchHook = globalThis.__edgeMockFetchHandler;
            globalThis.__edgeMockFetchHandler = async (input) => {
                const raw = typeof input === 'string' ? input : input?.url;
                const url = new URL(String(raw || 'https://example.com/'));

                if (url.pathname.includes('dns-query')) {
                    const type = (url.searchParams.get('type') || 'A').toUpperCase();
                    const name = (url.searchParams.get('name') || '').toLowerCase();

                    if (type === 'A' && name === 'example.com') {
                        return new Response(JSON.stringify({
                            Status: 0,
                            Answer: [{ type: 1, data: '93.184.216.34' }],
                        }), { status: 200, headers: { 'content-type': 'application/dns-json' } });
                    }

                    if (type === 'AAAA' && name === 'example.com') {
                        return new Response(JSON.stringify({ Status: 0, Answer: [] }), {
                            status: 200,
                            headers: { 'content-type': 'application/dns-json' },
                        });
                    }

                    if (type === 'PTR' && name === '34.216.184.93.in-addr.arpa') {
                        return new Response(JSON.stringify({
                            Status: 0,
                            Answer: [{ type: 12, data: 'example.com' }],
                        }), { status: 200, headers: { 'content-type': 'application/dns-json' } });
                    }

                    return new Response(JSON.stringify({ Status: 3, Answer: [] }), {
                        status: 200,
                        headers: { 'content-type': 'application/dns-json' },
                    });
                }

                return new Response('ok-from-mock', { status: 200 });
            };

            const dnsLookupCompat = await new Promise((resolve) => {
                dns.lookup('example.com', (err, address, family) => {
                    resolve(!err && address === '93.184.216.34' && family === 4);
                });
            });

            const dnsResolveCompat = await new Promise((resolve) => {
                dns.resolve4('example.com', (err, records) => {
                    resolve(!err && Array.isArray(records) && records[0] === '93.184.216.34');
                });
            });

            const dnsReverseCompat = await new Promise((resolve) => {
                dns.reverse('93.184.216.34', (err, hostnames) => {
                    resolve(!err && Array.isArray(hostnames) && hostnames[0] === 'example.com');
                });
            });

            const httpFetchCompat = await new Promise((resolve) => {
                http.get('https://example.com', (res) => {
                    let body = '';
                    res.on('data', (chunk) => { body += String(chunk); });
                    res.on('end', () => {
                        resolve(res.statusCode === 200 && body.includes('ok-from-mock'));
                    });
                    res.on('error', () => resolve(false));
                }).on('error', () => resolve(false));
            });

            const httpsFetchCompat = await new Promise((resolve) => {
                https.request('https://example.com', { method: 'GET' }, (res) => {
                    let body = '';
                    res.on('data', (chunk) => { body += String(chunk); });
                    res.on('end', () => {
                        resolve(res.statusCode === 200 && body.includes('ok-from-mock'));
                    });
                    res.on('error', () => resolve(false));
                }).on('error', () => resolve(false)).end();
            });

            globalThis.__edgeMockFetchHandler = previousFetchHook;

            let assertWorks = false;
            try { assertMod.strictEqual(1, 2); } catch (_) { assertWorks = true; }

            const fileUrl = urlMod.pathToFileURL('/tmp/a.txt');
            const asciiDomain = urlMod.domainToASCII('español.com');
            const unicodeDomain = urlMod.domainToUnicode('xn--espaol-zwa.com');

        const channel = diagnostics.channel('edge');
            const tracing = new diagnostics.TracingChannel('edge.trace');
            let traceStart = 0;
            let traceEnd = 0;
            tracing.subscribe({
                start: () => { traceStart++; },
                end: () => { traceEnd++; },
            });
            const tracedValue = tracing.traceSync((a, b) => a + b, null, 2, 3);
      const parsed = querystring.parse('a=1&b=2');
      const decoded = new stringDecoder.StringDecoder('utf-8').end(new Uint8Array([65]));

            globalThis.__nodeMoreStubsOk =
                typeof asyncHooks.createHook === 'function' &&
                typeof asyncHooks.executionAsyncId === 'function' &&
                typeof asyncHooks.triggerAsyncId === 'function' &&
                alsPromiseValue === 'ctx:promise' &&
                alsMicrotaskValue === 'ctx:microtask' &&
                typeof net.connect === 'function' &&
                typeof tls.connect === 'function' &&
                typeof http.request === 'function' &&
                typeof https.request === 'function' &&
                typeof dns.promises?.lookup === 'function' &&
                dnsLookupCompat &&
                dnsResolveCompat &&
                dnsReverseCompat &&
                zlibCompat &&
                zlibSyncCompat &&
                zlibLimitEnforced &&
                zlibInputLimitEnforced &&
                zlibInvalidTimeoutRejected &&
                httpFetchCompat &&
                httpsFetchCompat &&
                typeof querystring.stringify === 'function' &&
                typeof timers.setTimeout === 'function' &&
                typeof timersPromises.setTimeout === 'function' &&
                typeof diagnostics.TracingChannel === 'function' &&
                typeof diagnostics.tracingChannel === 'function' &&
                tracedValue === 5 &&
                traceStart === 1 &&
                traceEnd === 1 &&
                fileUrl.protocol === 'file:' &&
                typeof asciiDomain === 'string' &&
                typeof unicodeDomain === 'string' &&
                assertWorks &&
                typeof punycode.toASCII === 'function' &&
                deterministicErrors >= 10;
    "#;

    let eszip_bytes = build_eszip("file:///node_extra_stubs_test.ts", source);

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
            .execute_script(
                "<check>",
                deno_core::ascii_str!("globalThis.__nodeMoreStubsOk === true"),
            )
            .map_err(|e| format!("check script failed: {e}"))?;

        let passed = {
            deno_core::scope!(scope, js_runtime);
            let local_val = val.open(scope);
            local_val.is_true()
        };

        if passed {
            Ok(())
        } else {
            Err("additional node stub modules check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_zlib_timeout_guardrail_triggers_under_load() {
        deno_core::JsRuntime::init_platform(None);

        let source = r#"
            const zlib = await import('node:zlib');

            const payload = new Uint8Array(8 * 1024 * 1024);
            payload.fill(65);

            const compressed = zlib.gzipSync(payload, {
                maxOutputLength: 64 * 1024 * 1024,
                operationTimeoutMs: 2000,
            });

            const results = await Promise.all(
                Array.from({ length: 6 }, async () => {
                    try {
                        await zlib.gunzip(compressed, {
                            maxOutputLength: 64 * 1024 * 1024,
                            operationTimeoutMs: 1,
                        });
                        return false;
                    } catch (err) {
                        return String(err?.code || '').includes('ERR_ZLIB_OPERATION_TIMEOUT');
                    }
                })
            );

            globalThis.__nodeZlibTimeoutUnderLoadOk = results.some(Boolean);
        "#;

        let result = run_module_and_expect_true(
                "file:///node_zlib_timeout_under_load.ts",
                source,
                "globalThis.__nodeZlibTimeoutUnderLoadOk === true",
        );

        assert!(result.is_ok(), "{result:?}");
}

#[test]
fn node_zlib_runtime_config_defaults_are_respected() {
        deno_core::JsRuntime::init_platform(None);

        let source = r#"
            globalThis.__edgeRuntimeZlibConfig = {
                maxOutputLength: 64 * 1024 * 1024,
                maxInputLength: 1024,
                operationTimeoutMs: 200,
            };

            const zlib = await import('node:zlib');

            const tooBigInput = new Uint8Array(2048);
            tooBigInput.fill(7);

            let runtimeDefaultsApplied = false;
            try {
                await zlib.gzip(tooBigInput);
            } catch (err) {
                runtimeDefaultsApplied = String(err?.code || '').includes('ERR_ZLIB_INPUT_TOO_LARGE');
            }

            globalThis.__nodeZlibRuntimeDefaultsOk = runtimeDefaultsApplied;
        "#;

        let result = run_module_and_expect_true(
                "file:///node_zlib_runtime_defaults.ts",
                source,
                "globalThis.__nodeZlibRuntimeDefaultsOk === true",
        );

        assert!(result.is_ok(), "{result:?}");
}

#[test]
fn web_request_clone_preserves_body_and_locks_original_after_read() {
        deno_core::JsRuntime::init_platform(None);

        let source = r#"
                const req = new Request('https://example.com/test', {
                    method: 'POST',
                    body: 'clone-me',
                    headers: { 'content-type': 'text/plain' },
                });

                const cloned = req.clone();
                const [originalText, clonedText] = await Promise.all([
                    req.text(),
                    cloned.text(),
                ]);

                let secondReadThrows = false;
                try {
                    await req.text();
                } catch (_err) {
                    secondReadThrows = true;
                }

                globalThis.__webRequestCloneOk =
                    originalText === 'clone-me' &&
                    clonedText === 'clone-me' &&
                    req.bodyUsed === true &&
                    secondReadThrows;
        "#;

        let result = run_module_and_expect_true(
                "file:///web_request_clone_semantics.ts",
                source,
                "globalThis.__webRequestCloneOk === true",
        );

        assert!(result.is_ok(), "{result:?}");
}

#[test]
fn web_response_clone_preserves_body_and_locks_original_after_read() {
        deno_core::JsRuntime::init_platform(None);

        let source = r#"
                const resp = new Response('response-clone', {
                    headers: { 'content-type': 'text/plain' },
                });

                const cloned = resp.clone();
                const [originalText, clonedText] = await Promise.all([
                    resp.text(),
                    cloned.text(),
                ]);

                let secondReadThrows = false;
                try {
                    await resp.text();
                } catch (_err) {
                    secondReadThrows = true;
                }

                globalThis.__webResponseCloneOk =
                    originalText === 'response-clone' &&
                    clonedText === 'response-clone' &&
                    resp.bodyUsed === true &&
                    secondReadThrows;
        "#;

        let result = run_module_and_expect_true(
                "file:///web_response_clone_semantics.ts",
                source,
                "globalThis.__webResponseCloneOk === true",
        );

        assert!(result.is_ok(), "{result:?}");
}

#[test]
fn web_stream_tee_splits_stream_without_data_loss() {
        deno_core::JsRuntime::init_platform(None);

        let source = r#"
                const encoder = new TextEncoder();
                const stream = new ReadableStream({
                    start(controller) {
                        controller.enqueue(encoder.encode('A'));
                        controller.enqueue(encoder.encode('B'));
                        controller.enqueue(encoder.encode('C'));
                        controller.close();
                    },
                });

                const [left, right] = stream.tee();

                const readAll = async (readable) => {
                    const reader = readable.getReader();
                    let out = '';
                    while (true) {
                        const { done, value } = await reader.read();
                        if (done) break;
                        out += new TextDecoder().decode(value);
                    }
                    return out;
                };

                const [leftOut, rightOut] = await Promise.all([
                    readAll(left),
                    readAll(right),
                ]);

                globalThis.__webTeeOk = leftOut === 'ABC' && rightOut === 'ABC';
        "#;

        let result = run_module_and_expect_true(
                "file:///web_stream_tee_semantics.ts",
                source,
                "globalThis.__webTeeOk === true",
        );

        assert!(result.is_ok(), "{result:?}");
}

