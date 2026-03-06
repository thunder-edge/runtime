//! Ingress router for function traffic.
//!
//! Handles `/{function_name}/*` routes without authentication.
//! Rejects any `/_internal/*` requests to prevent admin access via ingress.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use tracing::info;

use functions::registry::FunctionRegistry;

use crate::body_limits::{
    check_content_length, check_response_body_size, collect_body_with_limit,
    payload_too_large_response, BodyLimitError, BodyLimitsConfig,
};
use crate::middleware::{RateLimitLayer, rate_limit_layer, rate_limited_response};
use crate::router::{is_valid_function_name, json_response, sanitize_internal_error};
use crate::trace_context::{
    add_correlation_id_header, apply_trace_headers, trace_context_from_headers,
};

type BoxBody = Full<Bytes>;

/// Ingress router for function invocation.
///
/// Routes `/{function_name}/*` requests to the appropriate isolate.
/// Rejects `/_internal/*` requests with 404.
#[derive(Clone)]
pub struct IngressRouter {
    registry: Arc<FunctionRegistry>,
    body_limits: BodyLimitsConfig,
    rate_limiter: Option<RateLimitLayer>,
}

impl IngressRouter {
    /// Create a new ingress router.
    pub fn new(
        registry: Arc<FunctionRegistry>,
        body_limits: BodyLimitsConfig,
        rate_limit_rps: Option<u64>,
    ) -> Self {
        Self {
            registry,
            body_limits,
            rate_limiter: rate_limit_rps.map(rate_limit_layer),
        }
    }

    /// Handle an incoming request.
    pub async fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody>, Infallible> {
        let trace_ctx = trace_context_from_headers(req.headers());

        if let Some(limiter) = &self.rate_limiter {
            if let Some(retry_after_secs) = limiter.check_limit() {
                let mut resp = rate_limited_response(retry_after_secs);
                add_correlation_id_header(&mut resp, &trace_ctx.trace_id);
                return Ok(resp);
            }
        }

        let path = req.uri().path();

        // Reject /_internal/* on ingress port
        if path.starts_with("/_internal") {
            let mut resp = json_response(
                StatusCode::NOT_FOUND,
                r#"{"error":"not found"}"#,
            );
            add_correlation_id_header(&mut resp, &trace_ctx.trace_id);
            return Ok(resp);
        }

        let mut resp = self.route_to_function(req, &trace_ctx).await;
        add_correlation_id_header(&mut resp, &trace_ctx.trace_id);
        Ok(resp)
    }

    /// Route request to the appropriate function isolate.
    async fn route_to_function(
        &self,
        req: Request<hyper::body::Incoming>,
        trace_ctx: &crate::trace_context::TraceContext,
    ) -> Response<BoxBody> {
        let path = req.uri().path().to_string();

        // Extract function name from first path segment
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        // segments: ["", "function_name", "rest/of/path"]
        let function_name = if segments.len() >= 2 {
            segments[1]
        } else {
            ""
        };

        if function_name.is_empty() {
            return json_response(
                StatusCode::NOT_FOUND,
                r#"{"error":"no function specified"}"#,
            );
        }

        if !is_valid_function_name(function_name) {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid function name"}"#,
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
            return payload_too_large_response(self.body_limits.max_request_body_bytes);
        }

        // Collect body bytes with size limit
        let (parts, body) = req.into_parts();
        let body_bytes =
            match collect_body_with_limit(body, self.body_limits.max_request_body_bytes).await {
                Ok(bytes) => bytes,
                Err(BodyLimitError::LimitExceeded)
                | Err(BodyLimitError::ContentLengthExceeded { .. }) => {
                    return payload_too_large_response(self.body_limits.max_request_body_bytes);
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

        let response = match tokio::time::timeout(timeout_duration, handle.send_request(forwarded_req)).await {
            Ok(Ok(resp)) => {
                let (parts, body) = resp.into_parts();
                // Check response body size
                if let Some(error_resp) =
                    check_response_body_size(&body, self.body_limits.max_response_body_bytes)
                {
                    return error_resp;
                }
                Response::from_parts(parts, Full::new(body))
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
            sampled = trace_ctx.sampled,
            function_name = %function_name,
            status = %response.status(),
            duration_ms = req_started.elapsed().as_millis() as u64,
            "ingress request completed"
        );

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_path_detection() {
        // Test the path detection logic
        let path = "/_internal/health";
        assert!(path.starts_with("/_internal"));

        let path = "/_internal/functions";
        assert!(path.starts_with("/_internal"));

        let path = "/my-function/hello";
        assert!(!path.starts_with("/_internal"));
    }

    #[test]
    fn test_function_name_extraction() {
        // Test path segment extraction logic
        let path = "/my-func/hello/world";
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], "");
        assert_eq!(segments[1], "my-func");
        assert_eq!(segments[2], "hello/world");
    }

    #[test]
    fn test_function_name_extraction_no_subpath() {
        let path = "/my-func";
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1], "my-func");
    }

    #[test]
    fn test_function_name_extraction_root() {
        let path = "/";
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        let function_name = if segments.len() >= 2 { segments[1] } else { "" };
        assert_eq!(function_name, "");
    }

    #[test]
    fn test_path_rewrite() {
        let path = "/my-func/v1/users/123";
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        let forwarded_path = if segments.len() >= 3 {
            format!("/{}", segments[2])
        } else {
            "/".to_string()
        };
        assert_eq!(forwarded_path, "/v1/users/123");
    }

    #[test]
    fn test_path_rewrite_no_subpath() {
        let path = "/my-func";
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        let forwarded_path = if segments.len() >= 3 {
            format!("/{}", segments[2])
        } else {
            "/".to_string()
        };
        assert_eq!(forwarded_path, "/");
    }

    #[test]
    fn test_reject_invalid_function_name() {
        assert!(!crate::router::is_valid_function_name("Bad_Name"));
        assert!(!crate::router::is_valid_function_name("../admin"));
        assert!(!crate::router::is_valid_function_name(""));
    }

    #[test]
    fn test_rate_limited_response_shape() {
        let resp = crate::middleware::rate_limited_response(1);
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "1");
    }
}
