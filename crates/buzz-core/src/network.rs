//! Network utility functions for Buzz.
//!
//! Provides shared helpers used across crates for SSRF protection and
//! IP address classification.

// RFC 6052 well-known NAT64 prefix (64:ff9b::/96).
const NAT64_WELL_KNOWN_PREFIX: [u8; 12] = [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0];

// Legacy SIIT IPv4-translated prefix (::ffff:0:0:0/96).
const IPV4_TRANSLATED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0];

/// Extract an IPv4 address stored in the final four octets under a `/96` prefix.
///
/// Using network-order octets directly avoids error-prone segment shifting.
fn embedded_ipv4(v6: &std::net::Ipv6Addr, prefix: &[u8; 12]) -> Option<std::net::Ipv4Addr> {
    let octets = v6.octets();
    octets
        .starts_with(prefix)
        .then(|| std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]))
}

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
/// - IPv4-compatible and mapped IPv6 (checked recursively against IPv4 rules)
/// - IPv4-translated     ::ffff:0:0:0/96 (embedded IPv4 checked recursively)
/// - NAT64 well-known    64:ff9b::/96 (embedded IPv4 checked recursively)
/// - NAT64 local-use     64:ff9b:1::/48 (RFC 8215)
/// - Teredo              2001::/32 (RFC 4380)
/// - 6to4                2002::/16 (RFC 3056)
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
            // Check IPv4-compatible and mapped addresses against IPv4 rules.
            if let Some(v4) = v6.to_ipv4() {
                return is_private_ip(&std::net::IpAddr::V4(v4));
            }

            let segments = v6.segments();

            // NAT64 well-known prefix (RFC 6052). Preserve access to public IPv4
            // destinations while rejecting embedded private/reserved addresses.
            if let Some(v4) = embedded_ipv4(v6, &NAT64_WELL_KNOWN_PREFIX) {
                return is_private_ip(&std::net::IpAddr::V4(v4));
            }

            // Legacy SIIT IPv4-translated addresses can route to the IPv4 value
            // in their final four octets but are not recognized by `to_ipv4()`.
            if let Some(v4) = embedded_ipv4(v6, &IPV4_TRANSLATED_PREFIX) {
                return is_private_ip(&std::net::IpAddr::V4(v4));
            }

            v6.is_loopback()
                || v6.is_unspecified()
                || segments[0] & 0xfe00 == 0xfc00 // fc00::/7 ULA
                || segments[0] & 0xffc0 == 0xfe80 // fe80::/10 link-local
                || segments[0] & 0xff00 == 0xff00 // ff00::/8 multicast
                || (segments[0] == 0x0064
                    && segments[1] == 0xff9b
                    && segments[2] == 1) // 64:ff9b:1::/48 local-use NAT64
                || (segments[0] == 0x2001 && segments[1] == 0) // 2001::/32 Teredo
                || segments[0] == 0x2002 // 2002::/16 6to4
                // RFC 3849 — documentation range, should never appear in production
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
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
    #[test]
    fn test_ipv4_compatible_v6_private() {
        assert!(is_private_ip(&"::10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"::127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"::169.254.169.254".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(&"::8.8.8.8".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_nat64_well_known_prefix() {
        let first = "64:ff9b::".parse().unwrap();
        let last = "64:ff9b::ffff:ffff".parse().unwrap();
        assert_eq!(
            embedded_ipv4(&first, &NAT64_WELL_KNOWN_PREFIX),
            Some("0.0.0.0".parse().unwrap())
        );
        assert_eq!(
            embedded_ipv4(&last, &NAT64_WELL_KNOWN_PREFIX),
            Some("255.255.255.255".parse().unwrap())
        );
        let embedded = "64:ff9b::172.16.1.2".parse().unwrap();
        assert_eq!(
            embedded_ipv4(&embedded, &NAT64_WELL_KNOWN_PREFIX),
            Some("172.16.1.2".parse().unwrap())
        );
        assert!(is_private_ip(
            &"64:ff9b::10.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(
            &"64:ff9b::127.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(
            &"64:ff9b::169.254.169.254".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(
            &"64:ff9b::8.8.8.8".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(
            &"64:ff9a:ffff:ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(&"64:ff9b::1:0:0".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_ipv4_translated_prefix() {
        let first = "0:0:0:0:ffff:0:0:0".parse().unwrap();
        let last = "0:0:0:0:ffff:0:ffff:ffff".parse().unwrap();
        assert_eq!(
            embedded_ipv4(&first, &IPV4_TRANSLATED_PREFIX),
            Some("0.0.0.0".parse().unwrap())
        );
        assert_eq!(
            embedded_ipv4(&last, &IPV4_TRANSLATED_PREFIX),
            Some("255.255.255.255".parse().unwrap())
        );
        assert!(is_private_ip(
            &"::ffff:0:10.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(
            &"::ffff:0:127.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(
            &"::ffff:0:169.254.169.254".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(
            &"::ffff:0:8.8.8.8".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(
            &"0:0:0:0:fffe:ffff:ffff:ffff".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(
            &"0:0:0:0:ffff:1:0:0".parse::<IpAddr>().unwrap()
        ));
    }
    #[test]
    fn test_nat64_local_use_prefix_boundaries() {
        assert!(is_private_ip(&"64:ff9b:1::".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"64:ff9b:1:ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(
            &"64:ff9b::ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(&"64:ff9b:2::".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_teredo_prefix_boundaries() {
        assert!(is_private_ip(&"2001::".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"2001:0:ffff:ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(
            &"2000:ffff:ffff:ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(&"2001:1::1".parse::<IpAddr>().unwrap()));
    }
    #[test]
    fn test_6to4_prefix_boundaries() {
        assert!(is_private_ip(&"2002::".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(
            &"2001:ffff:ffff:ffff:ffff:ffff:ffff:ffff"
                .parse::<IpAddr>()
                .unwrap()
        ));
        assert!(!is_private_ip(&"2003::1".parse::<IpAddr>().unwrap()));
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
