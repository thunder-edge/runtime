use super::*;
use url::Url;

#[test]
fn default_permissions_created_successfully() {
    // Just verify the container can be created without panic
    let _container = create_permissions_container();
}

#[test]
fn allow_all_permissions_created_successfully() {
    // Just verify the container can be created without panic
    let _container = create_allow_all_permissions();
}

#[test]
fn custom_network_allowlist_created_successfully() {
    let hosts = vec!["example.com".to_string(), "api.example.com:443".to_string()];
    // Just verify the container can be created without panic
    let _container = create_permissions_with_network_allowlist(hosts);
}

#[test]
fn ssrf_blocks_cloud_metadata_ip_for_fetch() {
    let mut container = create_permissions_with_ssrf_protection(&SsrfConfig::default());
    let url = Url::parse("http://169.254.169.254/latest/meta-data/").unwrap();

    let result = container.check_net_url(&url, "fetch()");
    assert!(
        result.is_err(),
        "expected SSRF protection to block metadata IP access"
    );
}

#[test]
fn ssrf_allows_public_https_host_for_fetch() {
    let mut container = create_permissions_with_ssrf_protection(&SsrfConfig::default());
    let url = Url::parse("https://api.github.com/").unwrap();

    let result = container.check_net_url(&url, "fetch()");
    assert!(
        result.is_ok(),
        "expected public host to be allowed with SSRF protection enabled"
    );
}

#[test]
fn policy_allowlist_blocks_unknown_hosts() {
    let mut container = create_permissions_with_policy(
        &SsrfConfig::default(),
        Some(vec!["api.example.com:443".to_string()]),
        None,
    );

    let allowed_url = Url::parse("https://api.example.com/").unwrap();
    let blocked_url = Url::parse("https://not-allowed.example.com/").unwrap();

    assert!(container.check_net_url(&allowed_url, "fetch()").is_ok());
    assert!(container.check_net_url(&blocked_url, "fetch()").is_err());
}

#[test]
fn ssrf_exception_allows_declared_private_subnet_only() {
    let config = SsrfConfig::with_exceptions(vec!["10.1.0.0/16".to_string()]);
    let mut container = create_permissions_with_ssrf_protection(&config);
    let allowed_url = Url::parse("http://10.1.2.3:8080/").unwrap();

    assert!(container.check_net_url(&allowed_url, "fetch()").is_ok());
    assert!(crate::ssrf::is_denied_ip_with_exceptions(
        "10.2.2.3".parse().unwrap(),
        &config.allow_private_subnets
    ));
}
