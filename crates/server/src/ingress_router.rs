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
    payload_too_large_response, prime_response_stream, response_body_too_large_response,
    BodyLimitError, BodyLimitsConfig, ResponseStreamPrimeError,
};
use crate::function_route_matcher::{is_asset_route, match_suffix_route, RouteMatchDecision};
use crate::global_routing::{
    load_global_routing_table_from_env, GlobalRoutingState, GlobalRoutingTable,
};
use crate::middleware::{rate_limit_layer, rate_limited_response, RateLimitLayer};
use crate::router::{
    build_limited_stream, is_valid_function_name, json_response, sanitize_internal_error,
    RouteTargetLease,
};
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

        if let Some(route_metadata) = self.registry.get_route_metadata(function_name.as_str()) {
            match match_suffix_route(&route_metadata, &forwarded_path, req.method()) {
                RouteMatchDecision::NotFound => {
                    return json_response(StatusCode::NOT_FOUND, r#"{"error":"route not found"}"#)
                }
                RouteMatchDecision::MethodNotAllowed { allow } => {
                    let mut resp = json_response(
                        StatusCode::METHOD_NOT_ALLOWED,
                        r#"{"error":"method not allowed"}"#,
                    );
                    if !allow.is_empty() {
                        let value = allow.join(", ");
                        if let Ok(header) = http::header::HeaderValue::from_str(&value) {
                            resp.headers_mut().insert(http::header::ALLOW, header);
                        }
                    }
                    return resp;
                }
                RouteMatchDecision::Matched(route) => {
                    if is_asset_route(&route) {
                        return json_response(
                            StatusCode::NOT_FOUND,
                            r#"{"error":"asset route not found"}"#,
                        );
                    }
                }
            }
        }

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

        // Resolve isolate + logical context target only after the request body is valid.
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

            let should_retry = matches!(&route_result, Ok(Err(err)) if attempt == 1 && err.to_string().contains("channel closed"));
            if !should_retry {
                break;
            }

            self.registry.release_route_target(&route_target);

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

        let route_target_lease = RouteTargetLease::new(self.registry.clone(), route_target);
        let response = match route_result {
            Ok(Ok(resp)) => {
                let (mut parts, body) = (resp.parts, resp.body);
                match body {
                    IsolateResponseBody::Full(bytes) => {
                        drop(route_target_lease);
                        if let Some(error_resp) = check_response_body_size(
                            &bytes,
                            self.body_limits.max_response_body_bytes,
                        ) {
                            return boxed_full_response(error_resp);
                        }
                        Response::from_parts(parts, Full::new(bytes).boxed())
                    }
                    IsolateResponseBody::Stream(receiver) => {
                        let primed = match prime_response_stream(
                            receiver,
                            self.body_limits.max_response_body_bytes,
                        )
                        .await
                        {
                            Ok(primed) => primed,
                            Err(ResponseStreamPrimeError::LimitExceeded) => {
                                drop(route_target_lease);
                                return boxed_full_response(response_body_too_large_response(
                                    self.body_limits.max_response_body_bytes,
                                ));
                            }
                            Err(ResponseStreamPrimeError::Producer(err)) => {
                                drop(route_target_lease);
                                return sanitize_internal_error(
                                    StatusCode::BAD_GATEWAY,
                                    "streaming response failed before headers were sent",
                                    &err,
                                );
                            }
                        };
                        parts.headers.remove(http::header::CONTENT_LENGTH);
                        let stream = build_limited_stream(
                            primed,
                            route_target_lease,
                            function_name.to_string(),
                            trace_ctx.trace_id.clone(),
                        );
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
#[path = "ingress_router_test.rs"]
mod tests;
