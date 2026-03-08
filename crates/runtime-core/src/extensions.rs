use std::borrow::Cow;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deno_ast::{EmitOptions, MediaType, ParseParams, TranspileModuleOptions, TranspileOptions};
use deno_core::url::Url;
use deno_core::{op2, Extension, ModuleCodeString, ModuleName, OpState, RuntimeOptions, SourceMapData};
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use hmac::{Hmac, Mac};
use node_resolver::errors::{
    PackageFolderResolveError, PackageFolderResolveErrorKind, PackageNotFoundError,
};
use node_resolver::{InNpmPackageChecker, NpmPackageFolderResolver, UrlOrPathRef};
use sha2::{Digest, Sha256, Sha512};
use tracing::{error, info, warn};

use crate::isolate_logs::{push_collected_log, IsolateConsoleLog, IsolateLogConfig};

// Bootstrap extension: imports all extension ESM modules so they get evaluated.
//
// deno_core loads extension ESM as side-modules but only evaluates those
// reachable from an `esm_entry_point`.  None of the upstream deno_* extensions
// set an entry point, so we provide one here that pulls everything in.
deno_core::extension!(
    edge_bootstrap,
    esm_entry_point = "ext:edge_bootstrap/bootstrap.js",
    esm = [dir "src", "bootstrap.js"],
);

// Node built-ins for compatibility with SSR/tooling ecosystems.
// Modules are registered as Full/Partial/Stub, and stubs remain importable
// (failing deterministically only when unsupported methods are invoked).
deno_core::extension!(
    edge_node_compat,
    ops = [
        op_edge_zlib_transform,
        op_edge_crypto_hash,
        op_edge_crypto_hmac,
    ],
    esm_entry_point = "ext:edge_node_compat/mod.ts",
    esm = [
        "ext:edge_node_compat/mod.ts" = "src/node_compat/mod.ts",
        "node:assert" = "src/node_compat/assert.ts",
        "node:async_hooks" = "src/node_compat/async_hooks.ts",
        "node:buffer" = "src/node_compat/buffer.ts",
        "node:child_process" = "src/node_compat/child_process.ts",
        "node:cluster" = "src/node_compat/cluster.ts",
        "node:console" = "src/node_compat/console.ts",
        "node:process" = "src/node_compat/process.ts",
        "node:crypto" = "src/node_compat/crypto.ts",
        "node:dgram" = "src/node_compat/dgram.ts",
        "node:diagnostics_channel" = "src/node_compat/diagnostics_channel.ts",
        "node:dns" = "src/node_compat/dns.ts",
        "node:events" = "src/node_compat/events.ts",
        "node:fs" = "src/node_compat/fs.ts",
        "node:fs/promises" = "src/node_compat/fs_promises.ts",
        "node:http" = "src/node_compat/http.ts",
        "node:http2" = "src/node_compat/http2.ts",
        "node:https" = "src/node_compat/https.ts",
        "node:inspector" = "src/node_compat/inspector.ts",
        "node:module" = "src/node_compat/module.ts",
        "node:net" = "src/node_compat/net.ts",
        "node:os" = "src/node_compat/os.ts",
        "node:stream" = "src/node_compat/stream.ts",
        "node:perf_hooks" = "src/node_compat/perf_hooks.ts",
        "node:punycode" = "src/node_compat/punycode.ts",
        "node:querystring" = "src/node_compat/querystring.ts",
        "node:readline" = "src/node_compat/readline.ts",
        "node:repl" = "src/node_compat/repl.ts",
        "node:request" = "src/node_compat/request.ts",
        "node:sqlite" = "src/node_compat/sqlite.ts",
        "node:string_decoder" = "src/node_compat/string_decoder.ts",
        "node:test" = "src/node_compat/test.ts",
        "node:timers" = "src/node_compat/timers.ts",
        "node:timers/promises" = "src/node_compat/timers_promises.ts",
        "node:tls" = "src/node_compat/tls.ts",
        "node:util" = "src/node_compat/util.ts",
        "node:v8" = "src/node_compat/v8.ts",
        "node:vm" = "src/node_compat/vm.ts",
        "node:path" = "src/node_compat/path.ts",
        "node:url" = "src/node_compat/url.ts",
        "node:zlib" = "src/node_compat/zlib.ts",
    ],
);

const DEFAULT_ZLIB_MAX_OUTPUT_LENGTH: usize = 16 * 1024 * 1024;
const ZLIB_HARD_MAX_OUTPUT_LENGTH: usize = 64 * 1024 * 1024;
const ZLIB_HARD_MAX_INPUT_LENGTH: usize = 8 * 1024 * 1024;
const DEFAULT_ZLIB_OPERATION_TIMEOUT_MS: u64 = 250;

fn read_all_limited<R: Read>(
    reader: &mut R,
    max_output_length: usize,
    operation_timeout: Duration,
) -> Result<Vec<u8>, deno_error::JsErrorBox> {
    let mut out = Vec::new();
    let mut chunk = [0_u8; 8192];
    let started = Instant::now();

    loop {
        if started.elapsed() > operation_timeout {
            return Err(deno_error::JsErrorBox::generic("zlib operation timeout exceeded"));
        }

        let read = reader
            .read(&mut chunk)
            .map_err(|err| deno_error::JsErrorBox::generic(format!("zlib read failed: {err}")))?;
        if read == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..read]);
        if out.len() > max_output_length {
            return Err(deno_error::JsErrorBox::generic(format!(
                "zlib output exceeds maxOutputLength ({max_output_length} bytes)",
            )));
        }
    }

    Ok(out)
}

fn compress_with_limit(
    format: &str,
    input: &[u8],
    max_output_length: usize,
    operation_timeout: Duration,
) -> Result<Vec<u8>, deno_error::JsErrorBox> {
    let started = Instant::now();

    let compressed = match format {
        "gzip" => {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(input)
                .map_err(|err| deno_error::JsErrorBox::generic(format!("gzip write failed: {err}")))?;
            encoder
                .finish()
                .map_err(|err| deno_error::JsErrorBox::generic(format!("gzip finish failed: {err}")))?
        }
        "deflate" => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(input)
                .map_err(|err| deno_error::JsErrorBox::generic(format!("deflate write failed: {err}")))?;
            encoder
                .finish()
                .map_err(|err| deno_error::JsErrorBox::generic(format!("deflate finish failed: {err}")))?
        }
        "deflate-raw" => {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(input)
                .map_err(|err| deno_error::JsErrorBox::generic(format!("deflateRaw write failed: {err}")))?;
            encoder
                .finish()
                .map_err(|err| deno_error::JsErrorBox::generic(format!("deflateRaw finish failed: {err}")))?
        }
        _ => {
            return Err(deno_error::JsErrorBox::generic(format!(
                "unsupported zlib format: {format}",
            )));
        }
    };

    if started.elapsed() > operation_timeout {
        return Err(deno_error::JsErrorBox::generic("zlib operation timeout exceeded"));
    }

    if compressed.len() > max_output_length {
        return Err(deno_error::JsErrorBox::generic(format!(
            "zlib output exceeds maxOutputLength ({max_output_length} bytes)",
        )));
    }

    Ok(compressed)
}

fn decompress_with_limit(
    format: &str,
    input: &[u8],
    max_output_length: usize,
    operation_timeout: Duration,
) -> Result<Vec<u8>, deno_error::JsErrorBox> {
    match format {
        "gzip" => {
            let mut decoder = GzDecoder::new(input);
            read_all_limited(&mut decoder, max_output_length, operation_timeout)
        }
        "deflate" => {
            let mut decoder = ZlibDecoder::new(input);
            read_all_limited(&mut decoder, max_output_length, operation_timeout)
        }
        "deflate-raw" => {
            let mut decoder = DeflateDecoder::new(input);
            read_all_limited(&mut decoder, max_output_length, operation_timeout)
        }
        _ => Err(deno_error::JsErrorBox::generic(format!(
            "unsupported zlib format: {format}",
        ))),
    }
}

#[op2]
#[buffer]
fn op_edge_zlib_transform(
    #[string] format: String,
    #[string] mode: String,
    #[buffer] input: &[u8],
    max_output_length: Option<u32>,
    operation_timeout_ms: Option<u32>,
    max_input_length: Option<u32>,
) -> Result<Vec<u8>, deno_error::JsErrorBox> {
    let max_input_length = match max_input_length {
        Some(value) => {
            let as_usize = value as usize;
            if as_usize == 0 {
                return Err(deno_error::JsErrorBox::generic(
                    "maxInputLength must be greater than zero",
                ));
            }
            if as_usize > ZLIB_HARD_MAX_INPUT_LENGTH {
                return Err(deno_error::JsErrorBox::generic(format!(
                    "maxInputLength exceeds hard cap ({ZLIB_HARD_MAX_INPUT_LENGTH} bytes)",
                )));
            }
            as_usize
        }
        None => ZLIB_HARD_MAX_INPUT_LENGTH,
    };

    if input.len() > max_input_length {
        return Err(deno_error::JsErrorBox::generic(format!(
            "zlib input exceeds maxInputLength ({max_input_length} bytes)",
        )));
    }

    let max_output_length = match max_output_length {
        Some(value) => {
            let as_usize = value as usize;
            if as_usize == 0 {
                return Err(deno_error::JsErrorBox::generic(
                    "maxOutputLength must be greater than zero",
                ));
            }
            if as_usize > ZLIB_HARD_MAX_OUTPUT_LENGTH {
                return Err(deno_error::JsErrorBox::generic(format!(
                    "maxOutputLength exceeds hard cap ({ZLIB_HARD_MAX_OUTPUT_LENGTH} bytes)",
                )));
            }
            as_usize
        }
        None => DEFAULT_ZLIB_MAX_OUTPUT_LENGTH,
    };

    let operation_timeout_ms = operation_timeout_ms
        .map(|value| value as u64)
        .unwrap_or(DEFAULT_ZLIB_OPERATION_TIMEOUT_MS);
    if operation_timeout_ms == 0 {
        return Err(deno_error::JsErrorBox::generic(
            "operationTimeoutMs must be greater than zero",
        ));
    }
    let operation_timeout = Duration::from_millis(operation_timeout_ms);

    match mode.as_str() {
        "compress" => compress_with_limit(&format, input, max_output_length, operation_timeout),
        "decompress" => decompress_with_limit(&format, input, max_output_length, operation_timeout),
        _ => Err(deno_error::JsErrorBox::generic(format!(
            "unsupported zlib mode: {mode}",
        ))),
    }
}

// ============ Crypto Native Operations ============

/// Native synchronous hash operation using sha2/sha512
#[op2]
#[buffer]
pub fn op_edge_crypto_hash(
    #[string] algorithm: String,
    #[buffer] data: &[u8],
) -> Result<Vec<u8>, deno_error::JsErrorBox> {
    match algorithm.as_str() {
        "SHA-256" => {
            let mut hasher = Sha256::new();
            hasher.update(data);
            Ok(hasher.finalize().to_vec())
        }
        "SHA-512" => {
            let mut hasher = Sha512::new();
            hasher.update(data);
            Ok(hasher.finalize().to_vec())
        }
        algo => Err(deno_error::JsErrorBox::generic(format!(
            "unsupported hash algorithm: {algo}"
        ))),
    }
}

/// Native synchronous HMAC operation
#[op2]
#[buffer]
pub fn op_edge_crypto_hmac(
    #[string] algorithm: String,
    #[buffer] key: &[u8],
    #[buffer] data: &[u8],
) -> Result<Vec<u8>, deno_error::JsErrorBox> {
    match algorithm.as_str() {
        "SHA-256" => {
            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(key)
                .map_err(|_| deno_error::JsErrorBox::generic("invalid HMAC key length"))?;
            mac.update(data);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        "SHA-512" => {
            type HmacSha512 = Hmac<Sha512>;
            let mut mac = HmacSha512::new_from_slice(key)
                .map_err(|_| deno_error::JsErrorBox::generic("invalid HMAC key length"))?;
            mac.update(data);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        algo => Err(deno_error::JsErrorBox::generic(format!(
            "unsupported HMAC algorithm: {algo}"
        ))),
    }
}

// Shims for deno_node: provides stubs for modules that would normally come from
// the full Deno runtime. This allows deno_node to work in a standalone edge runtime.
// The extension name is "runtime" so that imports like "ext:runtime/..." resolve here.
deno_core::extension!(
    runtime,
    esm = [dir "src", "98_global_scope_shared.js" = "global_scope_shim.js"],
);

// Shim for deno_http - provides stubs for HTTP server functionality
// that deno_node expects but isn't needed in an edge runtime.
deno_core::extension!(
    deno_http,
    esm = [dir "src", "00_serve.ts" = "http_serve_shim.js"],
);

// Shim for deno_node - provides minimal constants needed by deno_crypto
// Maps: ext:deno_node/internal/crypto/constants.ts
deno_core::extension!(
    deno_node,
    esm = [
        dir "src/internal/crypto",
        "ext:deno_node/internal/crypto/constants.ts" = "constants.ts",
    ],
);

// Native assert module for user code running in the edge runtime.
// Usage: import { assert, assertEquals } from "ext:edge_assert/mod.ts";
deno_core::extension!(
    edge_assert,
    esm = [
        dir "src/assert",
        "ext:edge_assert/assert.ts" = "assert.ts",
        "ext:edge_assert/mod.ts" = "mod.ts",
        "ext:edge_assert/mock/mod.ts" = "mock/mod.ts",
        "ext:edge_assert/mock/mockFn.ts" = "mock/mockFn.ts",
        "ext:edge_assert/mock/spy.ts" = "mock/spy.ts",
        "ext:edge_assert/mock/fetch.ts" = "mock/fetch.ts",
        "ext:edge_assert/mock/time.ts" = "mock/time.ts",
    ],
);

// === Stub ops for edge runtime ===
// These ops are referenced by deno_io and other extensions but are not needed
// in an edge runtime environment (no raw TTY, no interactive terminal).

/// Stub for op_set_raw - TTY raw mode is not supported in edge runtime
#[op2(fast)]
fn op_set_raw(_rid: u32, _mode: bool, _cbreak: bool) -> Result<(), deno_error::JsErrorBox> {
    // No-op: TTY raw mode is not supported in edge runtime
    Ok(())
}

/// Stub for op_console_size - no real console in edge runtime
#[op2]
fn op_console_size(_rid: u32) -> Result<(u32, u32), deno_error::JsErrorBox> {
    // Return default size (terminal size not available in edge runtime)
    Ok((80, 24))
}

/// Stub for op_tls_peer_certificate - TLS peer certificates not available in edge runtime
#[op2]
fn op_tls_peer_certificate(_rid: u32) -> Result<Option<Vec<Vec<u8>>>, deno_error::JsErrorBox> {
    // Not supported in edge runtime
    Ok(None)
}

// Extension that provides stub ops for edge runtime compatibility
// Note: op_is_terminal is already provided by deno_core
deno_core::extension!(
    edge_stubs,
    ops = [op_set_raw, op_console_size, op_tls_peer_certificate,],
);

#[op2(fast)]
fn op_edge_runtime_console_log(
    state: &mut OpState,
    #[string] message: String,
    level: u8,
) -> Result<(), deno_error::JsErrorBox> {
    let config = state.try_borrow::<IsolateLogConfig>().cloned().unwrap_or_default();

    if config.emit_to_stdout {
        match level {
            0 => info!(
                function_name = %config.function_name,
                request_id = "isolate-console",
                target = "isolate",
                "{}",
                message.trim_end_matches('\n')
            ),
            1 => warn!(
                function_name = %config.function_name,
                request_id = "isolate-console",
                target = "isolate",
                "{}",
                message.trim_end_matches('\n')
            ),
            _ => error!(
                function_name = %config.function_name,
                request_id = "isolate-console",
                target = "isolate",
                "{}",
                message.trim_end_matches('\n')
            ),
        }
    } else {
        // TODO: expose this collector to an external log stack per function owner.
        push_collected_log(IsolateConsoleLog {
            timestamp: chrono::Utc::now(),
            function_name: config.function_name,
            request_id: "isolate-console".to_string(),
            level,
            message,
        });
    }

    Ok(())
}

deno_core::extension!(
    edge_runtime_logging,
    ops = [op_edge_runtime_console_log],
);

// === Stub types for deno_node (no npm support in edge runtime) ===

/// Stub npm package checker - always returns false (no npm packages).
#[derive(Clone)]
pub struct NoNpmPackageChecker;

impl InNpmPackageChecker for NoNpmPackageChecker {
    fn in_npm_package(&self, _specifier: &Url) -> bool {
        false // No specifiers are in npm packages
    }
}

/// Stub npm package folder resolver - always returns error (no npm packages).
#[derive(Clone)]
pub struct NoNpmPackageFolderResolver;

impl NpmPackageFolderResolver for NoNpmPackageFolderResolver {
    fn resolve_package_folder_from_package(
        &self,
        name: &str,
        referrer: &UrlOrPathRef,
    ) -> Result<PathBuf, PackageFolderResolveError> {
        Err(PackageFolderResolveError(Box::new(
            PackageFolderResolveErrorKind::PackageNotFound(PackageNotFoundError {
                package_name: name.to_string(),
                referrer: referrer.display(),
                referrer_extra: None,
            }),
        )))
    }

    fn resolve_types_package_folder(
        &self,
        _types_package_name: &str,
        _maybe_package_version: Option<&deno_semver::Version>,
        _maybe_referrer: Option<&UrlOrPathRef>,
    ) -> Option<PathBuf> {
        None // No types packages available
    }
}

/// Stub ExtNodeSys - uses RealSys but won't be used for npm resolution.
pub type EdgeNodeSys = sys_traits::impls::RealSys;

/// Build the set of Deno extensions to register on every isolate.
///
/// This provides the JS runtime with Web APIs (console, URL, fetch, crypto, etc).
/// deno_web now includes console and URL APIs (previously deno_console, deno_url).
///
/// SECURITY: This is configured as a secure edge sandbox:
/// - Filesystem access goes through permissions
/// - Network access goes through permissions
/// - No subprocess/child_process execution
/// - No FFI
///
/// The `edge_bootstrap` extension is registered last — its entry point imports
/// all other extension ESM modules, causing them to be evaluated.
pub fn get_extensions_with_edge_assert(include_edge_assert: bool) -> Vec<Extension> {
    // Shared filesystem - use Rc (deno_fs expects Rc not Arc by default)
    let fs: deno_fs::FileSystemRc = Rc::new(deno_fs::RealFs);

    let mut extensions = vec![
        // Runtime log routing for isolate console output.
        edge_runtime_logging::init(),
        // 0. Stub ops for edge runtime (TTY ops not needed in serverless)
        edge_stubs::init(),
        // 1. Core (no deps)
        deno_webidl::deno_webidl::init(),
        // 2. Web (depends on webidl) - includes console, URL, events, streams, etc.
        deno_web::deno_web::init(
            Arc::new(deno_web::BlobStore::default()),
            None,
            deno_web::InMemoryBroadcastChannel::default(),
        ),
        // 3. TLS (no deps) - required by deno_net
        deno_tls::deno_tls::init(),
        // 4. IO (depends on web) - stdio handling
        deno_io::deno_io::init(Some(deno_io::Stdio::default())),
        // 5. FS (depends on web) - filesystem ops (access controlled by permissions)
        deno_fs::deno_fs::init(fs.clone()),
        // 6. Net (depends on web) - network ops (access controlled by permissions)
        deno_net::deno_net::init(
            None, // root_cert_store_provider - uses default webpki roots
            None, // unsafely_ignore_certificate_errors
        ),
        // 7. Telemetry
        deno_telemetry::deno_telemetry::init(),
        // 8. Fetch (depends on web, net, tls) - fetch API
        deno_fetch::deno_fetch::init(deno_fetch::Options::default()),
        // 9. WebSocket client API (depends on web, webidl)
        deno_websocket::deno_websocket::init(),
        // 10. Minimal Node compatibility modules with native crypto ops
        edge_node_compat::init(),
        // 10. Node shim - provides minimal constants for deno_crypto
        deno_node::init(),
        // 11. Crypto (depends on webidl, web, node shim) - Web Crypto API
        deno_crypto::deno_crypto::init(None), // maybe_seed
    ];

    if include_edge_assert {
        // Built-in assert helpers for CLI test runtime.
        extensions.push(edge_assert::init());
    }

    // Bootstrap must be last — its entry point imports all extension modules.
    extensions.push(edge_bootstrap::init());

    extensions
}

pub fn get_extensions() -> Vec<Extension> {
    get_extensions_with_edge_assert(false)
}

/// Set the extension transpiler on `RuntimeOptions`.
///
/// Some deno extensions (e.g. `deno_telemetry`, `deno_node`) ship TypeScript source that
/// V8 cannot execute directly. This configures TS → JS transpilation during
/// JsRuntime initialisation.
pub fn set_extension_transpiler(opts: &mut RuntimeOptions) {
    opts.extension_transpiler = Some(Rc::new(|name: ModuleName, code: ModuleCodeString| {
        let specifier_str: &str = &name;

        // Handle different specifier formats:
        // - Regular URLs (file:, https:, ext:)
        // - Node.js built-in modules (node:*)
        let media_type = if specifier_str.starts_with("node:") {
            // Node.js polyfills from deno_node are TypeScript
            MediaType::TypeScript
        } else {
            let url = deno_core::url::Url::parse(specifier_str)
                .unwrap_or_else(|_| deno_core::url::Url::parse("file:///unknown.ts").unwrap());
            MediaType::from_specifier_and_headers(&url, None)
        };

        if !matches!(
            media_type,
            MediaType::TypeScript | MediaType::Mts | MediaType::Cts | MediaType::Tsx
        ) {
            return Ok((code, None));
        }

        // Create a synthetic URL for parsing (required by deno_ast)
        let url = if specifier_str.starts_with("node:") {
            // Convert node: specifier to a parseable URL
            deno_core::url::Url::parse(&format!("file:///{}.ts", &specifier_str[5..]))
                .unwrap_or_else(|_| deno_core::url::Url::parse("file:///unknown.ts").unwrap())
        } else {
            deno_core::url::Url::parse(specifier_str)
                .unwrap_or_else(|_| deno_core::url::Url::parse("file:///unknown.ts").unwrap())
        };

        let source_text: &str = &code;
        let parsed = deno_ast::parse_module(ParseParams {
            specifier: url,
            text: source_text.into(),
            media_type,
            capture_tokens: false,
            scope_analysis: false,
            maybe_syntax: None,
        })
        .map_err(|e| {
            deno_error::JsErrorBox::generic(format!("failed to parse {specifier_str}: {e}"))
        })?;

        let transpiled = parsed
            .transpile(
                &TranspileOptions::default(),
                &TranspileModuleOptions::default(),
                &EmitOptions::default(),
            )
            .map_err(|e| {
                deno_error::JsErrorBox::generic(format!("failed to transpile {specifier_str}: {e}"))
            })?;

        let emitted = transpiled.into_source();
        let source_map = emitted
            .source_map
            .map(|sm| Cow::Owned(sm.into_bytes()) as SourceMapData);

        Ok((ModuleCodeString::from(emitted.text), source_map))
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();
    fn init_v8() {
        INIT.call_once(|| {
            deno_core::JsRuntime::init_platform(None);
        });
    }

    #[test]
    fn get_extensions_returns_expected_count() {
        let exts = get_extensions();
        // 15 extensions by default (no edge_assert in production profile)
        assert_eq!(exts.len(), 15, "expected 15 extensions, got {}", exts.len());
    }

    #[test]
    fn get_extensions_with_assert_returns_expected_count() {
        let base_exts = get_extensions();
        let exts = get_extensions_with_edge_assert(true);
        // Profile with edge_assert should always add exactly one extension.
        assert_eq!(
            exts.len(),
            base_exts.len() + 1,
            "expected assert profile to add one extension (base={}, assert={})",
            base_exts.len(),
            exts.len()
        );
    }

    #[test]
    fn set_extension_transpiler_configures_opts() {
        let mut opts = RuntimeOptions::default();
        assert!(opts.extension_transpiler.is_none());
        set_extension_transpiler(&mut opts);
        assert!(opts.extension_transpiler.is_some());
    }

    #[test]
    fn runtime_boots_with_extensions() {
        init_v8();
        let mut opts = RuntimeOptions {
            extensions: get_extensions(),
            ..Default::default()
        };
        set_extension_transpiler(&mut opts);
        let _rt = deno_core::JsRuntime::new(opts);
    }
}
