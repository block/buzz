//! Network utility functions for Sprout.
//!
//! Provides shared helpers used across crates for SSRF protection and
//! IP address classification.

/// Returns `true` if the IP address is in a private, reserved, or
/// loopback range. Used for SSRF protection — webhook targets must
/// not resolve to these addresses.
///
/// Blocked ranges:
/// - IPv4 loopback       127.0.0.0/8
/// - IPv4 private        10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
/// - IPv4 link-local     169.254.0.0/16
/// - IPv4 unspecified    0.0.0.0/8
/// - IPv4 broadcast      255.255.255.255
/// - IPv4 CGNAT          100.64.0.0/10 (RFC 6598) — cloud metadata risk
/// - IPv4 benchmarking   198.18.0.0/15 (RFC 2544)
/// - IPv6 loopback       ::1
/// - IPv6 unspecified    ::
/// - IPv6 ULA            fc00::/7
/// - IPv6 link-local     fe80::/10
/// - IPv6 multicast      ff00::/8
/// - IPv6 documentation  2001:db8::/32 (RFC 3849) — should never appear in production
/// - IPv4-mapped IPv6    ::ffff:0:0/96 (checked recursively against IPv4 rules)
pub fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || octets[0] == 0
                || v4.is_broadcast()
                // Carrier-Grade NAT (RFC 6598) — 100.64.0.0/10
                // Dangerous in cloud environments (AWS, GCP) where CGNAT can route to metadata services.
                || (octets[0] == 100 && (octets[1] & 0xC0) == 64)
                // Benchmarking (RFC 2544) — 198.18.0.0/15
                || (octets[0] == 198 && (octets[1] & 0xFE) == 18)
        }
        std::net::IpAddr::V6(v6) => {
            // Check IPv4-mapped IPv6 addresses (::ffff:x.x.x.x) against IPv4 rules.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ip(&std::net::IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.segments()[0] & 0xfe00 == 0xfc00 // fc00::/7 ULA
                || v6.segments()[0] & 0xffc0 == 0xfe80 // fe80::/10 link-local
                || v6.segments()[0] & 0xff00 == 0xff00 // ff00::/8 multicast
                // RFC 3849 — documentation range, should never appear in production
                || (v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn test_loopback_v4() {
        assert!(is_private_ip(&"127.0.0.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_private_10() {
        assert!(is_private_ip(&"10.0.0.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_private_172() {
        assert!(is_private_ip(&"172.16.0.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_private_192() {
        assert!(is_private_ip(&"192.168.1.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_link_local() {
        assert!(is_private_ip(&"169.254.1.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_unspecified() {
        assert!(is_private_ip(&"0.0.0.0".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_broadcast() {
        assert!(is_private_ip(&"255.255.255.255".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_public_v4() {
        assert!(!is_private_ip(&"8.8.8.8".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_loopback_v6() {
        assert!(is_private_ip(&"::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_unspecified_v6() {
        assert!(is_private_ip(&"::".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_ula_v6() {
        assert!(is_private_ip(&"fd00::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_link_local_v6() {
        assert!(is_private_ip(&"fe80::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_public_v6() {
        assert!(!is_private_ip(&"2606:4700::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_documentation_range_v6() {
        // 2001:db8::/32 — RFC 3849 documentation range, must be blocked
        assert!(is_private_ip(&"2001:db8::1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"2001:db8:ffff::1".parse::<IpAddr>().unwrap()
        ));
    }
    #[test]
    fn test_ipv4_mapped_v6_private() {
        // ::ffff:10.0.0.1 is an IPv4-mapped IPv6 address pointing to a private IPv4
        assert!(is_private_ip(&"::ffff:10.0.0.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_ipv4_mapped_v6_loopback() {
        assert!(is_private_ip(
            &"::ffff:127.0.0.1".parse::<IpAddr>().unwrap()
        ));
    }
    #[test]
    fn test_ipv4_mapped_v6_public() {
        assert!(!is_private_ip(&"::ffff:8.8.8.8".parse::<IpAddr>().unwrap()));
    }

    // CGNAT (RFC 6598) — 100.64.0.0/10
    #[test]
    fn test_cgnat_start() {
        // 100.64.0.1 — start of CGNAT range
        assert!(is_private_ip(&"100.64.0.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_cgnat_end() {
        // 100.127.255.254 — end of CGNAT range
        assert!(is_private_ip(&"100.127.255.254".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_cgnat_below_range() {
        // 100.63.255.255 — just below CGNAT range (100.0–100.63 is public)
        assert!(!is_private_ip(&"100.63.255.255".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_cgnat_above_range() {
        // 100.128.0.0 — just above CGNAT range (100.128+ is public)
        assert!(!is_private_ip(&"100.128.0.0".parse::<IpAddr>().unwrap()));
    }

    // Benchmarking (RFC 2544) — 198.18.0.0/15
    #[test]
    fn test_benchmarking_start() {
        assert!(is_private_ip(&"198.18.0.1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_benchmarking_end() {
        assert!(is_private_ip(&"198.19.255.254".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_benchmarking_below_range() {
        // 198.17.255.255 — just below benchmarking range
        assert!(!is_private_ip(&"198.17.255.255".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_benchmarking_above_range() {
        // 198.20.0.0 — just above benchmarking range
        assert!(!is_private_ip(&"198.20.0.0".parse::<IpAddr>().unwrap()));
    }

    // IPv6 multicast — ff00::/8
    #[test]
    fn test_ipv6_multicast_all_nodes() {
        // ff02::1 — all-nodes multicast
        assert!(is_private_ip(&"ff02::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_ipv6_multicast_all_routers() {
        // ff02::2 — all-routers multicast
        assert!(is_private_ip(&"ff02::2".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_ipv6_multicast_high() {
        // ffff::1 — still in ff00::/8
        assert!(is_private_ip(&"ffff::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_ipv6_not_multicast() {
        // fe00:: — just below ff00::/8 (not multicast, not link-local, not ULA)
        assert!(!is_private_ip(&"fe00::1".parse::<IpAddr>().unwrap()));
    }
}
