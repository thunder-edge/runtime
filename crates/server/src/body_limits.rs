//! Request and response body size limits.
//!
//! This module provides utilities for limiting HTTP body sizes to prevent
//! denial of service attacks via large payloads.

use bytes::Bytes;
use http::{header::CONTENT_LENGTH, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, Limited};

/// Body size limits configuration.
#[derive(Debug, Clone, Copy)]
pub struct BodyLimitsConfig {
    /// Maximum request body size in bytes (default: 5 MiB).
    pub max_request_body_bytes: usize,
    /// Maximum response body size in bytes (default: 10 MiB).
    pub max_response_body_bytes: usize,
}

impl Default for BodyLimitsConfig {
    fn default() -> Self {
        Self {
            max_request_body_bytes: 5 * 1024 * 1024,   // 5 MiB
            max_response_body_bytes: 10 * 1024 * 1024, // 10 MiB
        }
    }
}

/// Error types for body limit violations.
#[derive(Debug)]
pub enum BodyLimitError {
    /// Content-Length header exceeds the limit.
    ContentLengthExceeded { declared: u64, limit: usize },
    /// Body size limit exceeded while reading.
    LimitExceeded,
    /// Failed to read the body.
    ReadError(String),
}

impl std::fmt::Display for BodyLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyLimitError::ContentLengthExceeded { declared, limit } => {
                write!(f, "Content-Length {} exceeds limit {}", declared, limit)
            }
            BodyLimitError::LimitExceeded => {
                write!(f, "request body too large")
            }
            BodyLimitError::ReadError(e) => {
                write!(f, "failed to read body: {}", e)
            }
        }
    }
}

impl std::error::Error for BodyLimitError {}

/// Check Content-Length header for fast rejection.
///
/// Returns `Ok(())` if the Content-Length is within limits or not specified.
/// Returns `Err(BodyLimitError::ContentLengthExceeded)` if it exceeds the limit.
pub fn check_content_length<B>(req: &Request<B>, max_bytes: usize) -> Result<(), BodyLimitError> {
    if let Some(content_length) = req.headers().get(CONTENT_LENGTH) {
        if let Ok(length_str) = content_length.to_str() {
            if let Ok(length) = length_str.parse::<u64>() {
                if length > max_bytes as u64 {
                    return Err(BodyLimitError::ContentLengthExceeded {
                        declared: length,
                        limit: max_bytes,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Collect body with size limit using `http_body_util::Limited`.
///
/// This function wraps the body in a `Limited` adapter that enforces the
/// maximum size during reading. If the body exceeds the limit, an error
/// is returned.
pub async fn collect_body_with_limit(
    body: hyper::body::Incoming,
    max_bytes: usize,
) -> Result<Bytes, BodyLimitError> {
    let limited = Limited::new(body, max_bytes);

    match limited.collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(e) => {
            // Check if it's a size limit error
            let err_str = e.to_string();
            if err_str.contains("length limit exceeded") {
                Err(BodyLimitError::LimitExceeded)
            } else {
                Err(BodyLimitError::ReadError(err_str))
            }
        }
    }
}

/// Create a 413 Payload Too Large response.
pub fn payload_too_large_response(limit_bytes: usize) -> Response<Full<Bytes>> {
    let limit_mib = limit_bytes as f64 / (1024.0 * 1024.0);
    let body = format!(
        r#"{{"error":"request body too large","max_size_bytes":{},"max_size_mib":{:.2}}}"#,
        limit_bytes, limit_mib
    );

    Response::builder()
        .status(StatusCode::PAYLOAD_TOO_LARGE)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

/// Check response body size and return error response if exceeded.
pub fn check_response_body_size(body: &Bytes, max_bytes: usize) -> Option<Response<Full<Bytes>>> {
    if body.len() > max_bytes {
        let limit_mib = max_bytes as f64 / (1024.0 * 1024.0);
        let error_body = format!(
            r#"{{"error":"response body too large","max_size_bytes":{},"max_size_mib":{:.2}}}"#,
            max_bytes, limit_mib
        );

        Some(
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(error_body)))
                .unwrap(),
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits() {
        let config = BodyLimitsConfig::default();
        assert_eq!(config.max_request_body_bytes, 5 * 1024 * 1024);
        assert_eq!(config.max_response_body_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn check_content_length_within_limit() {
        let req = Request::builder()
            .header(CONTENT_LENGTH, "1000")
            .body(())
            .unwrap();
        assert!(check_content_length(&req, 5000).is_ok());
    }

    #[test]
    fn check_content_length_exceeds_limit() {
        let req = Request::builder()
            .header(CONTENT_LENGTH, "10000")
            .body(())
            .unwrap();
        let result = check_content_length(&req, 5000);
        assert!(matches!(
            result,
            Err(BodyLimitError::ContentLengthExceeded { .. })
        ));
    }

    #[test]
    fn check_content_length_no_header() {
        let req = Request::builder().body(()).unwrap();
        assert!(check_content_length(&req, 5000).is_ok());
    }

    #[test]
    fn payload_too_large_response_format() {
        let resp = payload_too_large_response(5 * 1024 * 1024);
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn check_response_body_within_limit() {
        let body = Bytes::from(vec![0u8; 1000]);
        assert!(check_response_body_size(&body, 5000).is_none());
    }

    #[test]
    fn check_response_body_exceeds_limit() {
        let body = Bytes::from(vec![0u8; 10000]);
        assert!(check_response_body_size(&body, 5000).is_some());
    }
}
