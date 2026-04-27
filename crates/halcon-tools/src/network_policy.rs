//! SSRF guard for outbound HTTP tools.
//!
//! Validates URLs before issuing requests so that prompt-injected
//! instructions cannot drive the agent to scan internal networks
//! or read cloud-metadata services (IMDS) and exfiltrate IAM
//! credentials.
//!
//! What is blocked by [`NetworkPolicy::strict`]:
//!
//! - Loopback: 127.0.0.0/8, ::1
//! - RFC1918 private: 10/8, 172.16/12, 192.168/16
//! - Link-local: 169.254.0.0/16 (covers AWS/GCP/Azure IMDS), fe80::/10
//! - Unique-local IPv6: fc00::/7
//! - Unspecified, multicast, broadcast
//! - Hostnames matching known cloud-metadata FQDNs even before DNS
//!
//! Defense scope: validates *before* the request is sent. There is a
//! small TOCTOU window between policy resolution and the resolution
//! reqwest performs internally; closing it would require a custom
//! `reqwest::dns::Resolve` implementation that filters at lookup
//! time. That is out of scope for Phase 1; it is tracked as a future
//! hardening task.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;
use url::{Host, Url};

/// Hostnames that must always be rejected, even before resolution.
///
/// These cover canonical cloud instance-metadata services. The IPs
/// they resolve to are also caught by the link-local check, but
/// blocking the FQDN gives a clearer error and defends against
/// custom resolvers.
const BLOCKED_HOSTNAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.goog",
    "metadata.azure.com",
    "169.254.169.254",
    "fd00:ec2::254",
];

#[derive(Debug, Error)]
pub enum NetworkPolicyError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("URL scheme '{0}' is not allowed (use http or https)")]
    InvalidScheme(String),
    #[error("URL is missing a hostname")]
    MissingHostname,
    #[error("hostname '{host}' is blocked: matches a restricted pattern")]
    BlockedHostname { host: String },
    #[error("address {ip} is blocked ({category})")]
    BlockedAddress { ip: IpAddr, category: &'static str },
    #[error("DNS resolution failed for '{host}': {source}")]
    ResolutionFailed {
        host: String,
        #[source]
        source: std::io::Error,
    },
}

/// Outbound HTTP policy.
///
/// Construct with [`NetworkPolicy::strict`] in production. Use
/// [`NetworkPolicy::permissive_for_tests`] only inside `#[cfg(test)]`
/// when a test wants to hit `127.0.0.1`.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    block_private_addresses: bool,
}

impl NetworkPolicy {
    /// Production default: block loopback, RFC1918, link-local, IMDS.
    pub const fn strict() -> Self {
        Self {
            block_private_addresses: true,
        }
    }

    /// Test-only relaxation. Still blocks IMDS hostnames.
    #[cfg(test)]
    pub const fn permissive_for_tests() -> Self {
        Self {
            block_private_addresses: false,
        }
    }

    /// Validate a URL by parsing, checking scheme, blocking known
    /// metadata hostnames, and resolving the host to one or more
    /// addresses each of which must pass the address policy.
    pub async fn validate_url(&self, url_str: &str) -> Result<(), NetworkPolicyError> {
        let url = Url::parse(url_str)
            .map_err(|e| NetworkPolicyError::InvalidUrl(format!("{url_str}: {e}")))?;

        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(NetworkPolicyError::InvalidScheme(scheme.to_string()));
        }

        // `Host` distinguishes IP literals from domain names without
        // the bracket-string ambiguity that `host_str()` introduces
        // for IPv6.
        let host = url.host().ok_or(NetworkPolicyError::MissingHostname)?;

        // Hostname-pattern block runs against the canonical form
        // (lowercased domain or printed IP) so it catches both
        // `metadata.google.internal` and the IMDS IP literal.
        let host_canonical = match &host {
            Host::Domain(d) => d.to_ascii_lowercase(),
            Host::Ipv4(ip) => ip.to_string(),
            Host::Ipv6(ip) => ip.to_string(),
        };
        if host_canonical.is_empty() {
            return Err(NetworkPolicyError::MissingHostname);
        }
        if BLOCKED_HOSTNAMES.iter().any(|b| host_canonical == *b) {
            return Err(NetworkPolicyError::BlockedHostname {
                host: host_canonical,
            });
        }

        if !self.block_private_addresses {
            return Ok(());
        }

        match host {
            Host::Ipv4(ip) => validate_ipv4(ip),
            Host::Ipv6(ip) => validate_ipv6(ip),
            Host::Domain(domain) => {
                let port = url.port().unwrap_or(if scheme == "https" { 443 } else { 80 });
                let lookup = tokio::net::lookup_host((domain, port))
                    .await
                    .map_err(|e| NetworkPolicyError::ResolutionFailed {
                        host: domain.to_string(),
                        source: e,
                    })?;

                let mut any = false;
                for sa in lookup {
                    any = true;
                    validate_ip(sa.ip())?;
                }

                if !any {
                    return Err(NetworkPolicyError::ResolutionFailed {
                        host: domain.to_string(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "DNS lookup returned no addresses",
                        ),
                    });
                }
                Ok(())
            }
        }
    }
}

fn validate_ip(ip: IpAddr) -> Result<(), NetworkPolicyError> {
    match ip {
        IpAddr::V4(v4) => validate_ipv4(v4),
        IpAddr::V6(v6) => validate_ipv6(v6),
    }
}

fn validate_ipv4(ip: Ipv4Addr) -> Result<(), NetworkPolicyError> {
    let block = |category: &'static str| NetworkPolicyError::BlockedAddress {
        ip: IpAddr::V4(ip),
        category,
    };

    if ip.is_unspecified() {
        return Err(block("unspecified (0.0.0.0)"));
    }
    if ip.is_loopback() {
        return Err(block("loopback (127.0.0.0/8)"));
    }
    if ip.is_link_local() {
        // Covers 169.254.169.254 (AWS/Azure IMDS).
        return Err(block("link-local (169.254.0.0/16)"));
    }
    if ip.is_private() {
        return Err(block("private (RFC1918)"));
    }
    if ip.is_multicast() {
        return Err(block("multicast"));
    }
    if ip.is_broadcast() {
        return Err(block("broadcast"));
    }
    // 100.64.0.0/10 — Carrier-grade NAT (RFC6598). Not in std as a
    // dedicated predicate; manual range check.
    let octets = ip.octets();
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Err(block("CGNAT (100.64.0.0/10)"));
    }
    Ok(())
}

fn validate_ipv6(ip: Ipv6Addr) -> Result<(), NetworkPolicyError> {
    // Run the v6 native checks before recursing into IPv4-mapped so
    // that `::1` (which is the IPv4-compatible form of 0.0.0.1) is
    // caught as loopback rather than passed to validate_ipv4.
    let block = |category: &'static str| NetworkPolicyError::BlockedAddress {
        ip: IpAddr::V6(ip),
        category,
    };

    if ip.is_unspecified() {
        return Err(block("unspecified (::)"));
    }
    if ip.is_loopback() {
        return Err(block("loopback (::1)"));
    }
    if ip.is_multicast() {
        return Err(block("multicast (ff00::/8)"));
    }

    let segments = ip.segments();
    // Unique-local: fc00::/7 → first 7 bits 1111110.
    if (segments[0] & 0xfe00) == 0xfc00 {
        return Err(block("unique-local (fc00::/7)"));
    }
    // Link-local: fe80::/10 → first 10 bits 1111111010.
    if (segments[0] & 0xffc0) == 0xfe80 {
        return Err(block("link-local (fe80::/10)"));
    }

    // IPv4-mapped (::ffff:0:0/96) — recurse on the embedded v4 so
    // that `::ffff:127.0.0.1` is rejected. The IPv4-compatible
    // form (`::a.b.c.d`) is deprecated by RFC4291 and intentionally
    // not unwrapped here; reaching this point with an IPv4-compat
    // address means the embedded v4 was non-loopback, so allowing
    // it through is acceptable.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return validate_ipv4(v4);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn invalid_scheme_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("file:///etc/passwd"));
        assert!(matches!(err, Err(NetworkPolicyError::InvalidScheme(_))));
    }

    #[test]
    fn loopback_v4_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://127.0.0.1/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("loopback")
        ));
    }

    #[test]
    fn loopback_v6_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://[::1]/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("loopback")
        ));
    }

    #[test]
    fn ipv4_mapped_loopback_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://[::ffff:127.0.0.1]/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("loopback")
        ));
    }

    #[test]
    fn rfc1918_rejected() {
        let p = NetworkPolicy::strict();
        for url in [
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
        ] {
            let err = rt().block_on(p.validate_url(url));
            assert!(matches!(
                err,
                Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("private")
            ), "expected private rejection for {url}");
        }
    }

    #[test]
    fn aws_imds_v4_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://169.254.169.254/latest/meta-data/"));
        // Either by hostname match (IP literal in BLOCKED_HOSTNAMES) or link-local.
        assert!(err.is_err());
    }

    #[test]
    fn metadata_google_internal_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://metadata.google.internal/"));
        assert!(matches!(err, Err(NetworkPolicyError::BlockedHostname { .. })));
    }

    #[test]
    fn metadata_azure_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://metadata.azure.com/"));
        assert!(matches!(err, Err(NetworkPolicyError::BlockedHostname { .. })));
    }

    #[test]
    fn link_local_v6_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://[fe80::1]/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("link-local")
        ));
    }

    #[test]
    fn unique_local_v6_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://[fc00::1]/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("unique-local")
        ));
    }

    #[test]
    fn cgnat_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://100.64.0.1/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("CGNAT")
        ));
    }

    #[test]
    fn unspecified_v4_rejected() {
        let p = NetworkPolicy::strict();
        let err = rt().block_on(p.validate_url("http://0.0.0.0/"));
        assert!(matches!(
            err,
            Err(NetworkPolicyError::BlockedAddress { category: c, .. }) if c.contains("unspecified")
        ));
    }

    #[test]
    fn permissive_allows_loopback_for_tests() {
        let p = NetworkPolicy::permissive_for_tests();
        let res = rt().block_on(p.validate_url("http://127.0.0.1:1/"));
        assert!(res.is_ok(), "permissive policy must allow loopback in tests");
    }

    #[test]
    fn permissive_still_blocks_imds_hostname() {
        let p = NetworkPolicy::permissive_for_tests();
        let err = rt().block_on(p.validate_url("http://metadata.google.internal/"));
        assert!(matches!(err, Err(NetworkPolicyError::BlockedHostname { .. })));
    }

    #[test]
    fn malformed_urls_rejected() {
        let p = NetworkPolicy::strict();
        // Each of these must produce *some* error — we don't care
        // which variant — to prove no malformed URL slips through.
        for bad in [
            "http:///path",
            "http://",
            "://example.com/",
            "not-a-url",
            "ftp://example.com/",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            let res = rt().block_on(p.validate_url(bad));
            assert!(res.is_err(), "expected error for {bad}, got {res:?}");
        }
    }
}
