//! `AdminOrigin` — a validated canonical admin console URL origin.
//!
//! An `AdminOrigin` holds exactly `scheme://host[:port]` and nothing else.
//! The webview supplies a raw URL string; this type validates and normalises
//! it before any downstream code can use it to construct request URLs.
//!
//! # Accepted inputs
//! - `https://host`          → `https://host`
//! - `https://host:8443`     → `https://host:8443`
//! - `http://localhost`      → `http://localhost`
//! - `http://localhost:3000` → `http://localhost:3000`
//! - `http://127.0.0.1`      → `http://127.0.0.1`
//! - `http://[::1]`          → `http://[::1]`
//!
//! # Rejected inputs
//! - Any URL with `http://` to a non-loopback host
//! - Any URL with credentials (`user:pass@`)
//! - Any URL with a non-root path (`/admin`, `/api`)
//! - Any URL with a query string (`?foo=bar`)
//! - Any URL with a fragment (`#section`)
//! - Unknown or unsupported schemes (`ftp://`, `ws://`)

use super::routes::{AdminQuery, AdminRoute};

/// A validated canonical admin console origin: `scheme://host[:port]`.
///
/// Constructed only through `AdminOrigin::parse`; the inner string is
/// guaranteed to be a valid canonical origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminOrigin(String);

impl AdminOrigin {
    /// Parse and validate an operator-supplied URL into a canonical origin.
    ///
    /// Strips path, query, and fragment. Returns `Err` with a human-readable
    /// message for any disallowed form.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let parsed =
            url::Url::parse(raw).map_err(|_| format!("invalid admin console URL: {raw:?}"))?;

        // Reject credentials.
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err("admin console URL must not contain credentials".to_string());
        }

        // Reject non-root path, query, and fragment.
        let path = parsed.path();
        if path != "/" && !path.is_empty() {
            return Err(format!(
                "admin console URL must be an origin only (no path); got {path:?}"
            ));
        }
        if parsed.query().is_some() {
            return Err("admin console URL must not contain a query string".to_string());
        }
        if parsed.fragment().is_some() {
            return Err("admin console URL must not contain a fragment".to_string());
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| "admin console URL has no host".to_string())?;

        let is_loopback = is_loopback_host(host);

        match parsed.scheme() {
            "https" => {
                // https is allowed for any host, including loopback (dev with TLS).
            }
            "http" => {
                if !is_loopback {
                    return Err(format!(
                        "admin console URL must use HTTPS for non-loopback host {host:?}"
                    ));
                }
            }
            other => {
                return Err(format!(
                    "admin console URL scheme must be https (or http for loopback); got {other:?}"
                ));
            }
        }

        // Build the canonical origin: scheme + "://" + host + optional :port.
        let canonical = match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        };

        Ok(AdminOrigin(canonical))
    }

    /// The canonical origin string, e.g. `https://admin.example.com`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The parsed host and effective port, used to SSRF-resolve an untrusted
    /// relay-advertised origin before it is offered to the operator. Re-derived
    /// from the validated canonical string, which is guaranteed to parse.
    pub fn resolution_target(&self) -> (url::Host<String>, u16) {
        let parsed = url::Url::parse(&self.0).expect("canonical AdminOrigin is always a valid URL");
        let host = parsed
            .host()
            .expect("canonical AdminOrigin always has a host")
            .to_owned();
        let port = parsed.port_or_known_default().unwrap_or(443);
        (host, port)
    }

    /// Build the full request URL for `route` with `query`.
    pub fn route_url(&self, route: &AdminRoute, query: &AdminQuery) -> String {
        let path = route.path();
        let qs = query.to_query_string();
        if qs.is_empty() {
            format!("{}/api/admin/v1{path}", self.0)
        } else {
            format!("{}/api/admin/v1{path}?{qs}", self.0)
        }
    }
}

/// Returns true when `host` is a loopback address (`localhost`,
/// names ending in `.localhost` per RFC 6761, `127.x.x.x`, `[::1]`).
/// This mirrors `media_download.rs`'s localhost carve-out.
fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "[::1]"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Valid inputs ──────────────────────────────────────────────────────────

    #[test]
    fn https_host_accepted() {
        let o = AdminOrigin::parse("https://admin.example.com").unwrap();
        assert_eq!(o.as_str(), "https://admin.example.com");
    }

    #[test]
    fn https_host_port_accepted() {
        let o = AdminOrigin::parse("https://admin.example.com:8443").unwrap();
        assert_eq!(o.as_str(), "https://admin.example.com:8443");
    }

    #[test]
    fn https_trailing_slash_stripped() {
        // url::Url always parses "/" as the path for scheme+host-only URLs.
        let o = AdminOrigin::parse("https://admin.example.com/").unwrap();
        assert_eq!(o.as_str(), "https://admin.example.com");
    }

    #[test]
    fn http_localhost_accepted() {
        let o = AdminOrigin::parse("http://localhost").unwrap();
        assert_eq!(o.as_str(), "http://localhost");
    }

    #[test]
    fn http_localhost_port_accepted() {
        let o = AdminOrigin::parse("http://localhost:3000").unwrap();
        assert_eq!(o.as_str(), "http://localhost:3000");
    }

    #[test]
    fn http_dot_localhost_accepted() {
        let o = AdminOrigin::parse("http://admin.localhost:3000").unwrap();
        assert_eq!(o.as_str(), "http://admin.localhost:3000");
    }

    #[test]
    fn http_dot_localhost_no_port_accepted() {
        let o = AdminOrigin::parse("http://admin.localhost").unwrap();
        assert_eq!(o.as_str(), "http://admin.localhost");
    }

    #[test]
    fn http_dot_localhost_lookalike_rejected() {
        // `admin.localhost.evil` must not be accepted — it doesn't end in `.localhost`.
        assert!(AdminOrigin::parse("http://admin.localhost.evil").is_err());
    }

    #[test]
    fn http_notlocalhost_rejected() {
        // `notlocalhost` is not loopback.
        assert!(AdminOrigin::parse("http://notlocalhost").is_err());
    }

    #[test]
    fn http_127_accepted() {
        let o = AdminOrigin::parse("http://127.0.0.1").unwrap();
        assert_eq!(o.as_str(), "http://127.0.0.1");
    }

    #[test]
    fn http_ipv6_loopback_accepted() {
        let o = AdminOrigin::parse("http://[::1]:3000").unwrap();
        assert_eq!(o.as_str(), "http://[::1]:3000");
    }

    // ── Invalid inputs ────────────────────────────────────────────────────────

    #[test]
    fn http_non_loopback_rejected() {
        assert!(AdminOrigin::parse("http://admin.example.com").is_err());
    }

    #[test]
    fn ftp_scheme_rejected() {
        assert!(AdminOrigin::parse("ftp://admin.example.com").is_err());
    }

    #[test]
    fn credentials_rejected() {
        assert!(AdminOrigin::parse("https://user:pass@admin.example.com").is_err());
    }

    #[test]
    fn path_rejected() {
        assert!(AdminOrigin::parse("https://admin.example.com/api").is_err());
    }

    #[test]
    fn query_rejected() {
        assert!(AdminOrigin::parse("https://admin.example.com?foo=bar").is_err());
    }

    #[test]
    fn fragment_rejected() {
        assert!(AdminOrigin::parse("https://admin.example.com#section").is_err());
    }

    #[test]
    fn garbage_rejected() {
        assert!(AdminOrigin::parse("not a url").is_err());
    }

    // ── route_url builds correct URLs ─────────────────────────────────────────

    #[test]
    fn route_url_reports_list_no_query() {
        let o = AdminOrigin::parse("https://admin.example.com").unwrap();
        let url = o.route_url(&AdminRoute::ReportsList, &AdminQuery::default());
        assert_eq!(url, "https://admin.example.com/api/admin/v1/reports");
    }

    #[test]
    fn route_url_report_detail() {
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let o = AdminOrigin::parse("https://admin.example.com").unwrap();
        let url = o.route_url(&AdminRoute::ReportDetail { id }, &AdminQuery::default());
        assert_eq!(
            url,
            "https://admin.example.com/api/admin/v1/reports/00000000-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn route_url_feedback_attachment() {
        let o = AdminOrigin::parse("https://admin.example.com").unwrap();
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
        let sha256 =
            crate::commands::admin::routes::AttachmentHash::parse(&"ab".repeat(32)).unwrap();
        let url = o.route_url(
            &AdminRoute::FeedbackAttachment { id, sha256 },
            &AdminQuery::default(),
        );
        assert!(url.contains("/api/admin/v1/feedback/"));
        assert!(url.contains("/attachments/"));
    }

    // ── Host case pin test ────────────────────────────────────────────────────
    //
    // The `url` crate (per the URL Standard) lowercases ASCII hostnames during
    // parsing. `AdminOrigin` preserves whatever the URL Standard produces —
    // which for ASCII hostnames is always lowercase. This matches the relay's
    // requirement that the admin console URL's host equals `BUZZ_ADMIN_HOST`
    // byte-for-byte: since the URL parser always lowercases, operators must
    // configure `BUZZ_ADMIN_HOST` in lowercase as well.
    //
    // A relay-side normalization chore (separate PR) would make `BUZZ_ADMIN_HOST`
    // lowercase on startup, eliminating the footgun entirely.
    #[test]
    fn host_case_preserved_as_supplied() {
        // Lowercase input stays lowercase.
        let lower = AdminOrigin::parse("https://admin.example.com").unwrap();
        assert_eq!(lower.as_str(), "https://admin.example.com");

        // The URL Standard normalises ASCII hostnames to lowercase — so "Admin.Example.Com"
        // becomes "admin.example.com" after parsing. Both inputs produce the same
        // canonical origin. Operators must therefore use lowercase in BUZZ_ADMIN_HOST.
        let from_mixed = AdminOrigin::parse("https://Admin.Example.Com").unwrap();
        assert_eq!(
            from_mixed.as_str(),
            "https://admin.example.com",
            "url::Url lowercases ASCII hostnames; canonical origin is always lowercase"
        );

        // Consequently the two parsed origins ARE equal — they produce identical
        // NIP-98 u-tag values and both match a lowercase BUZZ_ADMIN_HOST.
        assert_eq!(lower.as_str(), from_mixed.as_str());
    }
}
