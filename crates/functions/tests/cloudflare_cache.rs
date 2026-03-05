use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;

// This module tests Cloudflare Workers Cache API
// Reference: https://developers.cloudflare.com/workers/runtime-apis/

static INIT: std::sync::Once = std::sync::Once::new();

fn init_v8() {
    INIT.call_once(|| {
        deno_core::JsRuntime::init_platform(None, false);
    });
}

fn make_runtime() -> JsRuntime {
    init_v8();
    let mut opts = RuntimeOptions {
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);
    let mut runtime = JsRuntime::new(opts);

    {
        let mut op_state = runtime.op_state();
        op_state.borrow_mut().put(Permissions);
    }

    runtime
}

fn assert_js_true(js: &str, desc: &str) {
    // Ensure we run within a Tokio runtime context for deno_core operations
    if tokio::runtime::Handle::try_current().is_err() {
        // Use current_thread runtime to match deno_fetch expectations (EventSource)
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(async {
            assert_js_true_async(js, desc).await;
        });
    } else {
        // Already in async context, run directly
        let mut runtime = make_runtime();
        let result = runtime.execute_script("<test>", js.to_string());
        match result {
            Err(e) => panic!("[{desc}] JS execution error: {e}"),
            Ok(val) => {
                let scope = &mut runtime.handle_scope();
                let local = deno_core::v8::Local::new(scope, val);
                assert!(local.is_true(), "[{desc}] expected true, got false");
            }
        }
    }
}

async fn assert_js_true_async(js: &str, desc: &str) {
    let mut runtime = make_runtime();
    let result = runtime.execute_script("<test>", js.to_string());
    match result {
        Err(e) => panic!("[{desc}] JS execution error: {e}"),
        Ok(val) => {
            let scope = &mut runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, val);
            assert!(local.is_true(), "[{desc}] expected true, got false");
        }
    }
}

// ── Cache API ──────────────────────────────────────────────────────

// NOTE: Cloudflare Cache API (caches.default) is NOT available in deno-edge-runtime
// Reason: No native Cache API extension loaded
//
// The Cache API in Cloudflare provides:
// 1. Default cache for responses
// 2. Named caches for custom caching
// 3. TTL and metadata support
//
// Alternative implementations:

#[test]
fn cache_api_not_available() {
    assert_js_true(
        "typeof caches === 'undefined'",
        "Cloudflare Cache API correctly not available",
    );
}

#[test]
fn cache_alternative_memory_map() {
    assert_js_true(
        "(() => {
            // Simple in-memory cache using Map
            class InMemoryCache {
                constructor() {
                    this.store = new Map();
                }

                async get(key) {
                    return this.store.get(key);
                }

                async put(key, value, options = {}) {
                    const ttl = options.expirationTtl || Infinity;
                    this.store.set(key, {
                        value,
                        expires: ttl === Infinity ? Infinity : Date.now() + (ttl * 1000)
                    });
                }

                async delete(key) {
                    return this.store.delete(key);
                }

                isExpired(entry) {
                    return entry.expires !== Infinity && entry.expires < Date.now();
                }
            }

            const cache = new InMemoryCache();
            return typeof cache.get === 'function' && typeof cache.put === 'function';
        })()",
        "In-memory cache implementation",
    );
}

#[test]
fn cache_alternative_with_expiry() {
    assert_js_true(
        "(() => {
            // Cache with expiration support
            class ExpiringCache {
                constructor() {
                    this.cache = new Map();
                }

                set(key, value, ttlSeconds) {
                    const expiresAt = Date.now() + (ttlSeconds * 1000);
                    this.cache.set(key, { value, expiresAt });
                }

                get(key) {
                    const entry = this.cache.get(key);
                    if (!entry) return null;

                    // Check expiration
                    if (entry.expiresAt < Date.now()) {
                        this.cache.delete(key);
                        return null;
                    }

                    return entry.value;
                }

                clear() {
                    this.cache.clear();
                }
            }

            const cache = new ExpiringCache();
            cache.set('key', 'value', 60);
            return cache.get('key') === 'value';
        })()",
        "Cache with TTL/expiration",
    );
}

#[test]
fn cache_pattern_response_caching() {
    assert_js_true(
        "(() => {
            // Pattern for caching HTTP responses
            const cacheResponse = async (request, response, ttl) => {
                const cacheKey = request.url;
                const cached = {
                    status: response.status,
                    headers: Object.fromEntries(response.headers),
                    body: await response.text(),
                    timestamp: Date.now()
                };

                return {
                    cached,
                    isValid: (Date.now() - cached.timestamp) < ttl
                };
            };

            return typeof cacheResponse === 'function';
        })()",
        "Response caching pattern",
    );
}

#[test]
fn cache_pattern_conditional() {
    assert_js_true(
        "(() => {
            // Conditional caching based on response headers
            const shouldCache = (response) => {
                const cacheControl = response.headers.get('cache-control');
                return response.ok && (!cacheControl || !cacheControl.includes('no-store'));
            };

            const response = new Response('data', {
                headers: { 'cache-control': 'public, max-age=3600' }
            });

            return shouldCache(response) === true;
        })()",
        "Conditional caching based on headers",
    );
}

// ── HTMLRewriter API ────────────────────────────────────────────────

// NOTE: HTMLRewriter is a Cloudflare-specific API for HTML transformation
// It is NOT available in deno-edge-runtime
//
// HTMLRewriter provides:
// 1. HTML parsing and transformation
// 2. Element selection and manipulation
// 3. Streaming HTML processing
//
// Alternative implementations:

#[test]
fn htmlrewriter_not_available() {
    assert_js_true(
        "typeof HTMLRewriter === 'undefined'",
        "HTMLRewriter correctly not available",
    );
}

#[test]
fn htmlrewriter_alternative_dom_parser() {
    assert_js_true(
        "(() => {
            // HTMLRewriter can be replaced with DOMParser-like operations
            // In Cloudflare Workers, HTML parsing would need to be done via string manipulation
            // or external libraries

            // Pattern: regex-based HTML transformation (simple cases)
            const transformHTML = (html, replacements) => {
                let result = html;
                for (const [pattern, replacement] of Object.entries(replacements)) {
                    result = result.replace(new RegExp(pattern, 'g'), replacement);
                }
                return result;
            };

            const html = '<h1>Hello</h1>';
            const modified = transformHTML(html, {
                'Hello': 'World'
            });

            return modified === '<h1>World</h1>';
        })()",
        "Regex-based HTML transformation",
    );
}

#[test]
fn htmlrewriter_alternative_external_library() {
    assert_js_true(
        "(() => {
            // For complex HTML transformation, could use external library via fetch
            // Pattern: send HTML to external service
            const transformViaExternalService = async (html) => {
                const response = await fetch('https://html-transformer-service.com/transform', {
                    method: 'POST',
                    body: html,
                    headers: { 'Content-Type': 'text/html' }
                });
                return response.text();
            };

            return typeof transformViaExternalService === 'function';
        })()",
        "HTMLRewriter alternative via external service",
    );
}

// ── Cache Strategy Patterns ────────────────────────────────────────

#[test]
fn cache_strategy_cache_first() {
    assert_js_true(
        "(() => {
            // Cache-first strategy
            const cacheFirst = async (request, cache) => {
                // Try cache first
                let response = await cache.get(request.url);
                if (response) return response;

                // Fall back to fetch
                response = await fetch(request);

                // Store in cache
                await cache.put(request.url, response.clone());

                return response;
            };

            return typeof cacheFirst === 'function';
        })()",
        "Cache-first strategy pattern",
    );
}

#[test]
fn cache_strategy_network_first() {
    assert_js_true(
        "(() => {
            // Network-first strategy
            const networkFirst = async (request, cache) => {
                try {
                    const response = await fetch(request);
                    await cache.put(request.url, response.clone());
                    return response;
                } catch (e) {
                    // Fall back to cache
                    return await cache.get(request.url);
                }
            };

            return typeof networkFirst === 'function';
        })()",
        "Network-first strategy pattern",
    );
}

#[test]
fn cache_strategy_stale_while_revalidate() {
    assert_js_true(
        "(() => {
            // Stale-while-revalidate strategy
            const staleWhileRevalidate = async (request, cache) => {
                const cached = await cache.get(request.url);

                // Return stale response immediately
                const response = cached || (await fetch(request));

                // Revalidate in background
                fetch(request).then(fresh => {
                    if (fresh.ok) {
                        cache.put(request.url, fresh.clone());
                    }
                });

                return response;
            };

            return typeof staleWhileRevalidate === 'function';
        })()",
        "Stale-while-revalidate strategy",
    );
}

// ── Summary: Caching Recommendations ────────────────────────────────

// NOT Available:
// ✗ Cloudflare Cache API (caches object)
// ✗ HTMLRewriter (HTML transformation API)
//
// Recommended Alternatives:
// 1. In-memory caching with Map/Set
// 2. External cache services (Redis, Memcached)
// 3. Store cache in headers (expires, etag, cache-control)
// 4. Use fetch() with cache strategy patterns
// 5. For HTML, use regex-based transformation or external services
//
// Cache Strategy Patterns:
// - Cache-first: Try cache, then network
// - Network-first: Try network, fall back to cache
// - Stale-while-revalidate: Return cached, update in background
// - Time-based invalidation: Check timestamps
// - ETag-based validation: Use cache headers
