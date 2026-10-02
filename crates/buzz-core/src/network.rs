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

/// Enumerated-deny SSRF predicate: returns `true` when the address falls within
/// a blocked class, and `false` (accepted) for everything else, including
/// addresses not covered by any explicit deny rule (e.g. `fe00::1`).
///
/// Blocked classes are drawn from the IANA IPv4 and IPv6 Special-Purpose
/// Address Space registries (last updated 2025-10-09): ranges whose
/// `Globally Reachable` column is `False`, `None`, or absent, plus multicast
/// space. Within otherwise-denied envelopes, explicitly global entries are
/// carved out as exceptions (e.g., PCP/TURN/DNS-SD anycast inside 2001::/23).
/// IPv4 embedded in IPv4-mapped, IPv4-compatible, and NAT64 well-known
/// (64:ff9b::/96) space is evaluated recursively against the IPv4 table —
/// registry global=True for the IPv6 wrapper does not bypass the
/// embedded-address check. SIIT IPv4-translated (::ffff:0:0:0/96) follows the
/// same recursive path. The local-use NAT64 prefix (64:ff9b:1::/48) is blocked
/// wholesale as a non-global range; its embedded IPv4 payload is not decoded.
///
/// Used for SSRF protection: rejects outbound targets in known non-public
/// address classes; addresses not covered by an explicit deny rule pass through.
/// Conservative posture: `None`/blank registry entries are treated as non-global.
///
/// Registries retrieved 2026-08-31; registries last updated 2025-10-09:
///   https://www.iana.org/assignments/iana-ipv4-special-registry/
///   https://www.iana.org/assignments/iana-ipv6-special-registry/
///
/// Compatibility alias: `is_private_ip` (see below).
///
/// Callers: `buzz-auth` JWKS boundary, `buzz-workflow` webhook SSRF check,
///          desktop `link_preview` SSRF check.
pub fn is_not_global_unicast(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()                                     // 127.0.0.0/8
                || v4.is_private()                               // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()                            // 169.254.0.0/16
                || o[0] == 0                                     // 0.0.0.0/8 "This network"
                || v4.is_broadcast()                             // 255.255.255.255
                || (o[0] == 100 && (o[1] & 0xC0) == 64)         // 100.64.0.0/10 Shared/CGNAT
                || (o[0] == 198 && (o[1] & 0xFE) == 18)         // 198.18.0.0/15 Benchmarking
                || (o[0] & 0xF0) == 0xE0                        // 224.0.0.0/4 Multicast
                || (o[0] & 0xF0) == 0xF0                        // 240.0.0.0/4 Reserved
                // 192.0.0.0/24 IETF Protocol Assignments.
                // Globally reachable exceptions: 192.0.0.9 (PCP anycast, RFC 7723)
                // and 192.0.0.10 (TURN anycast, RFC 8155).
                || (o[0] == 192 && o[1] == 0 && o[2] == 0
                    && o[3] != 9 && o[3] != 10)
                || (o[0] == 192 && o[1] == 0 && o[2] == 2)      // 192.0.2.0/24 TEST-NET-1
                // 192.88.99.0/24 deprecated 6to4 relay anycast (RFC 7526).
                // Registry global field is None/blank — conservative posture: block.
                || (o[0] == 192 && o[1] == 88 && o[2] == 99)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)   // 198.51.100.0/24 TEST-NET-2
                || (o[0] == 203 && o[1] == 0 && o[2] == 113) // 203.0.113.0/24 TEST-NET-3
        }
        std::net::IpAddr::V6(v6) => {
            // IPv4-compatible and IPv4-mapped addresses are checked against IPv4 rules.
            if let Some(v4) = v6.to_ipv4() {
                return is_not_global_unicast(&std::net::IpAddr::V4(v4));
            }

            let s = v6.segments();

            // NAT64 well-known prefix (RFC 6052): reachability follows the embedded
            // IPv4 address (registry global=True, but SSRF policy checks payload).
            if let Some(v4) = embedded_ipv4(v6, &NAT64_WELL_KNOWN_PREFIX) {
                return is_not_global_unicast(&std::net::IpAddr::V4(v4));
            }

            // SIIT IPv4-translated addresses (::ffff:0:0:0/96) route to the embedded
            // IPv4 value and are not recognised by `to_ipv4()`.
            if let Some(v4) = embedded_ipv4(v6, &IPV4_TRANSLATED_PREFIX) {
                return is_not_global_unicast(&std::net::IpAddr::V4(v4));
            }

            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }

            // 2001::/23 IETF Protocol Assignments envelope (registry global=False).
            // All addresses within the /23 are non-global by default, with explicit
            // globally-reachable exceptions carved out below.
            //
            // /23 check: segments[0]==0x2001 and top 7 bits of segments[1] are zero
            // (i.e., segments[1] in [0x0000..0x01ff]).
            if s[0] == 0x2001 && (s[1] >> 9) == 0 {
                // Globally reachable exceptions inside 2001::/23 (registry global=True):
                //   2001:1::1  PCP Anycast          RFC 7723
                //   2001:1::2  TURN Anycast          RFC 8155
                //   2001:1::3  DNS-SD SRP Anycast    RFC 9665
                //   2001:3::/32  AMT               RFC 7450
                //   2001:4:112::/48  AS112-v6       RFC 7535
                //   2001:20::/28  ORCHIDv2          RFC 7343  (segments[1] in 0x0020..0x002f)
                //   2001:30::/28  DETs Prefix       RFC 9374  (segments[1] in 0x0030..0x003f)
                let is_global_exception = (s[1] == 1
                    && s[2] == 0
                    && s[3] == 0
                    && s[4] == 0
                    && s[5] == 0
                    && s[6] == 0
                    && matches!(s[7], 1..=3))
                    || s[1] == 3                              // 2001:3::/32 AMT
                    || (s[1] == 4 && s[2] == 0x0112)         // 2001:4:112::/48 AS112-v6
                    || (s[1] >> 4) == 0x0002                 // 2001:20::/28 ORCHIDv2
                    || (s[1] >> 4) == 0x0003; // 2001:30::/28 DETs

                if !is_global_exception {
                    return true;
                }
            }

            s[0] & 0xfe00 == 0xfc00                          // fc00::/7 ULA
                || s[0] & 0xffc0 == 0xfe80                   // fe80::/10 link-local
                || s[0] & 0xffc0 == 0xfec0                   // fec0::/10 deprecated site-local (RFC 3879)
                || s[0] & 0xff00 == 0xff00                   // ff00::/8 multicast
                // 64:ff9b:1::/48 local-use NAT64 (RFC 8215)
                || (s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 1)
                // 100::/64 Discard-Only (RFC 6666)
                || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)
                // 100:0:0:1::/64 Dummy IPv6 Prefix (RFC 9780)
                || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 1)
                // 2001:db8::/32 Documentation (RFC 3849) — outside 2001::/23
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || s[0] == 0x2002                            // 2002::/16 6to4 (RFC 3056)
                // 3fff::/20 Documentation (RFC 9637)
                || (s[0] == 0x3fff && (s[1] >> 12) == 0)
                || s[0] == 0x5f00 // 5f00::/16 SRv6 SIDs (RFC 9252)
        }
    }
}

/// Compatibility alias; prefer [`is_not_global_unicast`].
#[inline]
pub fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    is_not_global_unicast(ip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn blocked(s: &str) -> bool {
        is_not_global_unicast(&s.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn public_v4() {
        assert!(!blocked("1.1.1.1"));
        assert!(!blocked("8.8.8.8"));
    }

    #[test]
    fn public_v6_cloudflare() {
        assert!(!blocked("2606:4700::1"));
    }

    #[test]
    fn loopback_and_unspecified() {
        assert!(blocked("127.0.0.1"));
        assert!(blocked("0.0.0.0"));
        assert!(blocked("::1"));
        assert!(blocked("::"));
    }

    #[test]
    fn private_rfc1918() {
        assert!(blocked("10.0.0.1"));
        assert!(blocked("172.16.0.1"));
        assert!(blocked("192.168.1.1"));
    }

    #[test]
    fn link_local() {
        assert!(blocked("169.254.1.1"));
        assert!(blocked("fe80::1"));
    }

    #[test]
    fn broadcast() {
        assert!(blocked("255.255.255.255"));
    }

    #[test]
    fn cgnat() {
        assert!(blocked("100.64.0.1"));
        assert!(blocked("100.127.255.254"));
        assert!(!blocked("100.63.255.255"));
        assert!(!blocked("100.128.0.0"));
    }

    #[test]
    fn benchmarking_v4() {
        assert!(blocked("198.18.0.1"));
        assert!(blocked("198.19.255.254"));
        assert!(!blocked("198.17.255.255"));
        assert!(!blocked("198.20.0.0"));
    }

    #[test]
    fn multicast_and_reserved_v4() {
        assert!(blocked("224.0.0.0"));
        assert!(blocked("239.255.255.255"));
        assert!(blocked("240.0.0.0"));
        assert!(blocked("254.255.255.255"));
        assert!(!blocked("223.255.255.255"));
    }

    // Most of 192.0.0.0/24 is non-global; 192.0.0.9 (PCP, RFC 7723) and
    // 192.0.0.10 (TURN, RFC 8155) are the only globally-reachable exceptions.
    #[test]
    fn ietf_protocol_assignments() {
        assert!(blocked("192.0.0.0"));
        assert!(blocked("192.0.0.1"));
        assert!(blocked("192.0.0.170")); // NAT64/DNS64 discovery — non-global
        assert!(blocked("192.0.0.255"));
        assert!(!blocked("192.0.0.9")); // PCP Anycast (RFC 7723) — global
        assert!(!blocked("192.0.0.10")); // TURN Anycast (RFC 8155) — global
    }

    #[test]
    fn documentation_v4() {
        assert!(blocked("192.0.2.0"));
        assert!(blocked("192.0.2.255"));
        assert!(blocked("198.51.100.0"));
        assert!(blocked("198.51.100.255"));
        assert!(blocked("203.0.113.0"));
        assert!(blocked("203.0.113.255"));
        assert!(!blocked("192.0.1.255"));
        assert!(!blocked("192.0.3.0"));
        assert!(!blocked("198.51.99.255"));
        assert!(!blocked("198.51.101.0"));
        assert!(!blocked("203.0.112.255"));
        assert!(!blocked("203.0.114.0"));
    }

    // Registry global field is None/blank; conservative posture: block.
    #[test]
    fn deprecated_6to4_anycast_v4() {
        assert!(blocked("192.88.99.0"));
        assert!(blocked("192.88.99.1"));
        assert!(blocked("192.88.99.255"));
        assert!(!blocked("192.88.98.255"));
        assert!(!blocked("192.88.100.0"));
    }

    #[test]
    fn ula_v6() {
        assert!(blocked("fd00::1"));
        assert!(blocked("fc00::1"));
    }

    #[test]
    fn multicast_v6() {
        assert!(blocked("ff02::1"));
        assert!(blocked("ff02::2"));
        assert!(blocked("ffff::1"));
        assert!(!blocked("fe00::1"));
    }

    #[test]
    fn ietf_protocol_assignments_v6_interior() {
        assert!(blocked("2001::"));
        assert!(blocked("2001:2::1"));
        assert!(blocked("2001:10::1"));
        assert!(blocked("2001:db8::1")); // Documentation — outside /23 but blocked separately
        assert!(blocked("2001:1ff:ffff::1"));
        assert!(!blocked("2001:200::1"));
    }

    #[test]
    fn ietf_protocol_assignments_v6_global_exceptions() {
        // PCP/TURN/DNS-SD anycast /128s — registry global=True
        assert!(!blocked("2001:1::1")); // PCP Anycast (RFC 7723)
        assert!(!blocked("2001:1::2")); // TURN Anycast (RFC 8155)
        assert!(!blocked("2001:1::3")); // DNS-SD SRP Anycast (RFC 9665)
        assert!(blocked("2001:1::4")); // not an exception
        assert!(blocked("2001:1:1::1")); // not an exception

        // 2001:3::/32 AMT — registry global=True
        assert!(!blocked("2001:3::1"));
        assert!(!blocked("2001:3:ffff::1"));
        assert!(blocked("2001:4::1"));

        // 2001:4:112::/48 AS112-v6 — registry global=True
        assert!(!blocked("2001:4:112::1"));
        assert!(!blocked("2001:4:112:ffff::1"));
        assert!(blocked("2001:4:113::1"));

        // 2001:20::/28 ORCHIDv2 — registry global=True
        assert!(!blocked("2001:20::1"));
        assert!(!blocked("2001:2f::1"));
        assert!(blocked("2001:10::1"));

        // 2001:30::/28 DETs — registry global=True
        assert!(!blocked("2001:30::1"));
        assert!(!blocked("2001:3f::1"));
        assert!(!blocked("2001:3::1")); // AMT exception — distinct check
    }

    #[test]
    fn documentation_v6() {
        assert!(blocked("2001:db8::1"));
        assert!(blocked("2001:db8:ffff::1"));
    }

    #[test]
    fn six_to_four_v6() {
        assert!(blocked("2002::"));
        assert!(blocked("2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));
        assert!(!blocked("2003::1"));
    }

    #[test]
    fn discard_only_v6() {
        assert!(blocked("100::1"));
        assert!(blocked("100::ffff:ffff:ffff:ffff"));
        assert!(!blocked("100:0:1::1")); // outside both discard and dummy ranges
    }

    #[test]
    fn dummy_prefix_v6() {
        assert!(blocked("100:0:0:1::"));
        assert!(blocked("100:0:0:1:ffff:ffff:ffff:ffff"));
        assert!(!blocked("100:0:0:2::1"));
    }

    #[test]
    fn nat64_local_use_v6() {
        assert!(blocked("64:ff9b:1::"));
        assert!(blocked("64:ff9b:1:ffff:ffff:ffff:ffff:ffff"));
        assert!(!blocked("64:ff9b:2::"));
    }

    #[test]
    fn documentation_3fff_v6() {
        assert!(blocked("3fff::1"));
        assert!(blocked("3fff:0fff::1"));
        assert!(!blocked("3fff:1000::1"));
        assert!(!blocked("3ffe::1"));
    }

    #[test]
    fn srv6_sids_v6() {
        assert!(blocked("5f00::1"));
        assert!(blocked("5f00:ffff::1"));
        assert!(!blocked("5e00::1"));
        assert!(!blocked("5fff::1")); // 5fff ≠ 5f00 — outside /16
    }

    #[test]
    fn nat64_well_known_v6() {
        assert!(blocked("64:ff9b::10.0.0.1")); // private embedded
        assert!(blocked("64:ff9b::127.0.0.1")); // loopback embedded
        assert!(blocked("64:ff9b::169.254.169.254")); // link-local embedded
        assert!(!blocked("64:ff9b::8.8.8.8")); // public embedded — policy follows payload
        assert!(!blocked("64:ff9a:ffff:ffff:ffff:ffff:ffff:ffff")); // different prefix
        assert!(!blocked("64:ff9b::1:0:0")); // outside /96
    }

    #[test]
    fn ipv4_translated_v6() {
        assert!(blocked("::ffff:0:10.0.0.1"));
        assert!(blocked("::ffff:0:127.0.0.1"));
        assert!(!blocked("::ffff:0:8.8.8.8"));
        assert!(!blocked("0:0:0:0:fffe:ffff:ffff:ffff")); // outside prefix
    }

    #[test]
    fn ipv4_mapped_v6() {
        assert!(blocked("::ffff:10.0.0.1"));
        assert!(blocked("::ffff:127.0.0.1"));
        assert!(!blocked("::ffff:8.8.8.8"));
    }

    #[test]
    fn ipv4_compatible_v6() {
        assert!(blocked("::10.0.0.1"));
        assert!(blocked("::127.0.0.1"));
        assert!(!blocked("::8.8.8.8"));
    }

    #[test]
    fn deprecated_site_local_fec0() {
        // fec0::/10 — deprecated IPv6 site-local (RFC 3879); blocked as non-global.
        assert!(blocked("fec0::1"));
        assert!(blocked("feff::1")); // fec0::/10 boundary
    }
}
