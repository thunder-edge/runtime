use std::sync::Arc;

use deno_permissions::{
    Permissions, PermissionsContainer, PermissionsOptions, RuntimePermissionDescriptorParser,
};

use crate::ssrf::SsrfConfig;

/// Creates a PermissionsContainer for the edge runtime.
///
/// By default, this grants network access (for fetch, WebSocket, etc.)
/// but denies file system access, environment variables, and subprocess execution.
pub fn create_permissions_container() -> PermissionsContainer {
    let parser = Arc::new(RuntimePermissionDescriptorParser::new(
        sys_traits::impls::RealSys,
    ));

    // Configure permissions: allow network, deny everything else
    let options = PermissionsOptions {
        allow_env: None,
        deny_env: None,
        ignore_env: None,
        allow_net: Some(vec![]), // Empty vec = allow all network
        deny_net: None,
        allow_ffi: None,
        deny_ffi: None,
        allow_read: None,
        deny_read: None,
        ignore_read: None,
        allow_run: None,
        deny_run: None,
        allow_sys: None,
        deny_sys: None,
        allow_write: None,
        deny_write: None,
        allow_import: Some(vec![]), // Allow imports
        deny_import: None,
        prompt: false, // No interactive prompts
    };

    let permissions =
        Permissions::from_options(parser.as_ref(), &options).expect("failed to create permissions");

    PermissionsContainer::new(parser, permissions)
}

/// Creates a PermissionsContainer that allows all operations.
/// Use with caution - only for trusted code or development.
pub fn create_allow_all_permissions() -> PermissionsContainer {
    let parser = Arc::new(RuntimePermissionDescriptorParser::new(
        sys_traits::impls::RealSys,
    ));

    let permissions = Permissions::allow_all();
    PermissionsContainer::new(parser, permissions)
}

/// Creates a PermissionsContainer with custom network allowlist.
pub fn create_permissions_with_network_allowlist(
    allowed_hosts: Vec<String>,
) -> PermissionsContainer {
    let parser = Arc::new(RuntimePermissionDescriptorParser::new(
        sys_traits::impls::RealSys,
    ));

    let options = PermissionsOptions {
        allow_env: None,
        deny_env: None,
        ignore_env: None,
        allow_net: Some(allowed_hosts),
        deny_net: None,
        allow_ffi: None,
        deny_ffi: None,
        allow_read: None,
        deny_read: None,
        ignore_read: None,
        allow_run: None,
        deny_run: None,
        allow_sys: None,
        deny_sys: None,
        allow_write: None,
        deny_write: None,
        allow_import: Some(vec![]),
        deny_import: None,
        prompt: false,
    };

    let permissions =
        Permissions::from_options(parser.as_ref(), &options).expect("failed to create permissions");

    PermissionsContainer::new(parser, permissions)
}

/// Creates a PermissionsContainer with SSRF protection.
///
/// This function configures network permissions to block private IP ranges
/// (loopback, RFC 1918, link-local, cloud metadata endpoints) while allowing
/// public internet access.
///
/// # Arguments
///
/// * `ssrf_config` - Configuration for SSRF protection, including:
///   - Whether protection is enabled
///   - Exception subnets to allow (e.g., corporate networks)
///
/// # Example
///
/// ```rust,ignore
/// use runtime_core::ssrf::SsrfConfig;
/// use runtime_core::permissions::create_permissions_with_ssrf_protection;
///
/// // Default: block all private IPs
/// let config = SsrfConfig::default();
/// let perms = create_permissions_with_ssrf_protection(&config);
///
/// // Allow specific corporate subnet
/// let config = SsrfConfig::with_exceptions(vec!["10.1.0.0/16".to_string()]);
/// let perms = create_permissions_with_ssrf_protection(&config);
/// ```
pub fn create_permissions_with_ssrf_protection(ssrf_config: &SsrfConfig) -> PermissionsContainer {
    create_permissions_with_policy(ssrf_config, None, None)
}

/// Creates a PermissionsContainer with SSRF protection plus optional manifest allowlists.
///
/// - `network_allowlist`:
///   - `None` => allow all network destinations except SSRF denylist.
///   - `Some(vec)` => allow only declared destinations (still denied by SSRF denylist when matching).
/// - `env_allowlist`:
///   - `None` => deny env access.
///   - `Some(vec)` => allow only declared env keys.
pub fn create_permissions_with_policy(
    ssrf_config: &SsrfConfig,
    network_allowlist: Option<Vec<String>>,
    env_allowlist: Option<Vec<String>>,
) -> PermissionsContainer {
    let parser = Arc::new(RuntimePermissionDescriptorParser::new(
        sys_traits::impls::RealSys,
    ));

    // Build deny_net from SSRF config (None if disabled)
    let deny_net = ssrf_config.build_deny_net();

    let allow_net = Some(network_allowlist.unwrap_or_default());

    let allow_env = match env_allowlist {
        Some(allowlist) if allowlist.is_empty() => None,
        Some(allowlist) => Some(allowlist),
        None => None,
    };

    let options = PermissionsOptions {
        allow_env,
        deny_env: None,
        ignore_env: None,
        allow_net,
        deny_net,
        allow_ffi: None,
        deny_ffi: None,
        allow_read: None,
        deny_read: None,
        ignore_read: None,
        allow_run: None,
        deny_run: None,
        allow_sys: None,
        deny_sys: None,
        allow_write: None,
        deny_write: None,
        allow_import: Some(vec![]),
        deny_import: None,
        prompt: false,
    };

    let permissions =
        Permissions::from_options(parser.as_ref(), &options).expect("failed to create permissions");

    PermissionsContainer::new(parser, permissions)
}

#[cfg(test)]
mod tests {
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
}
