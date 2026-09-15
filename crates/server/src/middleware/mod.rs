use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::{Response, StatusCode};
use http_body_util::Full;

type BoxBody = Full<Bytes>;

#[derive(Debug)]
struct RateLimitState {
    window_start: Instant,
    request_count: u64,
    rps: u64,
}

/// Lightweight fixed-window rate limiter shared across requests.
#[derive(Clone, Debug)]
pub struct RateLimitLayer {
    state: Arc<Mutex<RateLimitState>>,
}

impl RateLimitLayer {
    pub fn new(rps: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimitState {
                window_start: Instant::now(),
                request_count: 0,
                rps: rps.max(1),
            })),
        }
    }

    /// Returns `Some(retry_after_secs)` when request must be rejected.
    pub fn check_limit(&self) -> Option<u64> {
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return Some(1),
        };

        let elapsed = guard.window_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            guard.window_start = Instant::now();
            guard.request_count = 0;
        }

        if guard.request_count < guard.rps {
            guard.request_count += 1;
            return None;
        }

        let remaining = Duration::from_secs(1).saturating_sub(elapsed);
        let secs = remaining.as_secs();
        Some(secs.max(1))
    }
}

/// Build rate limiter state if rate limiting is configured.
pub fn rate_limit_layer(rps: u64) -> RateLimitLayer {
    RateLimitLayer::new(rps)
}

/// Build a 429 response with Retry-After header.
pub fn rate_limited_response(retry_after_secs: u64) -> Response<BoxBody> {
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("content-type", "application/json")
        .header("retry-after", retry_after_secs.to_string())
        .body(Full::new(Bytes::from_static(
            br#"{"error":"rate limit exceeded"}"#,
        )))
        .expect("failed to build rate limited response")
}

#[cfg(test)]
#[path = "middleware_test.rs"]
mod tests;
