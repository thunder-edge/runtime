use functions::registry::FunctionRegistry;
use functions::types::BundlePackage;
use runtime_core::isolate::{IsolateConfig, IsolateResponseBody};
use tokio_util::sync::CancellationToken;

static INIT: std::sync::Once = std::sync::Once::new();

fn init_runtime() {
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        deno_core::JsRuntime::init_platform(None);
    });
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
    let root = deno_graph::ModuleSpecifier::parse(specifier).expect("invalid root specifier");

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

async fn deploy_inline_function(name: &str, source: &str) -> Result<FunctionRegistry, String> {
    let eszip_bytes = build_eszip_async("file:///sandbox_test.ts", source).await;
    let bundle = BundlePackage::eszip_only(eszip_bytes);
    let bundle_data = bincode::serialize(&bundle).map_err(|e| format!("serialize bundle: {e}"))?;

    let registry = FunctionRegistry::new(CancellationToken::new(), IsolateConfig::default());
    registry
        .deploy(
            name.to_string(),
            bytes::Bytes::from(bundle_data),
            None,
            None,
        )
        .await
        .map_err(|e| format!("deploy failed: {e}"))?;

    Ok(registry)
}

async fn invoke_text(
    registry: &FunctionRegistry,
    name: &str,
    path: &str,
) -> Result<(u16, String), String> {
    let handle = registry
        .get_handle(name)
        .ok_or_else(|| format!("missing handle for {name}"))?;

    let req = http::Request::builder()
        .method("GET")
        .uri(path)
        .header("host", "localhost:9000")
        .body(bytes::Bytes::new())
        .map_err(|e| format!("build request: {e}"))?;

    let response = handle
        .send_request(req)
        .await
        .map_err(|e| format!("send request: {e}"))?;

    let text = match response.body {
        IsolateResponseBody::Full(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        IsolateResponseBody::Stream(mut rx) => {
            let mut out = Vec::new();
            while let Some(next) = rx.recv().await {
                let chunk = next.map_err(|e| format!("stream chunk error: {e}"))?;
                out.extend_from_slice(&chunk);
            }
            String::from_utf8(out).map_err(|e| format!("stream utf8: {e}"))?
        }
    };

    Ok((response.parts.status.as_u16(), text))
}

#[test]
fn sandbox_blocks_private_fetch_targets() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async (req) => {
              const target = req.url.endsWith('/local')
                ? 'http://127.0.0.1:65535/ping'
                : 'http://169.254.169.254/latest/meta-data';
              try {
                await fetch(target);
                return new Response('unexpected-allow', { status: 200 });
              } catch (err) {
                return new Response(String(err), { status: 500 });
              }
            });
        "#;

        let registry = deploy_inline_function("sandbox-ssrf-block", source).await?;

        let (status_local, body_local) =
            invoke_text(&registry, "sandbox-ssrf-block", "/local").await?;
        if status_local != 500 || !body_local.contains("Requires net access") {
            return Err(format!(
                "expected localhost fetch to be blocked; status={status_local}, body={body_local}"
            ));
        }

        let (status_meta, body_meta) =
            invoke_text(&registry, "sandbox-ssrf-block", "/metadata").await?;
        if status_meta != 500 || !body_meta.contains("Requires net access") {
            return Err(format!(
                "expected metadata fetch to be blocked; status={status_meta}, body={body_meta}"
            ));
        }

        registry
            .delete("sandbox-ssrf-block")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_allows_public_fetch_host_policy_for_httpbin() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
              try {
                const resp = await fetch('https://httpbin.org/get');
                return new Response('ok:' + String(resp.status));
              } catch (err) {
                return new Response(String(err), { status: 500 });
              }
            });
        "#;

        let registry = deploy_inline_function("sandbox-public-host", source).await?;
        let (status, body) = invoke_text(&registry, "sandbox-public-host", "/public").await?;

        // This checks SSRF policy behavior. Network may still fail in CI (DNS/TLS),
        // but failure must not be the internal net-permission denial.
        if status == 500 && body.contains("Requires net access") {
            return Err(format!(
                "public fetch host should not be denied by SSRF policy: {body}"
            ));
        }

        if status != 200 && status != 500 {
            return Err(format!(
                "unexpected status for public fetch check: {status}"
            ));
        }

        registry
            .delete("sandbox-public-host")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_denies_deno_readfile_and_env_get() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
              let readFileState = 'missing';
              if (typeof Deno.readFile === 'function') {
                try {
                  await Deno.readFile('/etc/hosts');
                  readFileState = 'allowed';
                } catch (err) {
                  readFileState = 'denied:' + String(err);
                }
              }

              let envState = 'missing';
              if (Deno.env && typeof Deno.env.get === 'function') {
                try {
                  const v = Deno.env.get('EDGE_RUNTIME_SANDBOX_TEST');
                  envState = 'allowed:' + String(v);
                } catch (err) {
                  envState = 'denied:' + String(err);
                }
              }

              return new Response(JSON.stringify({ readFileState, envState }));
            });
        "#;

        let registry = deploy_inline_function("sandbox-deno-apis", source).await?;
        let (status, body) = invoke_text(&registry, "sandbox-deno-apis", "/deno-apis").await?;
        if status != 200 {
            return Err(format!("unexpected response status: {status}, body={body}"));
        }

        let payload: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse json: {e}; body={body}"))?;
        let read_file_state = payload
            .get("readFileState")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let env_state = payload
            .get("envState")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if read_file_state == "allowed" {
            return Err("Deno.readFile unexpectedly allowed inside sandbox".to_string());
        }
        if env_state.starts_with("allowed:") {
            return Err(format!("Deno.env.get unexpectedly allowed: {env_state}"));
        }

        registry
            .delete("sandbox-deno-apis")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_blocks_prototype_pollution_via_object_prototype_proto() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
              const before = ({}).polluted;
              try {
                Object.prototype.__proto__ = { polluted: true };
              } catch (_err) {
                // Expected in hardened runtimes.
              }
              const after = ({}).polluted;
              return new Response(JSON.stringify({ before, after }));
            });
        "#;

        let registry = deploy_inline_function("sandbox-proto-pollution", source).await?;
        let (status, body) =
            invoke_text(&registry, "sandbox-proto-pollution", "/pollution").await?;
        if status != 200 {
            return Err(format!("unexpected response status: {status}, body={body}"));
        }

        let payload: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse json: {e}; body={body}"))?;
        let after = payload
            .get("after")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if after == serde_json::Value::Bool(true) {
            return Err("prototype pollution leaked to plain objects".to_string());
        }

        registry
            .delete("sandbox-proto-pollution")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_eval_constructor_returns_only_sandbox_global() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
              try {
                const escaped = (1, eval)(
                  'this.constructor.constructor("return globalThis")()',
                );

                let processExitDenied = false;
                try {
                  escaped.process.exit(1);
                } catch (err) {
                  processExitDenied = String(err).includes(
                    '[thunder] process.exit is not implemented in this runtime profile',
                  );
                }

                const payload = {
                  sameGlobal: escaped === globalThis,
                  hasProcessObject: typeof escaped.process === 'object',
                  processExitDenied,
                  denoEnvHidden: typeof escaped.Deno?.env === 'undefined',
                  denoReadFileHidden: typeof escaped.Deno?.readFile === 'undefined',
                  requireHidden: typeof escaped.require === 'undefined',
                };

                return new Response(JSON.stringify(payload));
              } catch (err) {
                return new Response(String(err), { status: 500 });
              }
            });
        "#;

        let registry = deploy_inline_function("sandbox-eval-escape-attempt", source).await?;
        let (status, body) = invoke_text(&registry, "sandbox-eval-escape-attempt", "/").await?;

        if status != 200 {
            return Err(format!("unexpected response status: {status}, body={body}"));
        }

        let payload: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse json: {e}; body={body}"))?;

        let same_global = payload
            .get("sameGlobal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_process_object = payload
            .get("hasProcessObject")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let process_exit_denied = payload
            .get("processExitDenied")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let deno_env_hidden = payload
            .get("denoEnvHidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let deno_read_file_hidden = payload
            .get("denoReadFileHidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let require_hidden = payload
            .get("requireHidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !(same_global
            && has_process_object
            && process_exit_denied
            && deno_env_hidden
            && deno_read_file_hidden
            && require_hidden)
        {
            return Err(format!(
                "unexpected sandbox escape probe result: {}",
                payload
            ));
        }

        registry
            .delete("sandbox-eval-escape-attempt")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_function_constructor_global_symbol_is_not_available() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
              try {
                // Node-style `global` is not expected to exist in this runtime global scope.
                const getGlobal = Function('return global');
                getGlobal();
                return new Response('unexpected-allow', { status: 200 });
              } catch (err) {
                return new Response(String(err), { status: 500 });
              }
            });
        "#;

        let registry = deploy_inline_function("sandbox-function-global-symbol", source).await?;
        let (status, body) = invoke_text(&registry, "sandbox-function-global-symbol", "/").await?;

        if status != 500 {
            return Err(format!(
                "expected constructor probe to fail; status={status}, body={body}"
            ));
        }

        if !body.contains("global is not defined") {
            return Err(format!(
                "expected deterministic missing-global failure; body={body}"
            ));
        }

        registry
            .delete("sandbox-function-global-symbol")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_permissions_surface_has_no_escalation_path() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
              const perms = globalThis.Deno?.permissions;
              const hasRequest = typeof perms?.request === 'function';
              const hasRevoke = typeof perms?.revoke === 'function';
              const hasQuery = typeof perms?.query === 'function';

              let envQueryState = 'unavailable';
              if (hasQuery) {
                try {
                  const status = await perms.query({ name: 'env' });
                  envQueryState = String(status?.state ?? 'unknown');
                } catch (err) {
                  envQueryState = 'error:' + String(err);
                }
              }

              return new Response(JSON.stringify({
                hasRequest,
                hasRevoke,
                hasQuery,
                envQueryState,
              }));
            });
        "#;

        let registry = deploy_inline_function("sandbox-permissions-surface", source).await?;
        let (status, body) = invoke_text(&registry, "sandbox-permissions-surface", "/").await?;

        if status != 200 {
            return Err(format!("unexpected response status: {status}, body={body}"));
        }

        let payload: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse json: {e}; body={body}"))?;

        let has_request = payload
            .get("hasRequest")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let has_revoke = payload
            .get("hasRevoke")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let env_query_state = payload
            .get("envQueryState")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if has_request || has_revoke {
            return Err(format!(
                "permissions escalation methods should be unavailable: {}",
                payload
            ));
        }

        if env_query_state == "granted" {
            return Err(format!(
                "env permission should not be granted in sandbox: {}",
                payload
            ));
        }

        registry
            .delete("sandbox-permissions-surface")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_detects_ipv6_ssrf_bypass_vectors() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async () => {
                            const perms = globalThis.Deno?.permissions;
                            const query = perms?.query;
                            async function fetchWithTimeout(url) {
                                const controller = new AbortController();
                                const timer = setTimeout(() => controller.abort('timeout'), 250);
                                try {
                                    await fetch(url, { signal: controller.signal });
                                    return 'allowed';
                                } catch (err) {
                                    return String(err);
                                } finally {
                                    clearTimeout(timer);
                                }
                            }

                            const targets = [
                                { host: '127.0.0.1:65535', family: 'ipv4-control' },
                                { host: '[::ffff:169.254.169.254]:80', family: 'ipv6-mapped-metadata' },
                                { host: '[fd00::1]:8080', family: 'ipv6-ula' },
                                { host: '[fe80::1]:80', family: 'ipv6-link-local' },
              ];

              const checks = [];
                            if (typeof query === 'function') {
                                for (const target of targets) {
                                    try {
                                        const status = await query.call(perms, { name: 'net', host: target.host });
                                        checks.push({
                                            host: target.host,
                                            family: target.family,
                                            state: String(status?.state ?? 'unknown'),
                                        });
                                    } catch (err) {
                                        checks.push({
                                            host: target.host,
                                            family: target.family,
                                            state: 'error:' + String(err),
                                        });
                                    }
                }
                            } else {
                                for (const target of targets) {
                                    const outcome = await fetchWithTimeout(`http://${target.host}/`);
                                    checks.push({
                                        host: target.host,
                                        family: target.family,
                                        state: outcome.includes('Requires net access') ? 'denied' : `bypass:${outcome}`,
                                    });
                                }
              }

                            return new Response(JSON.stringify({ queryAvailable: typeof query === 'function', checks }));
            });
        "#;

        let registry = deploy_inline_function("sandbox-ipv6-ssrf-detect", source).await?;
        let (status, body) = invoke_text(&registry, "sandbox-ipv6-ssrf-detect", "/").await?;

        if status != 200 {
            return Err(format!("unexpected response status: {status}, body={body}"));
        }

        let payload: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse json: {e}; body={body}"))?;

        let checks = payload
            .get("checks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| format!("missing checks array in payload: {payload}"))?;

        if checks.len() != 4 {
            return Err(format!("expected 4 ipv6 probe checks, got {}", checks.len()));
        }

        let ipv4_control_denied = checks.iter().any(|entry| {
            entry
                .get("family")
                .and_then(|v| v.as_str())
                .map(|family| family == "ipv4-control")
                .unwrap_or(false)
                && entry
                    .get("state")
                    .and_then(|v| v.as_str())
                    .map(|state| state == "denied")
                    .unwrap_or(false)
        });

        let bypass_detected = checks.iter().any(|entry| {
            entry
                .get("family")
                .and_then(|v| v.as_str())
                .map(|family| family.starts_with("ipv6"))
                .unwrap_or(false)
                && entry
                    .get("state")
                    .and_then(|v| v.as_str())
                    .map(|state| state != "denied")
                    .unwrap_or(true)
        });

        if !ipv4_control_denied {
            return Err(format!(
                "expected ipv4 control host to be denied in sandbox policy baseline: {}",
                payload
            ));
        }

        if !bypass_detected {
            return Err(format!(
                "expected at least one IPv6 SSRF vector to remain non-denied for bypass detection: {}",
                payload
            ));
        }

        registry
            .delete("sandbox-ipv6-ssrf-detect")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}

#[test]
fn sandbox_detects_cross_request_monkey_patch_leak_on_non_frozen_globals() {
    init_runtime();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let result: Result<(), String> = rt.block_on(async {
        let source = r#"
            Deno.serve(async (req) => {
              const path = new URL(req.url).pathname;

              if (path === '/patch') {
                globalThis.__edgeLeakMarker = 'patched';
                                globalThis.atob = () => 'patched-atob';
                return new Response(JSON.stringify({ patched: true }));
              }

              if (path === '/check') {
                                let patched = false;
                                try {
                                    patched = globalThis.atob('ignored') === 'patched-atob';
                                } catch {
                                    patched = false;
                                }

                return new Response(JSON.stringify({
                  marker: globalThis.__edgeLeakMarker ?? null,
                                    atobPatched: patched,
                }));
              }

              return new Response('not-found', { status: 404 });
            });
        "#;

        let registry = deploy_inline_function("sandbox-monkey-patch-leak", source).await?;

        let (patch_status, patch_body) =
            invoke_text(&registry, "sandbox-monkey-patch-leak", "/patch").await?;
        if patch_status != 200 {
            return Err(format!(
                "patch request failed; status={patch_status}, body={patch_body}"
            ));
        }

        let (check_status, check_body) =
            invoke_text(&registry, "sandbox-monkey-patch-leak", "/check").await?;
        if check_status != 200 {
            return Err(format!(
                "check request failed; status={check_status}, body={check_body}"
            ));
        }

        let payload: serde_json::Value = serde_json::from_str(&check_body)
            .map_err(|e| format!("parse json: {e}; body={check_body}"))?;

        let marker = payload
            .get("marker")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let atob_patched = payload
            .get("atobPatched")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if marker != "patched" || !atob_patched {
            return Err(format!(
                "expected monkey-patch leakage across requests for detection purposes: {}",
                payload
            ));
        }

        registry
            .delete("sandbox-monkey-patch-leak")
            .await
            .map_err(|e| format!("delete failed: {e}"))?;

        Ok(())
    });

    assert!(result.is_ok(), "test failed: {:?}", result.err());
}
