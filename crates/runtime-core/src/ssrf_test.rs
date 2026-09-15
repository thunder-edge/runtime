use super::*;

#[test]
fn default_config_has_protection_enabled() {
    let config = SsrfConfig::default();
    assert!(config.enabled);
    assert!(config.allow_private_subnets.is_empty());
}

#[test]
fn disabled_config_returns_none_deny_net() {
    let config = SsrfConfig::disabled();
    assert!(!config.enabled);
    assert!(config.build_deny_net().is_none());
}

#[test]
fn enabled_config_returns_deny_ranges() {
    let config = SsrfConfig::new();
    let deny_net = config.build_deny_net().unwrap();
    assert!(deny_net.contains(&"127.0.0.0/8".to_string()));
    assert!(deny_net.contains(&"169.254.0.0/16".to_string()));
    assert!(deny_net.contains(&"10.0.0.0/8".to_string()));
    assert!(!deny_net.contains(&"[fc00::]/7".to_string()));
    assert!(!deny_net.contains(&"[fe80::]/10".to_string()));
    assert!(is_denied_ip("fc00::1".parse().unwrap()));
    assert!(is_denied_ip("fe80::1".parse().unwrap()));
}

#[test]
fn normalizes_ipv4_mapped_ipv6_before_classification() {
    let mapped = "::ffff:169.254.169.254".parse::<IpAddr>().unwrap();

    assert_eq!(
        normalize_ip(mapped),
        "169.254.169.254".parse::<IpAddr>().unwrap()
    );
    assert!(is_denied_ip(mapped));
}

#[test]
fn denies_ipv6_ula_and_link_local_ranges() {
    assert!(is_denied_ip("fd00::1".parse().unwrap()));
    assert!(is_denied_ip("fe80::1".parse().unwrap()));
    assert!(!is_denied_ip("2001:db8::1".parse().unwrap()));
}

#[test]
fn private_subnet_exceptions_do_not_reopen_protected_ranges() {
    let exceptions = vec![
        "10.1.0.0/16".to_string(),
        "127.0.0.0/8".to_string(),
        "169.254.0.0/16".to_string(),
        "fe80::/10".to_string(),
    ];

    assert!(!is_denied_ip_with_exceptions(
        "10.1.2.3".parse().unwrap(),
        &exceptions
    ));
    assert!(is_denied_ip_with_exceptions(
        "127.0.0.1".parse().unwrap(),
        &exceptions
    ));
    assert!(is_denied_ip_with_exceptions(
        "169.254.169.254".parse().unwrap(),
        &exceptions
    ));
    assert!(is_denied_ip_with_exceptions(
        "fe80::1".parse().unwrap(),
        &exceptions
    ));
}

#[test]
fn build_allow_net_filters_unsafe_exceptions() {
    let config = SsrfConfig::with_exceptions(vec![
        "10.1.0.0/16".to_string(),
        "127.0.0.0/8".to_string(),
        "169.254.0.0/16".to_string(),
    ]);

    assert_eq!(config.build_allow_net(), vec!["10.1.0.0/16"]);
}

#[test]
fn config_with_exceptions() {
    let config =
        SsrfConfig::with_exceptions(vec!["10.1.0.0/16".to_string(), "10.2.0.0/16".to_string()]);
    assert!(config.enabled);
    assert_eq!(config.allow_private_subnets.len(), 2);
    assert!(config.build_deny_net().is_some());
}
