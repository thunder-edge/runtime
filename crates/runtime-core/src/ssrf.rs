//! SSRF (Server-Side Request Forgery) protection configuration.
//!
//! This module provides configuration for blocking requests to private IP ranges,
//! which is critical for preventing SSRF attacks in multi-tenant environments.

use serde::{Deserialize, Serialize};

/// Default private IP ranges to block (SSRF protection).
///
/// These ranges cover:
/// - Loopback addresses (localhost)
/// - Private networks (RFC 1918)
/// - Link-local addresses (including cloud metadata endpoints like 169.254.169.254)
/// - Reserved addresses
pub const DEFAULT_DENY_RANGES: &[&str] = &[
    // IPv4 private ranges
    "127.0.0.0/8",    // Loopback
    "10.0.0.0/8",     // Private Class A (RFC 1918)
    "172.16.0.0/12",  // Private Class B (RFC 1918)
    "192.168.0.0/16", // Private Class C (RFC 1918)
    "169.254.0.0/16", // Link-local / Cloud metadata (AWS, GCP, Azure)
    "0.0.0.0/8",      // "This" network (reserved)
    // IPv6 equivalents
    "[::1]", // Loopback
             // NOTE: deno_permissions net descriptor parser in this version does not
             // support IPv6 CIDR entries here (for example, fc00::/7, fe80::/10).
];

/// SSRF protection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsrfConfig {
    /// Whether SSRF protection is enabled.
    /// When disabled, all network destinations are allowed.
    pub enabled: bool,

    /// Private subnets to allow despite SSRF protection.
    /// Use CIDR notation (e.g., "10.1.0.0/16").
    /// This is useful for corporate networks that need access to internal services.
    pub allow_private_subnets: Vec<String>,
}

impl Default for SsrfConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_private_subnets: Vec::new(),
        }
    }
}

impl SsrfConfig {
    /// Create a new SSRF config with protection enabled and no exceptions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a disabled SSRF config (allows all network destinations).
    /// **Warning**: Only use this for development or trusted environments.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            allow_private_subnets: Vec::new(),
        }
    }

    /// Create SSRF config with specific private subnet exceptions.
    pub fn with_exceptions(allow_private_subnets: Vec<String>) -> Self {
        Self {
            enabled: true,
            allow_private_subnets,
        }
    }

    /// Build the deny_net list for Deno permissions.
    /// Returns None if SSRF protection is disabled.
    pub fn build_deny_net(&self) -> Option<Vec<String>> {
        if !self.enabled {
            return None;
        }

        Some(DEFAULT_DENY_RANGES.iter().map(|s| s.to_string()).collect())
    }

    /// Build the allow_net list for Deno permissions.
    /// Includes exception subnets that should be allowed despite SSRF protection.
    pub fn build_allow_net(&self) -> Vec<String> {
        // Note: Deno's permission system evaluates allow_net for specific hosts
        // that would otherwise be blocked by deny_net. Empty vec means "allow all
        // public hosts". The allow_private_subnets are added to allow specific
        // private ranges.
        self.allow_private_subnets.clone()
    }
}

#[cfg(test)]
mod tests {
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
    }

    #[test]
    fn config_with_exceptions() {
        let config =
            SsrfConfig::with_exceptions(vec!["10.1.0.0/16".to_string(), "10.2.0.0/16".to_string()]);
        assert!(config.enabled);
        assert_eq!(config.allow_private_subnets.len(), 2);
        assert!(config.build_deny_net().is_some());
    }
}
