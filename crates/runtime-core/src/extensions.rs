use std::borrow::Cow;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use deno_ast::{EmitOptions, MediaType, ParseParams, TranspileModuleOptions, TranspileOptions};
use deno_core::url::Url;
use deno_core::{op2, Extension, ModuleCodeString, ModuleName, RuntimeOptions, SourceMapData};
use node_resolver::errors::{
    PackageFolderResolveError, PackageFolderResolveErrorKind, PackageNotFoundError,
};
use node_resolver::{InNpmPackageChecker, NpmPackageFolderResolver, UrlOrPathRef};

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
        // 9. Node shim - provides minimal constants for deno_crypto
        deno_node::init(),
        // 10. Crypto (depends on webidl, web, node shim) - Web Crypto API
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
        // 12 extensions by default (no edge_assert in production profile)
        assert_eq!(exts.len(), 12, "expected 12 extensions, got {}", exts.len());
    }

    #[test]
    fn get_extensions_with_assert_returns_expected_count() {
        let exts = get_extensions_with_edge_assert(true);
        // 13 extensions with edge_assert enabled
        assert_eq!(exts.len(), 13, "expected 13 extensions, got {}", exts.len());
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
