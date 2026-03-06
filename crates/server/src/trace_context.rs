use std::sync::OnceLock;

use http::header::HeaderName;
use http::{HeaderMap, HeaderValue, Response};
use tracing::warn;
use uuid::Uuid;

const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");
const TRACESTATE: HeaderName = HeaderName::from_static("tracestate");
const CORRELATION_ID: HeaderName = HeaderName::from_static("correlation-id");
const ENV_TRACE_SAMPLE_PERCENT: &str = "EDGE_RUNTIME_TRACE_SAMPLE_PERCENT";

static SAMPLE_PERCENT: OnceLock<u8> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct TraceContext {
    pub trace_id: String,
    pub traceparent: String,
    pub tracestate: Option<String>,
    pub sampled: bool,
}

pub fn sampling_percent() -> u8 {
    *SAMPLE_PERCENT.get_or_init(|| {
        let raw = std::env::var(ENV_TRACE_SAMPLE_PERCENT).ok();
        match raw.and_then(|v| v.parse::<i32>().ok()) {
            Some(v) if (0..=100).contains(&v) => v as u8,
            Some(v) => {
                warn!(
                    env = ENV_TRACE_SAMPLE_PERCENT,
                    value = v,
                    "trace sample percent out of range, falling back to 100"
                );
                100
            }
            None => 100,
        }
    })
}

pub fn trace_context_from_headers(headers: &HeaderMap) -> TraceContext {
    let sampled = should_sample(sampling_percent());
    let incoming_traceparent = headers.get(&TRACEPARENT).and_then(|v| v.to_str().ok());
    let incoming_tracestate = headers
        .get(&TRACESTATE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let trace_id = match incoming_traceparent {
        Some(tp) => match parse_traceparent(tp) {
            Some(parsed) => parsed.trace_id,
            None => {
                warn!("invalid traceparent received; discarding and generating a new trace");
                new_trace_id()
            }
        },
        None => new_trace_id(),
    };

    let span_id = new_span_id();
    let traceparent = format!(
        "00-{}-{}-{}",
        trace_id,
        span_id,
        if sampled { "01" } else { "00" }
    );

    TraceContext {
        trace_id,
        traceparent,
        tracestate: incoming_tracestate,
        sampled,
    }
}

pub fn apply_trace_headers(headers: &mut HeaderMap, ctx: &TraceContext) {
    if let Ok(value) = HeaderValue::from_str(&ctx.traceparent) {
        headers.insert(TRACEPARENT, value);
    }

    if let Some(state) = &ctx.tracestate {
        if let Ok(value) = HeaderValue::from_str(state) {
            headers.insert(TRACESTATE, value);
        }
    } else {
        headers.remove(TRACESTATE);
    }
}

pub fn add_correlation_id_header<B>(resp: &mut Response<B>, trace_id: &str) {
    if let Ok(value) = HeaderValue::from_str(trace_id) {
        resp.headers_mut().insert(CORRELATION_ID, value);
    }
}

#[derive(Debug)]
struct ParsedTraceparent {
    trace_id: String,
}

fn parse_traceparent(value: &str) -> Option<ParsedTraceparent> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 4 {
        return None;
    }

    let version = parts[0];
    let trace_id = parts[1];
    let span_id = parts[2];
    let flags = parts[3];

    if version.len() != 2 || !is_hex(version) {
        return None;
    }
    if trace_id.len() != 32 || !is_hex(trace_id) || is_all_zero(trace_id) {
        return None;
    }
    if span_id.len() != 16 || !is_hex(span_id) || is_all_zero(span_id) {
        return None;
    }
    if flags.len() != 2 || !is_hex(flags) {
        return None;
    }

    Some(ParsedTraceparent {
        trace_id: trace_id.to_string(),
    })
}

fn is_hex(v: &str) -> bool {
    v.as_bytes().iter().all(|b| b.is_ascii_hexdigit())
}

fn is_all_zero(v: &str) -> bool {
    v.as_bytes().iter().all(|b| *b == b'0')
}

fn new_trace_id() -> String {
    Uuid::new_v4().simple().to_string()
}

fn new_span_id() -> String {
    let raw = Uuid::new_v4().as_u128() as u64;
    format!("{:016x}", raw.max(1))
}

fn should_sample(percent: u8) -> bool {
    if percent >= 100 {
        return true;
    }
    if percent == 0 {
        return false;
    }

    (Uuid::new_v4().as_u128() % 100) < percent as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_traceparent() {
        let tp = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let parsed = parse_traceparent(tp);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap().trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn parse_invalid_traceparent_rejects() {
        assert!(parse_traceparent("bad").is_none());
        assert!(parse_traceparent("00-xyz-00f067aa0ba902b7-01").is_none());
        assert!(parse_traceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01").is_none());
    }

    #[test]
    fn context_generates_on_invalid_traceparent() {
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT, HeaderValue::from_static("invalid"));
        let ctx = trace_context_from_headers(&headers);
        assert_eq!(ctx.trace_id.len(), 32);
        assert!(ctx.traceparent.starts_with("00-"));
    }
}
