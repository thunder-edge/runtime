//! SSRF (Server-Side Request Forgery) protection configuration.
//!
//! This module provides configuration for blocking requests to private IP ranges,
//! which is critical for preventing SSRF attacks in multi-tenant environments.

use std::net::IpAddr;

use ipnet::IpNet;
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
    // IPv6 private and reserved ranges
    "[::1]",       // Loopback
    "[fc00::]/7",  // Unique local addresses
    "[fe80::]/10", // Link-local
    // IPv4-mapped IPv6 forms of the blocked IPv4 ranges.
    "[::ffff:127.0.0.0]/104",
    "[::ffff:10.0.0.0]/104",
    "[::ffff:172.16.0.0]/108",
    "[::ffff:192.168.0.0]/112",
    "[::ffff:169.254.0.0]/112",
    "[::ffff:0.0.0.0]/104",
];

pub(crate) fn normalize_network_rule(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix('[') else {
        return trimmed.to_string();
    };
    let Some(close_index) = rest.find(']') else {
        return trimmed.to_string();
    };

    format!("{}{}", &rest[..close_index], &rest[close_index + 1..])
}

/// Normalize an IP address before applying the SSRF policy.
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ipv6) => ipv6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ipv6)),
        IpAddr::V4(ipv4) => IpAddr::V4(ipv4),
    }
}

/// Return whether an IP address is covered by the built-in SSRF denylist.
pub fn is_denied_ip(ip: IpAddr) -> bool {
    let normalized = normalize_ip(ip);

    DEFAULT_DENY_RANGES.iter().any(|raw| {
        let candidate = normalize_network_rule(raw);
        if let Ok(deny_ip) = candidate.parse::<IpAddr>() {
            return deny_ip == normalized;
        }

        candidate
            .parse::<IpNet>()
            .map(|deny_net| deny_net.contains(&normalized))
            .unwrap_or(false)
    })
}

/// Return whether an IP is denied after applying explicitly permitted private subnets.
///
/// Only RFC1918 IPv4 and IPv6 ULA subnets can be exceptions. Loopback, metadata,
/// link-local, reserved, and IPv4-mapped protected ranges remain denied.
pub fn is_denied_ip_with_exceptions(ip: IpAddr, exceptions: &[String]) -> bool {
    if !is_denied_ip(ip) {
        return false;
    }

    let normalized = normalize_ip(ip);
    !exceptions.iter().any(|raw| {
        let candidate = normalize_network_rule(raw);
        let Ok(network) = candidate.parse::<IpNet>() else {
            return false;
        };

        is_allowed_private_network(network) && network.contains(&normalized)
    })
}

fn is_allowed_private_network(network: IpNet) -> bool {
    match network {
        IpNet::V4(network) => {
            let ip = network.network();
            (IpNet::V4("10.0.0.0/8".parse().unwrap()).contains(&IpAddr::V4(ip))
                && network.prefix_len() >= 8)
                || (IpNet::V4("172.16.0.0/12".parse().unwrap()).contains(&IpAddr::V4(ip))
                    && network.prefix_len() >= 12)
                || (IpNet::V4("192.168.0.0/16".parse().unwrap()).contains(&IpAddr::V4(ip))
                    && network.prefix_len() >= 16)
        }
        IpNet::V6(network) => {
            let ip = network.network();
            IpNet::V6("fc00::/7".parse().unwrap()).contains(&IpAddr::V6(ip))
                && network.prefix_len() >= 7
        }
    }
}

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

        Some(
            DEFAULT_DENY_RANGES
                .iter()
                .filter(|raw| is_supported_by_deno_net_descriptor(raw))
                .filter(|raw| !self.exception_overlaps_deny_rule(raw))
                .map(|s| s.to_string())
                .collect(),
        )
    }

    /// Build the allow_net list for Deno permissions.
    /// Includes exception subnets that should be allowed despite SSRF protection.
    pub fn build_allow_net(&self) -> Vec<String> {
        // Note: Deno's permission system evaluates allow_net for specific hosts
        // that would otherwise be blocked by deny_net. Empty vec means "allow all
        // public hosts". The allow_private_subnets are added to allow specific
        // private ranges.
        self.allow_private_subnets
            .iter()
            .filter(|raw| {
                normalize_network_rule(raw)
                    .parse::<IpNet>()
                    .map(is_allowed_private_network)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    fn exception_overlaps_deny_rule(&self, raw: &str) -> bool {
        let Ok(deny_network) = normalize_network_rule(raw).parse::<IpNet>() else {
            return false;
        };

        self.build_allow_net().iter().any(|exception| {
            let Ok(exception_network) = normalize_network_rule(exception).parse::<IpNet>() else {
                return false;
            };
            match (deny_network, exception_network) {
                (IpNet::V4(deny), IpNet::V4(exception)) => {
                    deny.contains(&exception.network())
                        && exception.prefix_len() >= deny.prefix_len()
                }
                (IpNet::V6(deny), IpNet::V6(exception)) => {
                    deny.contains(&exception.network())
                        && exception.prefix_len() >= deny.prefix_len()
                }
                _ => false,
            }
        })
    }
}

fn is_supported_by_deno_net_descriptor(raw: &str) -> bool {
    !(raw.contains(':') && raw.contains('/'))
}

#[cfg(test)]
#[path = "ssrf_test.rs"]
mod tests;
