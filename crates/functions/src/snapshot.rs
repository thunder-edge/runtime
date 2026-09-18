use std::rc::Rc;
use std::sync::Arc;

use anyhow::Error;
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use runtime_core::extensions;
use runtime_core::isolate::{determine_root_specifier, IsolateConfig, OutgoingProxyConfig};
use runtime_core::isolate_logs::IsolateLogConfig;
use runtime_core::manifest::ResolvedFunctionManifest;
use runtime_core::module_loader::{EszipModuleLoader, ModuleCodeCacheMap};
use runtime_core::permissions::create_permissions_with_policy;

use crate::handler;
use crate::runtime_base_snapshot::{
    RUNTIME_BASE_RESIDUAL_LAZY_ESM_SOURCES, RUNTIME_BASE_RESIDUAL_LAZY_JS_SOURCES,
    RUNTIME_BASE_STARTUP_SNAPSHOT,
};

const BYTECODE_CACHE_MAGIC: [u8; 8] = *b"TBCCACHE";

#[derive(Debug, Serialize, Deserialize)]
struct FunctionBytecodeCacheEnvelope {
    magic: [u8; 8],
    version: u32,
    entries: ModuleCodeCacheMap,
}

pub fn decode_function_bytecode_cache(payload: &[u8]) -> Option<ModuleCodeCacheMap> {
    let envelope: FunctionBytecodeCacheEnvelope = match bincode::deserialize(payload) {
        Ok(envelope) => envelope,
        Err(err) => {
            warn!("failed to decode function bytecode cache envelope: {}", err);
            return None;
        }
    };
    if envelope.magic != BYTECODE_CACHE_MAGIC || envelope.version != 1 {
        warn!(
            "invalid function bytecode cache envelope (magic/version mismatch): version={}",
            envelope.version
        );
        return None;
    }
    info!(
        "decoded function bytecode cache envelope with {} module(s)",
        envelope.entries.len()
    );
    Some(envelope.entries)
}

pub async fn create_function_bytecode_cache_from_eszip(
    eszip_bytes: Vec<u8>,
    config: &IsolateConfig,
    outgoing_proxy: &OutgoingProxyConfig,
    manifest: Option<&ResolvedFunctionManifest>,
    function_name: &str,
) -> Result<Vec<u8>, Error> {
    let reader = futures_util::io::BufReader::new(futures_util::io::Cursor::new(eszip_bytes));
    let (eszip, loader_fut) = eszip::EszipV2::parse(reader)
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse eszip for snapshot creation: {e}"))?;
    tokio::spawn(loader_fut);

    let eszip = Arc::new(eszip);
    let root_specifier = determine_root_specifier(&eszip)?;

    let module_loader = Rc::new(EszipModuleLoader::new_with_source_maps_and_code_cache(
        eszip,
        config.enable_source_maps,
        Some(Default::default()),
    ));

    let mut runtime_extensions =
        extensions::get_extensions_with_ssrf_config(false, &config.ssrf_config);
    runtime_extensions.push(handler::response_stream_extension());
    runtime_extensions.push(extensions::runtime_bootstrap_extension());

    let create_params = if config.max_heap_size_bytes > 0 {
        Some(deno_core::v8::CreateParams::default().heap_limits(0, config.max_heap_size_bytes))
    } else {
        None
    };

    let mut runtime_opts = RuntimeOptions {
        module_loader: Some(module_loader.clone()),
        create_params,
        extensions: runtime_extensions,
        startup_snapshot: Some(RUNTIME_BASE_STARTUP_SNAPSHOT),
        residual_lazy_js_sources: RUNTIME_BASE_RESIDUAL_LAZY_JS_SOURCES,
        residual_lazy_esm_sources: RUNTIME_BASE_RESIDUAL_LAZY_ESM_SOURCES,
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut runtime_opts);

    // Use regular JsRuntime: deno_core disables module code-cache generation
    // when running in snapshotting mode (`will_snapshot`).
    let mut js_runtime = JsRuntime::new(runtime_opts);

    handler::ensure_response_stream_registry(&mut js_runtime);

    {
        let mut env_allow = None;
        let mut net_allow = None;
        if let Some(policy) = manifest {
            let mut merged_env = policy.env_allow.clone();
            for secret_name in &policy.env_secret_refs {
                if !merged_env.iter().any(|name| name == secret_name) {
                    merged_env.push(secret_name.clone());
                }
            }
            env_allow = Some(merged_env);
            net_allow = Some(policy.network_allow.clone());
        }

        let op_state = js_runtime.op_state();
        let mut state = op_state.borrow_mut();
        state.put(create_permissions_with_policy(
            &config.ssrf_config,
            net_allow,
            env_allow,
        ));
        state.put(IsolateLogConfig {
            function_name: function_name.to_string(),
            emit_to_stdout: config.print_isolate_logs,
        });
    }

    handler::inject_request_bridge_with_proxy_and_config(&mut js_runtime, outgoing_proxy, config)?;

    let module_id = js_runtime.load_main_es_module(&root_specifier).await?;
    let eval_result = js_runtime.mod_evaluate(module_id);

    js_runtime
        .run_event_loop(PollEventLoopOptions {
            wait_for_inspector: false,
        })
        .await?;

    eval_result.await?;
    handler::register_handler_from_module_exports(&mut js_runtime, &root_specifier).await?;

    let code_cache = module_loader.code_cache_snapshot().unwrap_or_default();
    info!(
        "generated function bytecode cache with {} module(s) for '{}'",
        code_cache.len(),
        function_name
    );
    let envelope = FunctionBytecodeCacheEnvelope {
        magic: BYTECODE_CACHE_MAGIC,
        version: 1,
        entries: code_cache,
    };
    let payload = bincode::serialize(&envelope)
        .map_err(|e| anyhow::anyhow!("failed to serialize bytecode cache envelope: {e}"))?;

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use deno_core::RuntimeOptions;

    use crate::handler;
    use crate::runtime_base_snapshot::{
        RUNTIME_BASE_RESIDUAL_LAZY_ESM_SOURCES, RUNTIME_BASE_RESIDUAL_LAZY_JS_SOURCES,
        RUNTIME_BASE_STARTUP_SNAPSHOT,
    };
    use runtime_core::extensions;

    #[test]
    fn runtime_base_snapshot_boots_with_production_extensions() {
        deno_core::JsRuntime::init_platform(None);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime");
        let _guard = runtime.enter();

        let mut extension_stack = extensions::get_extensions();
        extension_stack.push(handler::response_stream_extension());
        extension_stack.push(extensions::runtime_bootstrap_extension());
        let mut options = RuntimeOptions {
            extensions: extension_stack,
            startup_snapshot: Some(RUNTIME_BASE_STARTUP_SNAPSHOT),
            residual_lazy_js_sources: RUNTIME_BASE_RESIDUAL_LAZY_JS_SOURCES,
            residual_lazy_esm_sources: RUNTIME_BASE_RESIDUAL_LAZY_ESM_SOURCES,
            ..Default::default()
        };
        extensions::set_extension_transpiler(&mut options);

        let mut js_runtime = deno_core::JsRuntime::new(options);
        let value = js_runtime
            .execute_script(
                "<snapshot-bootstrap-test>",
                "[
                  typeof URL,
                  typeof Request,
                  typeof Response,
                  typeof fetch,
                  typeof crypto,
                  typeof ReadableStream,
                ].join(',')",
            )
            .expect("evaluate Web API availability");
        deno_core::scope!(scope, &mut js_runtime);
        let value = value
            .open(scope)
            .to_string(scope)
            .expect("Web API availability is a string")
            .to_rust_string_lossy(scope);
        assert_eq!(value, "function,function,function,function,object,function");
    }
}
