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

#[test]
fn response_body_limiter_handles_exact_and_excess_chunks() {
    let mut limiter = ResponseBodyLimiter::new(5);
    assert_eq!(
        limiter.consume(Bytes::from_static(b"abcde")),
        ResponseBodyChunk::Forward(Bytes::from_static(b"abcde"))
    );
    assert_eq!(
        limiter.consume(Bytes::from_static(b"f")),
        ResponseBodyChunk::LimitExceeded(Bytes::new())
    );
    assert_eq!(limiter.bytes_seen(), 5);
}

#[test]
fn response_body_limiter_returns_only_remaining_prefix() {
    let mut limiter = ResponseBodyLimiter::new(5);
    assert_eq!(
        limiter.consume(Bytes::from_static(b"abc")),
        ResponseBodyChunk::Forward(Bytes::from_static(b"abc"))
    );
    assert_eq!(
        limiter.consume(Bytes::from_static(b"def")),
        ResponseBodyChunk::LimitExceeded(Bytes::from_static(b"de"))
    );
}

#[test]
fn response_body_limiter_rejects_first_chunk_when_limit_is_zero() {
    let mut limiter = ResponseBodyLimiter::new(0);
    assert_eq!(
        limiter.consume(Bytes::from_static(b"a")),
        ResponseBodyChunk::LimitExceeded(Bytes::new())
    );
}

#[test]
fn response_body_too_large_response_is_stable() {
    let response = response_body_too_large_response(5 * 1024 * 1024);
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
}
