use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, StreamBody};
use runtime_core::isolate::IsolateResponseBody;
use tokio::sync::RwLock;
use tracing::{error, info};
use uuid::Uuid;

use crate::service::BoxBody;
use functions::registry::FunctionRegistry;

use crate::body_limits::{
    check_content_length, check_response_body_size, collect_body_with_limit,
    payload_too_large_response, BodyLimitError, BodyLimitsConfig,
};
use crate::middleware::{rate_limit_layer, rate_limited_response, RateLimitLayer};
use crate::trace_context::{
    add_correlation_id_header, apply_trace_headers, trace_context_from_headers,
};

const MAX_LOG_ERROR_BYTES: usize = 1024;
const MAX_FUNCTION_NAME_LEN: usize = 63;
pub const METRICS_CACHE_TTL_SECS: u64 = 15;

#[derive(Debug, Clone, Copy)]
pub enum ClientError {
    InternalError,
}

impl ClientError {
    fn as_code(self) -> &'static str {
        match self {
            ClientError::InternalError => "internal_error",
        }
    }
}

#[derive(Clone, Debug)]
struct CachedMetrics {
    body: String,
    cached_at: Instant,
}

/// In-memory cache for the expensive metrics endpoint computation.
#[derive(Debug)]
pub struct MetricsCache {
    ttl: Duration,
    entry: RwLock<Option<CachedMetrics>>,
}

impl MetricsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entry: RwLock::new(None),
        }
    }

    pub async fn get_or_compute<F>(&self, build_body: F) -> String
    where
        F: FnOnce() -> String,
    {
        {
            let read = self.entry.read().await;
            if let Some(cached) = &*read {
                if cached.cached_at.elapsed() < self.ttl {
                    return cached.body.clone();
                }
            }
        }

        let body = build_body();
        let mut write = self.entry.write().await;
        *write = Some(CachedMetrics {
            body: body.clone(),
            cached_at: Instant::now(),
        });
        body
    }
}

fn truncate_for_log(message: &str, max_bytes: usize) -> String {
    if message.len() <= max_bytes {
        return message.to_string();
    }

    if max_bytes == 0 {
        return String::new();
    }

    let suffix = "... [truncated]";
    if max_bytes <= suffix.len() {
        return suffix[..max_bytes].to_string();
    }

    let mut cut = max_bytes - suffix.len();
    while cut > 0 && !message.is_char_boundary(cut) {
        cut -= 1;
    }

    format!("{}{}", &message[..cut], suffix)
}

fn log_truncated_error(context: &str, err: &impl std::fmt::Display) {
    let truncated = truncate_for_log(&err.to_string(), MAX_LOG_ERROR_BYTES);
    error!(
        function_name = "runtime",
        request_id = "system",
        "{}: {}",
        context,
        truncated
    );
}

fn boxed_full_response(response: Response<Full<Bytes>>) -> Response<BoxBody> {
    let (parts, body) = response.into_parts();
    Response::from_parts(parts, body.boxed())
}

pub fn sanitize_internal_error<E>(status: StatusCode, context: &str, err: &E) -> Response<BoxBody>
where
    E: std::fmt::Display + std::fmt::Debug,
{
    let request_id = Uuid::new_v4().to_string();
    error!(
        function_name = "runtime",
        request_id = %request_id,
        error = ?err,
        "{}",
        context
    );
    client_error_response(status, ClientError::InternalError, &request_id)
}

fn parse_manifest_from_headers(
    headers: &http::header::HeaderMap,
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

pub fn client_error_response(
    status: StatusCode,
    client_error: ClientError,
    request_id: &str,
) -> Response<BoxBody> {
    let body = client_error_json(client_error, request_id);
    json_response(status, &body)
}

pub fn client_error_json(client_error: ClientError, request_id: &str) -> String {
    serde_json::json!({
        "error": client_error.as_code(),
        "request_id": request_id,
    })
    .to_string()
}

/// The top-level HTTP router.
///
/// Splits traffic between:
/// - `/_internal/*` → management API
/// - `/{function_name}/*` → ingress to function isolates
#[derive(Clone)]
pub struct Router {
    registry: Arc<FunctionRegistry>,
    body_limits: BodyLimitsConfig,
    rate_limiter: Option<RateLimitLayer>,
    metrics_cache: Arc<MetricsCache>,
}

impl Router {
    pub fn new(
        registry: Arc<FunctionRegistry>,
        body_limits: BodyLimitsConfig,
        rate_limit_rps: Option<u64>,
    ) -> Self {
        Self {
            registry,
            body_limits,
            rate_limiter: rate_limit_rps.map(rate_limit_layer),
            metrics_cache: Arc::new(MetricsCache::new(Duration::from_secs(
                METRICS_CACHE_TTL_SECS,
            ))),
        }
    }

    /// Handle an incoming request.
    pub async fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody>, Infallible> {
        let path = req.uri().path().to_string();
        let trace_ctx = trace_context_from_headers(req.headers());

        let mut resp = if path.starts_with("/_internal/") || path == "/_internal" {
            self.handle_internal(req).await
        } else {
            self.handle_ingress(req, &trace_ctx).await
        };

        add_correlation_id_header(&mut resp, &trace_ctx.trace_id);
        Ok(resp)
    }

    /// Route ingress traffic: /{function_name}/rest/of/path
    async fn handle_ingress(
        &self,
        req: Request<hyper::body::Incoming>,
        trace_ctx: &crate::trace_context::TraceContext,
    ) -> Response<BoxBody> {
        if let Some(limiter) = &self.rate_limiter {
            if let Some(retry_after_secs) = limiter.check_limit() {
                return boxed_full_response(rate_limited_response(retry_after_secs));
            }
        }

        let path = req.uri().path().to_string();

        // Extract function name from first path segment
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        // segments: ["", "function_name", "rest/of/path"]
        let function_name = if segments.len() >= 2 { segments[1] } else { "" };

        if function_name.is_empty() {
            return json_response(
                StatusCode::NOT_FOUND,
                r#"{"error":"no function specified"}"#,
            );
        }

        if !is_valid_function_name(function_name) {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid function name; use lowercase slug [a-z0-9-], max 63 chars"}"#,
            );
        }

        // Get isolate handle
        let Some(handle) = self.registry.get_handle(function_name) else {
            return json_response(
                StatusCode::NOT_FOUND,
                &format!(
                    r#"{{"error":"function '{}' not found or not running"}}"#,
                    function_name
                ),
            );
        };

        // Get function config for timeouts
        let config = self.registry.get_config(function_name).unwrap_or_default();

        // Rewrite path: strip the function_name prefix
        let forwarded_path = if segments.len() >= 3 {
            format!("/{}", segments[2])
        } else {
            "/".to_string()
        };

        // Check Content-Length header for fast rejection
        if let Err(BodyLimitError::ContentLengthExceeded { .. }) =
            check_content_length(&req, self.body_limits.max_request_body_bytes)
        {
            return boxed_full_response(payload_too_large_response(
                self.body_limits.max_request_body_bytes,
            ));
        }

        // Collect body bytes with size limit
        let (parts, body) = req.into_parts();
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

        // Build forwarded request
        let mut forwarded_req = http::Request::builder()
            .method(parts.method)
            .uri(&forwarded_path)
            .body(body_bytes)
            .unwrap();
        *forwarded_req.headers_mut() = parts.headers;

        // Send to isolate with timeout
        let timeout_duration = if config.wall_clock_timeout_ms > 0 {
            std::time::Duration::from_millis(config.wall_clock_timeout_ms)
        } else {
            std::time::Duration::from_secs(60) // default 60s
        };

        let req_started = Instant::now();
        apply_trace_headers(forwarded_req.headers_mut(), trace_ctx);

        let response = match tokio::time::timeout(
            timeout_duration,
            handle.send_request(forwarded_req),
        )
        .await
        {
            Ok(Ok(resp)) => {
                let (parts, body) = (resp.parts, resp.body);
                match body {
                    IsolateResponseBody::Full(bytes) => {
                        if let Some(error_resp) = check_response_body_size(
                            &bytes,
                            self.body_limits.max_response_body_bytes,
                        ) {
                            return boxed_full_response(error_resp);
                        }
                        Response::from_parts(parts, Full::new(bytes).boxed())
                    }
                    IsolateResponseBody::Stream(receiver) => {
                        let log_function_name = function_name.to_string();
                        let log_request_id = trace_ctx.trace_id.clone();
                        let stream = futures_util::stream::unfold(receiver, move |mut rx| {
                            let log_function_name = log_function_name.clone();
                            let log_request_id = log_request_id.clone();
                            async move {
                                match rx.recv().await {
                                    Some(Ok(chunk)) => {
                                        Some((Ok(http_body::Frame::data(chunk)), rx))
                                    }
                                    Some(Err(err)) => {
                                        error!(
                                            function_name = %log_function_name,
                                            request_id = %log_request_id,
                                            "streaming response chunk failed: {}",
                                            err
                                        );
                                        None
                                    }
                                    None => None,
                                }
                            }
                        });
                        Response::from_parts(parts, StreamBody::new(stream).boxed())
                    }
                }
            }
            Ok(Err(e)) => sanitize_internal_error(
                StatusCode::BAD_GATEWAY,
                "failed to handle ingress request in isolate",
                &e,
            ),
            Err(_) => json_response(
                StatusCode::GATEWAY_TIMEOUT,
                r#"{"error":"request timeout"}"#,
            ),
        };

        info!(
            trace_id = %trace_ctx.trace_id,
            request_id = %trace_ctx.trace_id,
            sampled = trace_ctx.sampled,
            function_name = %function_name,
            status = %response.status(),
            duration_ms = req_started.elapsed().as_millis() as u64,
            "ingress request completed"
        );

        response
    }

    /// Route internal management API.
    async fn handle_internal(&self, req: Request<hyper::body::Incoming>) -> Response<BoxBody> {
        let path = req.uri().path().to_string();
        let method = req.method().clone();

        match (method.clone(), path.as_str()) {
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
                self.handle_function_route(req, &path, method).await
            }

            _ => json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
        }
    }

    async fn handle_metrics(&self) -> Response<BoxBody> {
        let body = self
            .metrics_cache
            .get_or_compute(|| build_metrics_body(&self.registry))
            .await;
        json_response(StatusCode::OK, &body)
    }

    /// Deploy a new function: POST /_internal/functions
    ///
    /// Expects multipart or raw body with:
    /// - Header `x-function-name`: the function name
    /// - Body: the eszip bundle bytes
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

        match self
            .registry
            .deploy(name, body_bytes, None, resolved_manifest)
            .await
        {
            Ok(info) => {
                let json = serde_json::to_string(&info).unwrap_or_default();
                json_response(StatusCode::CREATED, &json)
            }
            Err(e) => {
                log_truncated_error("failed to deploy function", &e);
                sanitize_internal_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to deploy function",
                    &e,
                )
            }
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

                match self
                    .registry
                    .update(name, body_bytes, None, resolved_manifest)
                    .await
                {
                    Ok(info) => {
                        let json = serde_json::to_string(&info).unwrap_or_default();
                        json_response(StatusCode::OK, &json)
                    }
                    Err(e) => {
                        log_truncated_error("failed to update function", &e);
                        sanitize_internal_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "failed to update function",
                            &e,
                        )
                    }
                }
            }

            // DELETE /_internal/functions/{name}
            (Method::DELETE, None) => match self.registry.delete(name).await {
                Ok(()) => json_response(StatusCode::OK, r#"{"status":"deleted"}"#),
                Err(e) => {
                    log_truncated_error("failed to delete function", &e);
                    json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#)
                }
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
                        Err(e) => {
                            log_truncated_error("failed to hot-reload function", &e);
                            sanitize_internal_error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "failed to hot-reload function",
                                &e,
                            )
                        }
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

            _ => json_response(
                StatusCode::METHOD_NOT_ALLOWED,
                r#"{"error":"method not allowed"}"#,
            ),
        }
    }
}

/// Extract the function name and forwarded path from a URL path.
pub fn extract_function_and_path(path: &str) -> (&str, String) {
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    let function_name = if segments.len() >= 2 { segments[1] } else { "" };
    let forwarded_path = if segments.len() >= 3 {
        format!("/{}", segments[2])
    } else {
        "/".to_string()
    };
    (function_name, forwarded_path)
}

pub fn build_metrics_body(registry: &FunctionRegistry) -> String {
    let functions = registry.list();
    let total_requests: u64 = functions.iter().map(|f| f.metrics.total_requests).sum();
    let total_errors: u64 = functions.iter().map(|f| f.metrics.total_errors).sum();
    let total_cold_starts: u64 = functions.iter().map(|f| f.metrics.cold_starts).sum();
    let total_cold_start_ms: u64 = functions
        .iter()
        .map(|f| f.metrics.total_cold_start_time_ms)
        .sum();
    let total_warm_start_ms: u64 = functions
        .iter()
        .map(|f| f.metrics.total_warm_start_time_ms)
        .sum();

    let avg_cold_start_ms = if total_cold_starts > 0 {
        total_cold_start_ms / total_cold_starts
    } else {
        0
    };

    let avg_warm_start_ms = if total_requests > 0 {
        total_warm_start_ms / total_requests
    } else {
        0
    };

    // This syscall-heavy section is why caching is needed.
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes();
    let current_pid = sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from(0));
    let process_memory_mb = sys
        .process(current_pid)
        .map(|p| p.memory() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);

    let function_count = registry.count();
    let estimated_memory_per_function_mb = if function_count > 0 {
        process_memory_mb / (function_count as f64)
    } else {
        0.0
    };

    let body = serde_json::json!({
        "function_count": function_count,
        "total_requests": total_requests,
        "total_errors": total_errors,
        "total_cold_starts": total_cold_starts,
        "avg_cold_start_ms": avg_cold_start_ms,
        "avg_warm_start_ms": avg_warm_start_ms,
        "memory": {
            "process_memory_mb": process_memory_mb,
            "estimated_per_function_mb": estimated_memory_per_function_mb
        },
        "functions": functions,
    });

    body.to_string()
}

/// Validate canonical function name slug format: `^[a-z0-9][a-z0-9-]{0,62}$`.
pub fn is_valid_function_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_FUNCTION_NAME_LEN {
        return false;
    }

    let bytes = name.as_bytes();
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }

    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Slugify a raw function name into URL-safe lowercase form.
///
/// Rules:
/// - lowercase ASCII
/// - keep `[a-z0-9]`
/// - map all separators/punctuation to `-`
/// - collapse repeated dashes and trim leading/trailing dashes
/// - truncate to 63 chars, preserving valid boundaries
pub fn slugify_function_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_FUNCTION_NAME_LEN));
    let mut last_was_dash = false;

    for ch in raw.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }

        if out.len() >= MAX_FUNCTION_NAME_LEN {
            break;
        }
    }

    let trimmed = out.trim_matches('-').to_string();
    if trimmed.len() <= MAX_FUNCTION_NAME_LEN {
        trimmed
    } else {
        trimmed[..MAX_FUNCTION_NAME_LEN]
            .trim_matches('-')
            .to_string()
    }
}

/// Convert user-provided name to canonical slug, returning None if it cannot
/// produce a valid function name.
pub fn normalize_function_name(raw: &str) -> Option<String> {
    let slug = slugify_function_name(raw);
    if is_valid_function_name(&slug) {
        Some(slug)
    } else {
        None
    }
}

/// Build a JSON response.
pub fn json_response(status: StatusCode, body: &str) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())).boxed())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn extract_simple() {
        let (name, path) = extract_function_and_path("/my-func/hello/world");
        assert_eq!(name, "my-func");
        assert_eq!(path, "/hello/world");
    }

    #[test]
    fn extract_no_sub_path() {
        let (name, path) = extract_function_and_path("/my-func");
        assert_eq!(name, "my-func");
        assert_eq!(path, "/");
    }

    #[test]
    fn extract_root() {
        let (name, path) = extract_function_and_path("/");
        assert_eq!(name, "");
        assert_eq!(path, "/");
    }

    #[test]
    fn extract_deep() {
        let (name, path) = extract_function_and_path("/api/v1/users/123");
        assert_eq!(name, "api");
        assert_eq!(path, "/v1/users/123");
    }

    #[test]
    fn json_response_content_type() {
        let resp = json_response(StatusCode::OK, r#"{"ok":true}"#);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn json_response_status() {
        let resp = json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn sanitized_internal_error_contains_request_id() {
        let body = client_error_json(ClientError::InternalError, "req-123");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "internal_error");
        assert_eq!(parsed["request_id"], "req-123");
    }

    #[test]
    fn truncate_for_log_keeps_short_message() {
        let msg = "short error";
        assert_eq!(truncate_for_log(msg, 1024), msg);
    }

    #[test]
    fn truncate_for_log_limits_to_1kib() {
        let msg = "x".repeat(2000);
        let truncated = truncate_for_log(&msg, 1024);
        assert!(truncated.len() <= 1024);
        assert!(truncated.ends_with("... [truncated]"));
    }

    #[test]
    fn function_name_validation_accepts_slug() {
        assert!(is_valid_function_name("my-function-01"));
    }

    #[test]
    fn function_name_validation_rejects_invalid() {
        assert!(!is_valid_function_name(""));
        assert!(!is_valid_function_name("UpperCase"));
        assert!(!is_valid_function_name("name..dots"));
        assert!(!is_valid_function_name("with/slash"));
        assert!(!is_valid_function_name("função"));
        let too_long = "a".repeat(64);
        assert!(!is_valid_function_name(&too_long));
    }

    #[test]
    fn slugify_normalizes_to_url_safe_slug() {
        assert_eq!(slugify_function_name(" My Func_v2 "), "my-func-v2");
        assert_eq!(
            slugify_function_name("api..gateway///edge"),
            "api-gateway-edge"
        );
        assert_eq!(
            normalize_function_name("___hello___"),
            Some("hello".to_string())
        );
    }

    #[tokio::test]
    async fn metrics_cache_reuses_until_ttl_then_refreshes() {
        let cache = MetricsCache::new(Duration::from_millis(30));
        let calls = Arc::new(AtomicUsize::new(0));

        let c1 = calls.clone();
        let first = cache
            .get_or_compute(move || {
                let n = c1.fetch_add(1, Ordering::SeqCst) + 1;
                format!("payload-{n}")
            })
            .await;
        assert_eq!(first, "payload-1");

        let c2 = calls.clone();
        let second = cache
            .get_or_compute(move || {
                let n = c2.fetch_add(1, Ordering::SeqCst) + 1;
                format!("payload-{n}")
            })
            .await;
        assert_eq!(second, "payload-1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        tokio::time::sleep(Duration::from_millis(40)).await;

        let c3 = calls.clone();
        let third = cache
            .get_or_compute(move || {
                let n = c3.fetch_add(1, Ordering::SeqCst) + 1;
                format!("payload-{n}")
            })
            .await;
        assert_eq!(third, "payload-2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
