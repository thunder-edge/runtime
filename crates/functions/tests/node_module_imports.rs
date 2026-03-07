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
        setPriorityThrows = String(err?.message || "").includes("os.setPriority is not implemented");
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
      import stringDecoder from "node:string_decoder";
    import timers from "node:timers";
    import timersPromises from "node:timers/promises";
      import tls from "node:tls";
      import v8 from "node:v8";
      import vm from "node:vm";
      import zlib from "node:zlib";

      let deterministicErrors = 0;
      try { childProcess.exec('echo hi'); } catch (_) { deterministicErrors++; }
      try { cluster.fork(); } catch (_) { deterministicErrors++; }
      try { dns.lookupService('1.1.1.1', 53, () => {}); } catch (_) { deterministicErrors++; }
      try { dgram.createSocket('udp4'); } catch (_) { deterministicErrors++; }
    try { net.createServer().listen(80); } catch (_) { deterministicErrors++; }
    try { tls.createServer(); } catch (_) { deterministicErrors++; }
      try { repl.start(); } catch (_) { deterministicErrors++; }
      try { new vm.Script('1+1').runInThisContext(); } catch (_) { deterministicErrors++; }
      try { zlib.gzipSync('x'); } catch (_) { deterministicErrors++; }

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
                typeof net.connect === 'function' &&
                typeof tls.connect === 'function' &&
                typeof http.request === 'function' &&
                typeof https.request === 'function' &&
                typeof dns.promises?.lookup === 'function' &&
                dnsLookupCompat &&
                dnsResolveCompat &&
                dnsReverseCompat &&
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
                deterministicErrors >= 5;
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

        deno_core::scope!(scope, js_runtime);
        let local_val = val.open(scope);
        if local_val.is_true() {
            Ok(())
        } else {
            Err("additional node stub modules check failed".to_string())
        }
    });

    assert!(result.is_ok(), "{result:?}");
}

