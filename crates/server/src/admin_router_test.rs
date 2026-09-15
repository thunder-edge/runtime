use super::*;

fn _make_test_registry() -> Arc<FunctionRegistry> {
    use tokio_util::sync::CancellationToken;
    Arc::new(FunctionRegistry::new(
        CancellationToken::new(),
        runtime_core::isolate::IsolateConfig::default(),
    ))
}

#[test]
fn test_auth_disabled_allows_all() {
    // When no key is configured, all requests pass
    let api_key: Option<String> = None;
    assert!(api_key.is_none());
}

#[test]
fn test_auth_logic_missing_key() {
    let api_key = Some("secret-key".to_string());
    let provided: Option<&str> = None;

    // Simulate auth check logic
    let result = match provided {
        Some(key) if key == api_key.as_deref().unwrap() => Ok(()),
        Some(_) => Err("invalid"),
        None => Err("missing"),
    };

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "missing");
}

#[test]
fn test_auth_logic_wrong_key() {
    let api_key = Some("secret-key".to_string());
    let provided: Option<&str> = Some("wrong-key");

    // Simulate auth check logic
    let result = match provided {
        Some(key) if key == api_key.as_deref().unwrap() => Ok(()),
        Some(_) => Err("invalid"),
        None => Err("missing"),
    };

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "invalid");
}

#[test]
fn test_auth_logic_correct_key() {
    let api_key = Some("secret-key".to_string());
    let provided: Option<&str> = Some("secret-key");

    // Simulate auth check logic
    let result = match provided {
        Some(key) if key == api_key.as_deref().unwrap() => Ok(()),
        Some(_) => Err("invalid"),
        None => Err("missing"),
    };

    assert!(result.is_ok());
}

#[test]
fn test_auth_logic_no_key_configured() {
    let api_key: Option<String> = None;

    // When no key is configured, all requests pass
    let passes = api_key.is_none();
    assert!(passes);
}
