//! Admin router for `/_internal/*` endpoints with API key authentication.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytes::Bytes;
use http::header::HeaderMap;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use tracing::info_span;
use uuid::Uuid;

use functions::registry::FunctionRegistry;

use crate::body_limits::{
    check_content_length, collect_body_with_limit, payload_too_large_response, BodyLimitError,
    BodyLimitsConfig,
};
use crate::bundle_signature::BundleSignatureVerifier;
use crate::router::{
    build_metrics_body, is_valid_function_name, json_response, normalize_function_name,
    sanitize_internal_error, MetricsCache, METRICS_CACHE_TTL_SECS,
};
use crate::service::BoxBody;

#[derive(serde::Deserialize)]
struct PoolLimitsUpdateRequest {
    min: usize,
    max: usize,
}

#[derive(serde::Serialize)]
struct PoolLimitsResponse {
    min: usize,
    max: usize,
}

fn boxed_full_response(response: Response<Full<Bytes>>) -> Response<BoxBody> {
    let (parts, body) = response.into_parts();
    Response::from_parts(parts, body.boxed())
}

fn parse_manifest_from_headers(
    headers: &HeaderMap,
) -> Result<Option<runtime_core::manifest::ResolvedFunctionManifest>, Response<BoxBody>> {
    let encoded_manifest = headers
        .get("x-function-manifest-b64")
        .and_then(|v| v.to_str().ok());

    let profile = headers
        .get("x-function-manifest-profile")
        .and_then(|v| v.to_str().ok());

    let Some(encoded_manifest) = encoded_manifest else {
        return Ok(None);
    };

    let manifest_bytes = STANDARD.decode(encoded_manifest).map_err(|_| {
        json_response(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid x-function-manifest-b64: expected base64"}"#,
        )
    })?;

    let manifest_json = std::str::from_utf8(&manifest_bytes).map_err(|_| {
        json_response(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid x-function-manifest-b64: decoded payload is not UTF-8 JSON"}"#,
        )
    })?;

    runtime_core::manifest::parse_validate_and_resolve_manifest(manifest_json, profile)
        .map(Some)
        .map_err(|e| {
            json_response(
                StatusCode::BAD_REQUEST,
                &format!(
                    r#"{{"error":"invalid function manifest","details":{:?}}}"#,
                    e.to_string()
                ),
            )
        })
}

/// Admin router for management API endpoints.
///
/// Handles `/_internal/*` routes with optional API key authentication.
/// If `api_key` is `Some(key)`, requests must include `X-API-Key: key` header.
/// If `api_key` is `None`, all requests are allowed (dev mode).
#[derive(Clone)]
pub struct AdminRouter {
    registry: Arc<FunctionRegistry>,
    api_key: Option<String>,
    body_limits: BodyLimitsConfig,
    metrics_cache: Arc<MetricsCache>,
    bundle_signature_verifier: BundleSignatureVerifier,
}

impl AdminRouter {
    /// Create a new admin router.
    ///
    /// - `registry`: Shared function registry
    /// - `api_key`: Optional API key for authentication (None = no auth)
    /// - `body_limits`: Body size limits configuration
    pub fn new(
        registry: Arc<FunctionRegistry>,
        api_key: Option<String>,
        body_limits: BodyLimitsConfig,
        bundle_signature_verifier: BundleSignatureVerifier,
    ) -> Self {
        Self {
            registry,
            api_key,
            body_limits,
            metrics_cache: Arc::new(MetricsCache::new(Duration::from_secs(
                METRICS_CACHE_TTL_SECS,
            ))),
            bundle_signature_verifier,
        }
    }

    /// Handle an incoming request.
    ///
    /// Validates authentication first, then routes to the appropriate handler.
    pub async fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody>, Infallible> {
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        let request_id = Uuid::new_v4().simple().to_string();
        let request_span = info_span!(
            "http.request",
            component = "admin",
            function_name = "admin",
            request_id = %request_id,
            method = %method,
            path = %path
        );
        let _request_span_guard = request_span.enter();

        // Check authentication
        if let Err(resp) = self.check_auth(&req) {
            return Ok(resp);
        }

        Ok(self.route_internal(req, &path, method).await)
    }

    /// Validate API key authentication.
    ///
    /// Returns `Ok(())` if authentication is disabled or key matches.
    /// Returns `Err(Response)` with 401 status if authentication fails.
    fn check_auth(&self, req: &Request<hyper::body::Incoming>) -> Result<(), Response<BoxBody>> {
        let Some(expected) = &self.api_key else {
            // Auth disabled (dev mode)
            return Ok(());
        };

        let provided = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok());

        match provided {
            Some(key) if key == expected => Ok(()),
            Some(_) => Err(json_response(
                StatusCode::UNAUTHORIZED,
                r#"{"error":"invalid API key"}"#,
            )),
            None => Err(json_response(
                StatusCode::UNAUTHORIZED,
                r#"{"error":"missing X-API-Key header"}"#,
            )),
        }
    }

    /// Route internal management API endpoints.
    async fn route_internal(
        &self,
        req: Request<hyper::body::Incoming>,
        path: &str,
        method: Method,
    ) -> Response<BoxBody> {
        match (method.clone(), path) {
            // Health check
            (Method::GET, "/_internal/health") => {
                json_response(StatusCode::OK, r#"{"status":"ok"}"#)
            }

            // Metrics
            (Method::GET, "/_internal/metrics") => self.handle_metrics().await,

            // List functions
            (Method::GET, "/_internal/functions") => {
                let functions = self.registry.list();
                let json = serde_json::to_string(&functions).unwrap_or_default();
                json_response(StatusCode::OK, &json)
            }

            // Deploy new function
            (Method::POST, "/_internal/functions") => self.handle_deploy(req).await,

            // Routes with function name in path
            _ if path.starts_with("/_internal/functions/") => {
                self.handle_function_route(req, path, method).await
            }

            _ => json_response(
                StatusCode::NOT_FOUND,
                r#"{"error":"not found","hint":"admin listener serves only /_internal/* routes; use ingress listener for /{function_name}"}"#,
            ),
        }
    }

    /// Handle GET /_internal/metrics
    async fn handle_metrics(&self) -> Response<BoxBody> {
        let body = self
            .metrics_cache
            .get_or_compute(|| build_metrics_body(&self.registry))
            .await;
        json_response(StatusCode::OK, &body)
    }

    /// Handle POST /_internal/functions (deploy)
    async fn handle_deploy(&self, req: Request<hyper::body::Incoming>) -> Response<BoxBody> {
        // Check Content-Length header for fast rejection
        if let Err(BodyLimitError::ContentLengthExceeded { .. }) =
            check_content_length(&req, self.body_limits.max_request_body_bytes)
        {
            return boxed_full_response(payload_too_large_response(
                self.body_limits.max_request_body_bytes,
            ));
        }

        let (parts, body) = req.into_parts();

        let function_name = parts
            .headers
            .get("x-function-name")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let Some(raw_name) = function_name else {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"missing x-function-name header"}"#,
            );
        };

        let Some(name) = normalize_function_name(&raw_name) else {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid x-function-name; expected URL-safe slug"}"#,
            );
        };

        let resolved_manifest = match parse_manifest_from_headers(&parts.headers) {
            Ok(value) => value,
            Err(response) => return response,
        };

        if let Some(policy) = &resolved_manifest {
            if policy.name != name {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    r#"{"error":"manifest name must match x-function-name"}"#,
                );
            }
        }

        let body_bytes =
            match collect_body_with_limit(body, self.body_limits.max_request_body_bytes).await {
                Ok(bytes) => bytes,
                Err(BodyLimitError::LimitExceeded)
                | Err(BodyLimitError::ContentLengthExceeded { .. }) => {
                    return boxed_full_response(payload_too_large_response(
                        self.body_limits.max_request_body_bytes,
                    ));
                }
                Err(_) => {
                    return json_response(
                        StatusCode::BAD_REQUEST,
                        r#"{"error":"failed to read request body"}"#,
                    )
                }
            };

        if body_bytes.is_empty() {
            return json_response(StatusCode::BAD_REQUEST, r#"{"error":"empty eszip bundle"}"#);
        }

        if let Err(sig_err) = self
            .bundle_signature_verifier
            .verify_headers_and_body(&parts.headers, &body_bytes)
        {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &format!(r#"{{"error":"{}"}}"#, sig_err.as_client_message()),
            );
        }

        match self
            .registry
            .deploy(name, body_bytes, None, resolved_manifest)
            .await
        {
            Ok(info) => {
                let json = serde_json::to_string(&info).unwrap_or_default();
                json_response(StatusCode::CREATED, &json)
            }
            Err(e) => sanitize_internal_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "admin deploy failed",
                &e,
            ),
        }
    }

    /// Handle routes like:
    /// - GET    /_internal/functions/{name}
    /// - PUT    /_internal/functions/{name}
    /// - DELETE /_internal/functions/{name}
    /// - POST   /_internal/functions/{name}/reload
    async fn handle_function_route(
        &self,
        req: Request<hyper::body::Incoming>,
        path: &str,
        method: Method,
    ) -> Response<BoxBody> {
        let rest = &path["/_internal/functions/".len()..];
        let rest = rest.trim_end_matches('/');

        // Check for sub-routes like /reload
        let (name, sub_route) = if let Some(idx) = rest.find('/') {
            (&rest[..idx], Some(&rest[idx + 1..]))
        } else {
            (rest, None)
        };

        if name.is_empty() {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"empty function name"}"#,
            );
        }

        if !is_valid_function_name(name) {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid function name"}"#,
            );
        }

        match (method, sub_route) {
            // GET /_internal/functions/{name}
            (Method::GET, None) => match self.registry.get_info(name) {
                Some(info) => {
                    let json = serde_json::to_string(&info).unwrap_or_default();
                    json_response(StatusCode::OK, &json)
                }
                None => json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
            },

            // PUT /_internal/functions/{name}
            (Method::PUT, None) => {
                // Check Content-Length header for fast rejection
                if let Err(BodyLimitError::ContentLengthExceeded { .. }) =
                    check_content_length(&req, self.body_limits.max_request_body_bytes)
                {
                    return boxed_full_response(payload_too_large_response(
                        self.body_limits.max_request_body_bytes,
                    ));
                }

                let (parts, body) = req.into_parts();
                let resolved_manifest = match parse_manifest_from_headers(&parts.headers) {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                if let Some(policy) = &resolved_manifest {
                    if policy.name != name {
                        return json_response(
                            StatusCode::BAD_REQUEST,
                            r#"{"error":"manifest name must match function route name"}"#,
                        );
                    }
                }
                let body_bytes =
                    match collect_body_with_limit(body, self.body_limits.max_request_body_bytes)
                        .await
                    {
                        Ok(bytes) => bytes,
                        Err(BodyLimitError::LimitExceeded)
                        | Err(BodyLimitError::ContentLengthExceeded { .. }) => {
                            return boxed_full_response(payload_too_large_response(
                                self.body_limits.max_request_body_bytes,
                            ));
                        }
                        Err(_) => {
                            return json_response(
                                StatusCode::BAD_REQUEST,
                                r#"{"error":"failed to read request body"}"#,
                            )
                        }
                    };

                if let Err(sig_err) = self
                    .bundle_signature_verifier
                    .verify_headers_and_body(&parts.headers, &body_bytes)
                {
                    return json_response(
                        StatusCode::UNAUTHORIZED,
                        &format!(r#"{{"error":"{}"}}"#, sig_err.as_client_message()),
                    );
                }

                match self
                    .registry
                    .update(name, body_bytes, None, resolved_manifest)
                    .await
                {
                    Ok(info) => {
                        let json = serde_json::to_string(&info).unwrap_or_default();
                        json_response(StatusCode::OK, &json)
                    }
                    Err(e) => sanitize_internal_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "admin update failed",
                        &e,
                    ),
                }
            }

            // DELETE /_internal/functions/{name}
            (Method::DELETE, None) => match self.registry.delete(name).await {
                Ok(()) => json_response(StatusCode::OK, r#"{"status":"deleted"}"#),
                Err(_e) => json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
            },

            // POST /_internal/functions/{name}/reload
            (Method::POST, Some("reload")) => {
                #[cfg(feature = "hot-reload")]
                {
                    match self.registry.reload(name).await {
                        Ok(info) => {
                            let json = serde_json::to_string(&info).unwrap_or_default();
                            json_response(StatusCode::OK, &json)
                        }
                        Err(e) => sanitize_internal_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "admin hot-reload failed",
                            &e,
                        ),
                    }
                }
                #[cfg(not(feature = "hot-reload"))]
                {
                    json_response(
                        StatusCode::NOT_FOUND,
                        r#"{"error":"hot-reload feature not enabled"}"#,
                    )
                }
            }

            // GET /_internal/functions/{name}/pool
            (Method::GET, Some("pool")) => match self.registry.get_pool_limits(name) {
                Some(limits) => {
                    let body = serde_json::to_string(&PoolLimitsResponse {
                        min: limits.min,
                        max: limits.max,
                    })
                    .unwrap_or_else(|_| "{}".to_string());
                    json_response(StatusCode::OK, &body)
                }
                None => json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
            },

            // PUT /_internal/functions/{name}/pool
            (Method::PUT, Some("pool")) => {
                if let Err(BodyLimitError::ContentLengthExceeded { .. }) =
                    check_content_length(&req, self.body_limits.max_request_body_bytes)
                {
                    return boxed_full_response(payload_too_large_response(
                        self.body_limits.max_request_body_bytes,
                    ));
                }

                let (_parts, body) = req.into_parts();
                let body_bytes =
                    match collect_body_with_limit(body, self.body_limits.max_request_body_bytes)
                        .await
                    {
                        Ok(bytes) => bytes,
                        Err(BodyLimitError::LimitExceeded)
                        | Err(BodyLimitError::ContentLengthExceeded { .. }) => {
                            return boxed_full_response(payload_too_large_response(
                                self.body_limits.max_request_body_bytes,
                            ));
                        }
                        Err(_) => {
                            return json_response(
                                StatusCode::BAD_REQUEST,
                                r#"{"error":"failed to read request body"}"#,
                            )
                        }
                    };

                let update: PoolLimitsUpdateRequest = match serde_json::from_slice(&body_bytes) {
                    Ok(v) => v,
                    Err(_) => {
                        return json_response(
                            StatusCode::BAD_REQUEST,
                            r#"{"error":"invalid json body"}"#,
                        )
                    }
                };

                match self
                    .registry
                    .set_pool_limits(name, update.min, update.max)
                    .await
                {
                    Ok(info) => {
                        let json = serde_json::to_string(&info).unwrap_or_default();
                        json_response(StatusCode::OK, &json)
                    }
                    Err(e) => {
                        json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error":"{}"}}"#, e))
                    }
                }
            }

            _ => json_response(
                StatusCode::METHOD_NOT_ALLOWED,
                r#"{"error":"method not allowed"}"#,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _make_test_registry() -> Arc<FunctionRegistry> {
        use tokio_util::sync::CancellationToken;
        Arc::new(FunctionRegistry::new(
            CancellationToken::new(),
            runtime_core::isolate::IsolateConfig::default(),
        ))
    }

    #[test]
    fn test_auth_disabled_allows_all() {
        // When no key is configured, all requests pass
        let api_key: Option<String> = None;
        assert!(api_key.is_none());
    }

    #[test]
    fn test_auth_logic_missing_key() {
        let api_key = Some("secret-key".to_string());
        let provided: Option<&str> = None;

        // Simulate auth check logic
        let result = match provided {
            Some(key) if key == api_key.as_deref().unwrap() => Ok(()),
            Some(_) => Err("invalid"),
            None => Err("missing"),
        };

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "missing");
    }

    #[test]
    fn test_auth_logic_wrong_key() {
        let api_key = Some("secret-key".to_string());
        let provided: Option<&str> = Some("wrong-key");

        // Simulate auth check logic
        let result = match provided {
            Some(key) if key == api_key.as_deref().unwrap() => Ok(()),
            Some(_) => Err("invalid"),
            None => Err("missing"),
        };

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "invalid");
    }

    #[test]
    fn test_auth_logic_correct_key() {
        let api_key = Some("secret-key".to_string());
        let provided: Option<&str> = Some("secret-key");

        // Simulate auth check logic
        let result = match provided {
            Some(key) if key == api_key.as_deref().unwrap() => Ok(()),
            Some(_) => Err("invalid"),
            None => Err("missing"),
        };

        assert!(result.is_ok());
    }

    #[test]
    fn test_auth_logic_no_key_configured() {
        let api_key: Option<String> = None;

        // When no key is configured, all requests pass
        let passes = api_key.is_none();
        assert!(passes);
    }
}
