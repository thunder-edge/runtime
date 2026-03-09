//! Ingress router for function traffic.
//!
//! Handles `/{function_name}/*` routes without authentication.
//! Rejects any `/_internal/*` requests to prevent admin access via ingress.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, StreamBody};
use runtime_core::isolate::IsolateResponseBody;
use tracing::{info, warn};

use crate::service::BoxBody;
use functions::registry::{FunctionRegistry, RouteTargetError};
use http::header::HOST;

use crate::body_limits::{
    check_content_length, check_response_body_size, collect_body_with_limit,
    payload_too_large_response, BodyLimitError, BodyLimitsConfig,
};
use crate::middleware::{rate_limit_layer, rate_limited_response, RateLimitLayer};
use crate::global_routing::{load_global_routing_table_from_env, GlobalRoutingState, GlobalRoutingTable};
use crate::router::{is_valid_function_name, json_response, sanitize_internal_error};
use crate::trace_context::{
    add_correlation_id_header, apply_trace_headers, trace_context_from_headers,
};

fn boxed_full_response(response: Response<Full<Bytes>>) -> Response<BoxBody> {
    let (parts, body) = response.into_parts();
    Response::from_parts(parts, body.boxed())
}

/// Ingress router for function invocation.
///
/// Routes `/{function_name}/*` requests to the appropriate isolate.
/// Rejects `/_internal/*` requests with 404.
#[derive(Clone)]
pub struct IngressRouter {
    registry: Arc<FunctionRegistry>,
    body_limits: BodyLimitsConfig,
    rate_limiter: Option<RateLimitLayer>,
    global_routing: GlobalRoutingState,
}

impl IngressRouter {
    /// Create a new ingress router.
    pub fn new(
        registry: Arc<FunctionRegistry>,
        body_limits: BodyLimitsConfig,
        rate_limit_rps: Option<u64>,
    ) -> Self {
        Self::new_with_global_routing(
            registry,
            body_limits,
            rate_limit_rps,
            load_global_routing_table_from_env(),
        )
    }

    pub fn new_with_global_routing(
        registry: Arc<FunctionRegistry>,
        body_limits: BodyLimitsConfig,
        rate_limit_rps: Option<u64>,
        global_routing: Option<GlobalRoutingTable>,
    ) -> Self {
        Self::new_with_global_routing_state(
            registry,
            body_limits,
            rate_limit_rps,
            GlobalRoutingState::new(global_routing),
        )
    }

    pub fn new_with_global_routing_state(
        registry: Arc<FunctionRegistry>,
        body_limits: BodyLimitsConfig,
        rate_limit_rps: Option<u64>,
        global_routing: GlobalRoutingState,
    ) -> Self {
        Self {
            registry,
            body_limits,
            rate_limiter: rate_limit_rps.map(rate_limit_layer),
            global_routing,
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
                let mut resp = boxed_full_response(rate_limited_response(retry_after_secs));
                add_correlation_id_header(&mut resp, &trace_ctx.trace_id);
                return Ok(resp);
            }
        }

        let path = req.uri().path();

        // Reject /_internal/* on ingress port
        if path.starts_with("/_internal") {
            let mut resp = json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#);
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
        let host = req
            .headers()
            .get(HOST)
            .and_then(|value| value.to_str().ok());

        let (function_name, forwarded_path) =
            match self.resolve_route_target(path.as_str(), host) {
                Ok(value) => value,
                Err(response) => return response,
            };

        // Resolve isolate + logical context target
        let route_target = match self
            .registry
            .get_route_target_with_status(function_name.as_str())
            .await
        {
            Ok(target) => target,
            Err(RouteTargetError::FunctionUnavailable) => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &format!(
                        r#"{{"error":"function '{}' not found or not running"}}"#,
                        function_name
                    ),
                )
            }
            Err(RouteTargetError::CapacityExhausted) => {
                return json_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    r#"{"error":"capacity exhausted"}"#,
                )
            }
        };

        // Get function config for timeouts
        let config = self
            .registry
            .get_config(function_name.as_str())
            .unwrap_or_default();

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

        let forwarded_method = parts.method.clone();
        let forwarded_headers = parts.headers.clone();
        let forwarded_body = body_bytes.clone();

        // Send to isolate with timeout
        let timeout_duration = if config.wall_clock_timeout_ms > 0 {
            std::time::Duration::from_millis(config.wall_clock_timeout_ms)
        } else {
            std::time::Duration::from_secs(60) // default 60s
        };

        let req_started = Instant::now();

        let mut route_target = route_target;
        let mut route_result;
        let mut attempt = 0_u8;
        loop {
            attempt = attempt.saturating_add(1);

            let mut forwarded_req = http::Request::builder()
                .method(forwarded_method.clone())
                .uri(&forwarded_path)
                .body(forwarded_body.clone())
                .unwrap();
            *forwarded_req.headers_mut() = forwarded_headers.clone();
            apply_trace_headers(forwarded_req.headers_mut(), trace_ctx);

            route_result = tokio::time::timeout(
                timeout_duration,
                route_target.handle.send_routed_request(
                    forwarded_req,
                    Some(function_name.clone()),
                    Some(route_target.context_id.clone()),
                ),
            )
            .await;
            self.registry.release_route_target(&route_target);

            let should_retry = matches!(&route_result, Ok(Err(err)) if attempt == 1 && err.to_string().contains("channel closed"));
            if !should_retry {
                break;
            }

            warn!(
                function_name = %function_name,
                request_id = %trace_ctx.trace_id,
                "transient channel-closed while routing request; retrying once"
            );

            route_target = match self
                .registry
                .get_route_target_with_status(function_name.as_str())
                .await
            {
                Ok(target) => target,
                Err(RouteTargetError::FunctionUnavailable) => {
                    return json_response(
                        StatusCode::NOT_FOUND,
                        &format!(
                            r#"{{"error":"function '{}' not found or not running"}}"#,
                            function_name
                        ),
                    )
                }
                Err(RouteTargetError::CapacityExhausted) => {
                    return json_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        r#"{"error":"capacity exhausted"}"#,
                    )
                }
            };
        }

        let response = match route_result {
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
                                        tracing::error!(
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

    fn resolve_route_target(
        &self,
        path: &str,
        host: Option<&str>,
    ) -> Result<(String, String), Response<BoxBody>> {
        if let Some(table) = self.global_routing.get() {
            if let Some(matched) = table.resolve(host, path) {
                return Ok((matched.target_function, path.to_string()));
            }
        }

        // Fallback to canonical /{function_name}/... path-prefix routing.
        let segments: Vec<&str> = path.splitn(3, '/').collect();
        let function_name = if segments.len() >= 2 { segments[1] } else { "" };

        if function_name.is_empty() {
            return Err(json_response(
                StatusCode::NOT_FOUND,
                r#"{"error":"no function specified"}"#,
            ));
        }

        if !is_valid_function_name(function_name) {
            return Err(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid function name"}"#,
            ));
        }

        let forwarded_path = if segments.len() >= 3 {
            format!("/{}", segments[2])
        } else {
            "/".to_string()
        };

        Ok((function_name.to_string(), forwarded_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_routing::GlobalRoutingTable;
    use functions::registry::FunctionRegistry;
    use runtime_core::isolate::IsolateConfig;
    use tokio_util::sync::CancellationToken;

    fn test_registry() -> Arc<FunctionRegistry> {
        Arc::new(FunctionRegistry::new(
            CancellationToken::new(),
            IsolateConfig::default(),
        ))
    }

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

    #[test]
    fn test_resolve_route_target_uses_global_stage0_when_matched() {
        let manifest = r#"{
            "manifestVersion": 1,
            "routes": [
                {
                    "host": "api.example.com",
                    "path": "/users/:id",
                    "targetFunction": "users-api"
                }
            ]
        }"#;
        let table = GlobalRoutingTable::from_manifest_json(manifest, "test")
            .expect("global routing manifest should parse");

        let router = IngressRouter::new_with_global_routing(
            test_registry(),
            BodyLimitsConfig::default(),
            None,
            Some(table),
        );

        let resolved = router
            .resolve_route_target("/users/123", Some("api.example.com"))
            .expect("stage0 should resolve route");

        assert_eq!(resolved.0, "users-api");
        assert_eq!(resolved.1, "/users/123");
    }

    #[test]
    fn test_resolve_route_target_falls_back_to_prefix_mode() {
        let router = IngressRouter::new_with_global_routing(
            test_registry(),
            BodyLimitsConfig::default(),
            None,
            None,
        );

        let resolved = router
            .resolve_route_target("/my-function/v1/ping", Some("api.example.com"))
            .expect("fallback routing should resolve");

        assert_eq!(resolved.0, "my-function");
        assert_eq!(resolved.1, "/v1/ping");
    }
}
