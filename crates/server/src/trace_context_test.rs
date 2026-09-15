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
