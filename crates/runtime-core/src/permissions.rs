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

    let permissions = Permissions::from_options(parser.as_ref(), &options)
        .expect("failed to create permissions");

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

    let permissions = Permissions::from_options(parser.as_ref(), &options)
        .expect("failed to create permissions");

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
    let parser = Arc::new(RuntimePermissionDescriptorParser::new(
        sys_traits::impls::RealSys,
    ));

    // Build deny_net from SSRF config (None if disabled)
    let deny_net = ssrf_config.build_deny_net();

    // allow_net: empty vec means "allow all hosts not in deny_net"
    // If there are exception subnets, we still use empty vec for allow_net
    // because Deno's permission system will check deny_net first, then allow_net
    // for the specific host being accessed.
    let allow_net = Some(vec![]);

    let options = PermissionsOptions {
        allow_env: None,
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
}
