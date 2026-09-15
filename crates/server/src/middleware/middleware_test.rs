use super::*;

#[test]
fn rate_limit_layer_allows_until_limit_then_rejects() {
    let layer = rate_limit_layer(2);
    assert_eq!(layer.check_limit(), None);
    assert_eq!(layer.check_limit(), None);
    assert!(layer.check_limit().is_some());
}

#[test]
fn rate_limited_response_has_retry_after() {
    let resp = rate_limited_response(3);
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "3");
}
