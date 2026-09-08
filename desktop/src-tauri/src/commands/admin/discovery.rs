//! NIP-11 admin-origin discovery with same-host trust binding.
//!
//! Fetches the relay's information document and extracts a validated admin
//! console origin from its `admin_api` field, together with a `same_host` flag
//! recording whether that advertised origin targets the same host as the
//! connected relay.
//!
//! # Trust model
//!
//! The `admin_api` value is untrusted relay input. On first mount with no saved
//! origin the desktop auto-saves and auto-probes a discovered origin, and
//! `admin_probe` signs a NIP-98 (kind-27235) header with the operator's key as
//! soon as the origin answers `401 WWW-Authenticate: Nostr`. Auto-probing a
//! *cross-host* advertisement would hand an attacker-controlled server an
//! unconsented signature proving the operator's key ownership and intent.
//!
//! The binding closes that gap: an origin whose host matches the connected
//! relay's host is trusted for auto-save + auto-probe (`same_host == true`); a
//! cross-host advertisement is still surfaced but marked `same_host == false`,
//! and the TypeScript layer treats it as pre-fill-only, requiring the operator
//! to review and explicitly save before anything is signed. Host identity is
//! the binding — scheme and port are not compared, so an operator can legitimately
//! run the admin console on a different port or scheme than the relay.
//!
//! Residual exposure is bounded even for a same-host relay the operator does not
//! fully trust: the NIP-98 header binds the exact request URL, method, and
//! payload, so a captured signature is neither replayable against another
//! endpoint nor usable as a general credential.
//!
//! Discovery still rejects private/reserved hosts and DNS-rebinding
//! (`advertised_host_is_reserved`, `advertised_hostname_resolves_private`)
//! regardless of the same-host flag. Separated from `mod.rs` to keep the parent
//! file under the repository's line-count gate.

use super::origin;

// ── NIP-11 admin-origin discovery ─────────────────────────────────────────

/// Minimal projection of the relay's NIP-11 information document — only the
/// field needed to auto-discover the admin console origin. Unknown fields are
/// ignored, so a full NIP-11 document deserializes cleanly.
#[derive(serde::Deserialize)]
pub(super) struct AdminApiInfo {
    #[serde(default)]
    pub(super) admin_api: Option<String>,
}

/// Validate a relay-advertised `admin_api` value into a canonical origin.
///
/// The value is untrusted relay input, so this is stricter than manual entry:
/// it is accepted only if it passes the same `AdminOrigin` structural
/// validation (origin only — no path, query, fragment, or credentials) AND its
/// host is not a reserved IP literal or the `localhost` name. Manual entry
/// still permits loopback `http` for local development; an auto-advertised
/// origin must never point the operator at an internal target. Hostname
/// targets are additionally DNS-checked by `discover_admin_origin_at` to reject
/// a public name that resolves to a private address (DNS-rebinding-safe).
///
/// Passing this gate does not by itself authorise auto-probing: whether the
/// discovered origin is auto-saved and auto-probed or only pre-filled is
/// governed by the same-host binding (`advertised_host_matches_relay`), which
/// the caller records in the returned `same_host` flag.
///
/// An absent, structurally invalid, or reserved-literal value yields `None` so
/// the desktop falls back to manual entry rather than offering an unsafe origin.
pub(super) fn admin_origin_from_nip11(info: &AdminApiInfo) -> Option<origin::AdminOrigin> {
    let raw = info.admin_api.as_deref()?;
    let origin = origin::AdminOrigin::parse(raw).ok()?;
    if advertised_host_is_reserved(&origin) {
        return None;
    }
    Some(origin)
}

/// Whether an advertised origin's host is a reserved IP literal or `localhost`.
///
/// IP literals are classified synchronously via the shared SSRF predicate;
/// the bare `localhost` name is rejected here because it never needs DNS to be
/// recognised as loopback. Every other hostname is resolved and re-checked in
/// `discover_admin_origin_at`.
pub(super) fn advertised_host_is_reserved(origin: &origin::AdminOrigin) -> bool {
    match origin.resolution_target().0 {
        url::Host::Ipv4(ip) => buzz_core_pkg::network::is_private_ip(&std::net::IpAddr::V4(ip)),
        url::Host::Ipv6(ip) => buzz_core_pkg::network::is_private_ip(&std::net::IpAddr::V6(ip)),
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
    }
}

/// Whether the advertised admin origin targets the same host as the relay
/// reachable at `relay_http_base`.
///
/// Host identity is the trust binding for auto-save + auto-probe (see the module
/// docs); scheme and port are deliberately excluded so an operator can run the
/// admin console on a different port or scheme than the relay. The comparison is
/// ASCII-case-insensitive on the host string forms: `AdminOrigin` already
/// lowercases the advertised host, but the relay-URL side is compared defensively
/// rather than trusting the `url` crate to have lowercased it. IPv6 literals are
/// compared unbracketed on both sides. A relay base that fails to parse or has no
/// host yields `false`, so an unparseable relay URL never binds.
pub(super) fn advertised_host_matches_relay(
    advertised: &origin::AdminOrigin,
    relay_http_base: &str,
) -> bool {
    let Ok(relay_url) = url::Url::parse(relay_http_base) else {
        return false;
    };
    let Some(relay_host) = relay_url.host_str() else {
        return false;
    };
    let relay_host = relay_host.trim_start_matches('[').trim_end_matches(']');
    advertised_host_string(&advertised.resolution_target().0).eq_ignore_ascii_case(relay_host)
}

/// The bare host string of a parsed `url::Host`, without IPv6 brackets.
fn advertised_host_string(host: &url::Host<String>) -> String {
    match host {
        url::Host::Domain(name) => name.clone(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
    }
}

/// Resolve an advertised hostname and reject if any address is private/reserved.
///
/// Split out with an injectable resolver so the DNS-rebinding case (a public
/// name resolving to a private address) is unit-testable without live DNS.
pub(super) async fn advertised_hostname_resolves_private<R, Fut>(
    origin: &origin::AdminOrigin,
    resolve: R,
) -> bool
where
    R: Fn(String, u16) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<std::net::IpAddr>, String>>,
{
    let (host, port) = origin.resolution_target();
    let url::Host::Domain(name) = host else {
        // IP literals are already classified synchronously; nothing to resolve.
        return false;
    };
    match resolve(name, port).await {
        Ok(addrs) => addrs.is_empty() || addrs.iter().any(buzz_core_pkg::network::is_private_ip),
        // A resolution failure is not a positive private verdict; the reserved
        // check has already run, and a same-host auto-probe or the operator's
        // explicit save re-validates the origin against the live network.
        Err(_) => false,
    }
}

/// Real DNS resolver used in production discovery.
pub(super) async fn resolve_host_addrs(
    host: String,
    port: u16,
) -> Result<Vec<std::net::IpAddr>, String> {
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("admin origin DNS resolution failed: {e}"))?
        .map(|addr| addr.ip())
        .collect();
    Ok(addrs)
}

/// Fetch the relay's NIP-11 document and extract a validated admin origin
/// together with its same-host binding flag.
///
/// Returns `Ok(Some(DiscoveredAdminOrigin))` when the relay advertises a valid
/// `admin_api`, `Ok(None)` when the field is absent, fails validation, or
/// resolves to a private/reserved address, and `Err` on a transport or non-2xx
/// failure. The `same_host` flag records whether the advertised origin's host
/// matches `relay_http_base`'s host — the caller (TypeScript) uses it to gate
/// auto-save + auto-probe versus pre-fill-only. Split from the Tauri command so
/// it can be exercised against a live test server without constructing `AppState`.
pub(super) async fn discover_admin_origin_at(
    client: &reqwest::Client,
    relay_http_base: &str,
) -> Result<Option<super::DiscoveredAdminOrigin>, String> {
    discover_admin_origin_at_with(client, relay_http_base, resolve_host_addrs).await
}

/// `discover_admin_origin_at` with an injectable hostname resolver.
pub(super) async fn discover_admin_origin_at_with<R, Fut>(
    client: &reqwest::Client,
    relay_http_base: &str,
    resolve: R,
) -> Result<Option<super::DiscoveredAdminOrigin>, String>
where
    R: Fn(String, u16) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<std::net::IpAddr>, String>>,
{
    use crate::relay::{classify_request_error, parse_json_response, relay_error_message};

    let url = format!("{}/info", relay_http_base.trim_end_matches('/'));
    let response = client
        .get(url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .map_err(|error| classify_request_error(&error))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let info = parse_json_response::<AdminApiInfo>(response).await?;
    let Some(origin) = admin_origin_from_nip11(&info) else {
        return Ok(None);
    };
    if advertised_hostname_resolves_private(&origin, resolve).await {
        return Ok(None);
    }
    let same_host = advertised_host_matches_relay(&origin, relay_http_base);
    Ok(Some(super::DiscoveredAdminOrigin {
        origin: origin.as_str().to_string(),
        same_host,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ── Minimal TCP test server ────────────────────────────────────────────

    type RequestInspector = Arc<dyn Fn(usize, &[u8]) + Send + Sync>;

    async fn serve_sequence_inspect(
        responses: Vec<(&'static str, &'static str, &'static str)>,
        inspect: Option<RequestInspector>,
    ) -> std::net::SocketAddr {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (idx, (status, headers, body)) in responses.into_iter().enumerate() {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if let Some(ref f) = inspect {
                        f(idx, &buf[..n]);
                    }
                    let body_bytes = body.as_bytes();
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
                        body_bytes.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(body_bytes);
                    let _ = stream.flush();
                }
            }
        });
        addr
    }

    async fn serve_sequence(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> std::net::SocketAddr {
        serve_sequence_inspect(responses, None).await
    }

    // ── NIP-11 admin-origin discovery ─────────────────────────────────────

    // Tests for admin_origin_from_nip11 — the untrusted-value gate. An advertised
    // value only PRE-FILLS the operator's origin field (nothing probes it), so it
    // is held to a stricter standard than manual entry: it must pass AdminOrigin's
    // structural validation AND not target a reserved IP literal or `localhost`.
    // DNS-resolving hostnames are re-checked in discover_admin_origin_at_with.

    #[test]
    fn discover_parse_accepts_valid_public_https() {
        let info = AdminApiInfo {
            admin_api: Some("https://admin.example.com".to_string()),
        };
        assert_eq!(
            admin_origin_from_nip11(&info).map(|o| o.as_str().to_string()),
            Some("https://admin.example.com".to_string())
        );
    }

    #[test]
    fn discover_parse_none_when_absent() {
        let info = AdminApiInfo { admin_api: None };
        assert!(admin_origin_from_nip11(&info).is_none());
    }

    #[test]
    fn discover_parse_rejects_structurally_invalid_value() {
        let info = AdminApiInfo {
            admin_api: Some("http://admin.example.com".to_string()),
        };
        assert!(admin_origin_from_nip11(&info).is_none());
    }

    #[test]
    fn discover_parse_rejects_advertised_loopback_literal() {
        for raw in [
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
            "http://localhost:3000",
        ] {
            let info = AdminApiInfo {
                admin_api: Some(raw.to_string()),
            };
            assert!(
                admin_origin_from_nip11(&info).is_none(),
                "advertised loopback {raw:?} must be rejected"
            );
        }
    }

    #[test]
    fn discover_parse_rejects_advertised_private_and_link_local_literal() {
        for raw in [
            "https://10.0.0.5",
            "https://192.168.1.1",
            "https://172.16.0.1",
            "https://169.254.169.254",
            "https://[fe80::1]",
        ] {
            let info = AdminApiInfo {
                admin_api: Some(raw.to_string()),
            };
            assert!(
                admin_origin_from_nip11(&info).is_none(),
                "advertised private/link-local {raw:?} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn discover_returns_origin_when_public_admin_api_advertised() {
        let body = r#"{"name":"Buzz Relay","supported_nips":[1,11],"admin_api":"https://admin.example.com"}"#;
        let addr = serve_sequence(vec![(
            "200 OK",
            "Content-Type: application/nostr+json\r\n",
            body,
        )])
        .await;
        let client = reqwest::Client::new();
        let result =
            discover_admin_origin_at_with(&client, &format!("http://{addr}"), |_host, _port| {
                Box::pin(async { Ok(vec!["93.184.216.34".parse().unwrap()]) })
            })
            .await
            .unwrap()
            .expect("a valid public admin_api is discovered");
        assert_eq!(result.origin, "https://admin.example.com");
        // The relay under test is on 127.0.0.1; the advertised host differs, so
        // the origin is surfaced but not same-host-bound for auto-probe.
        assert!(
            !result.same_host,
            "cross-host advertisement must not be same-host-bound"
        );
    }

    #[tokio::test]
    async fn discover_returns_none_when_admin_api_absent() {
        let body = r#"{"name":"Buzz Relay","supported_nips":[1,11]}"#;
        let addr = serve_sequence(vec![(
            "200 OK",
            "Content-Type: application/nostr+json\r\n",
            body,
        )])
        .await;
        let client = reqwest::Client::new();
        let result = discover_admin_origin_at(&client, &format!("http://{addr}"))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn discover_returns_none_when_advertised_value_invalid() {
        let body = r#"{"admin_api":"http://admin.example.com"}"#;
        let addr = serve_sequence(vec![(
            "200 OK",
            "Content-Type: application/nostr+json\r\n",
            body,
        )])
        .await;
        let client = reqwest::Client::new();
        let result = discover_admin_origin_at(&client, &format!("http://{addr}"))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn discover_returns_none_when_public_hostname_resolves_private() {
        let body = r#"{"admin_api":"https://admin.internal.example"}"#;
        let addr = serve_sequence(vec![(
            "200 OK",
            "Content-Type: application/nostr+json\r\n",
            body,
        )])
        .await;
        let client = reqwest::Client::new();
        let result =
            discover_admin_origin_at_with(&client, &format!("http://{addr}"), |_host, _port| {
                Box::pin(async { Ok(vec!["10.0.0.7".parse().unwrap()]) })
            })
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "a public name resolving to a private address must not be offered"
        );
    }

    #[tokio::test]
    async fn discover_errors_on_non_2xx() {
        let addr = serve_sequence(vec![("500 Internal Server Error", "", "")]).await;
        let client = reqwest::Client::new();
        let result = discover_admin_origin_at(&client, &format!("http://{addr}")).await;
        assert!(
            result.is_err(),
            "non-2xx must surface as Err; got {result:?}"
        );
    }

    #[tokio::test]
    async fn discover_requests_info_path_with_nostr_accept_header() {
        let captured: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_bg = Arc::clone(&captured);
        let body = r#"{"admin_api":"http://127.0.0.1:3000"}"#;
        let addr = serve_sequence_inspect(
            vec![("200 OK", "Content-Type: application/nostr+json\r\n", body)],
            Some(Arc::new(move |_idx, bytes: &[u8]| {
                captured_bg.lock().unwrap().extend_from_slice(bytes);
            })),
        )
        .await;
        let client = reqwest::Client::new();
        let _ = discover_admin_origin_at(&client, &format!("http://{addr}"))
            .await
            .unwrap();
        let request = String::from_utf8_lossy(&captured.lock().unwrap()).to_string();
        assert!(
            request.starts_with("GET /info "),
            "discovery must GET /info; got: {:?}",
            request.lines().next()
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: application/nostr+json"),
            "discovery must send the NIP-11 Accept header"
        );
    }
}
