use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};

use functions::registry::FunctionRegistry;

type BoxBody = Full<Bytes>;

/// The top-level HTTP router.
///
/// Splits traffic between:
/// - `/_internal/*` → management API
/// - `/{function_name}/*` → ingress to function isolates
#[derive(Clone)]
pub struct Router {
    registry: Arc<FunctionRegistry>,
}

impl Router {
    pub fn new(registry: Arc<FunctionRegistry>) -> Self {
        Self { registry }
    }

    /// Handle an incoming request.
    pub async fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<BoxBody>, Infallible> {
        let path = req.uri().path().to_string();

        if path.starts_with("/_internal/") || path == "/_internal" {
            Ok(self.handle_internal(req).await)
        } else {
            Ok(self.handle_ingress(req).await)
        }
    }

    /// Route ingress traffic: /{function_name}/rest/of/path
    async fn handle_ingress(
        &self,
        req: Request<hyper::body::Incoming>,
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

        // Collect body bytes
        let (parts, body) = req.into_parts();
        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
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

        match tokio::time::timeout(timeout_duration, handle.send_request(forwarded_req)).await {
            Ok(Ok(resp)) => {
                let (parts, body) = resp.into_parts();
                Response::from_parts(parts, Full::new(body))
            }
            Ok(Err(e)) => json_response(
                StatusCode::BAD_GATEWAY,
                &format!(r#"{{"error":"isolate error: {}"}}"#, e),
            ),
            Err(_) => json_response(
                StatusCode::GATEWAY_TIMEOUT,
                r#"{"error":"request timeout"}"#,
            ),
        }
    }

    /// Route internal management API.
    async fn handle_internal(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Response<BoxBody> {
        let path = req.uri().path().to_string();
        let method = req.method().clone();

        match (method.clone(), path.as_str()) {
            // Health check
            (Method::GET, "/_internal/health") => {
                json_response(StatusCode::OK, r#"{"status":"ok"}"#)
            }

            // Metrics
            (Method::GET, "/_internal/metrics") => {
                let functions = self.registry.list();
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

                // Get process memory info
                let mut sys = sysinfo::System::new_all();
                sys.refresh_processes();
                let current_pid = sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from(0));
                let process_memory_mb = sys
                    .process(current_pid)
                    .map(|p| p.memory() / 1024) // Convert from KB to MB
                    .unwrap_or(0);

                // Estimate memory per function (simple division)
                let function_count = self.registry.count();
                let estimated_memory_per_function_mb = if function_count > 0 {
                    process_memory_mb / (function_count as u64)
                } else {
                    0
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
                json_response(StatusCode::OK, &body.to_string())
            }

            // List functions
            (Method::GET, "/_internal/functions") => {
                let functions = self.registry.list();
                let json = serde_json::to_string(&functions).unwrap_or_default();
                json_response(StatusCode::OK, &json)
            }

            // Deploy new function
            (Method::POST, "/_internal/functions") => {
                self.handle_deploy(req).await
            }

            // Routes with function name in path
            _ if path.starts_with("/_internal/functions/") => {
                self.handle_function_route(req, &path, method).await
            }

            _ => json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
        }
    }

    /// Deploy a new function: POST /_internal/functions
    ///
    /// Expects multipart or raw body with:
    /// - Header `x-function-name`: the function name
    /// - Body: the eszip bundle bytes
    async fn handle_deploy(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Response<BoxBody> {
        let (parts, body) = req.into_parts();

        let function_name = parts
            .headers
            .get("x-function-name")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let Some(name) = function_name else {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"missing x-function-name header"}"#,
            );
        };

        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    r#"{"error":"failed to read request body"}"#,
                )
            }
        };

        if body_bytes.is_empty() {
            return json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"empty eszip bundle"}"#,
            );
        }

        match self.registry.deploy(name, body_bytes, None).await {
            Ok(info) => {
                let json = serde_json::to_string(&info).unwrap_or_default();
                json_response(StatusCode::CREATED, &json)
            }
            Err(e) => json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!(r#"{{"error":"{}"}}"#, e),
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
            return json_response(StatusCode::BAD_REQUEST, r#"{"error":"empty function name"}"#);
        }

        match (method, sub_route) {
            // GET /_internal/functions/{name}
            (Method::GET, None) => {
                match self.registry.get_info(name) {
                    Some(info) => {
                        let json = serde_json::to_string(&info).unwrap_or_default();
                        json_response(StatusCode::OK, &json)
                    }
                    None => json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#),
                }
            }

            // PUT /_internal/functions/{name}
            (Method::PUT, None) => {
                let (_, body) = req.into_parts();
                let body_bytes = match body.collect().await {
                    Ok(collected) => collected.to_bytes(),
                    Err(_) => {
                        return json_response(
                            StatusCode::BAD_REQUEST,
                            r#"{"error":"failed to read request body"}"#,
                        )
                    }
                };

                match self.registry.update(name, body_bytes, None).await {
                    Ok(info) => {
                        let json = serde_json::to_string(&info).unwrap_or_default();
                        json_response(StatusCode::OK, &json)
                    }
                    Err(e) => json_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!(r#"{{"error":"{}"}}"#, e),
                    ),
                }
            }

            // DELETE /_internal/functions/{name}
            (Method::DELETE, None) => {
                match self.registry.delete(name).await {
                    Ok(()) => json_response(StatusCode::OK, r#"{"status":"deleted"}"#),
                    Err(e) => json_response(
                        StatusCode::NOT_FOUND,
                        &format!(r#"{{"error":"{}"}}"#, e),
                    ),
                }
            }

            // POST /_internal/functions/{name}/reload
            (Method::POST, Some("reload")) => {
                #[cfg(feature = "hot-reload")]
                {
                    match self.registry.reload(name).await {
                        Ok(info) => {
                            let json = serde_json::to_string(&info).unwrap_or_default();
                            json_response(StatusCode::OK, &json)
                        }
                        Err(e) => json_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!(r#"{{"error":"{}"}}"#, e),
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

            _ => json_response(StatusCode::METHOD_NOT_ALLOWED, r#"{"error":"method not allowed"}"#),
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

/// Build a JSON response.
pub fn json_response(status: StatusCode, body: &str) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(resp.headers().get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn json_response_status() {
        let resp = json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
