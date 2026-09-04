use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::lookup_host;
use url::Url;

const MAX_RESULT_RESPONSE_BYTES: usize = 4_096;
const MAX_RESOLVED_ADDRESSES: usize = 16;
const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

const ACTION_REQUIRED_CODES: &[&str] = &[
    "BUZZ_ADOPTION_CALLBACK_INVALID",
    "BUZZ_ADOPTION_OFFER_NOT_ACTIVE",
    "BUZZ_ADOPTION_TARGET_MISMATCH",
    "BUZZ_IDEMPOTENCY_CONFLICT",
    "BUZZ_OWNER_PROOF_INVALID",
    "STEP_UP_REQUIRED",
    "BUZZ_ADOPTION_COMPLETION_REJECTED",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResultResponse {
    status: String,
    result_code: Option<String>,
}

/// Minimal authoritative result returned to the consent UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NostrBindResult {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_code: Option<String>,
}

/// Stable, non-sensitive failure classification returned to the consent UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NostrBindResultError {
    kind: &'static str,
    message: &'static str,
}

impl NostrBindResultError {
    fn unsafe_endpoint() -> Self {
        Self {
            kind: "unsafe_endpoint",
            message: "The confirmation endpoint is not safe to contact.",
        }
    }

    fn unreachable() -> Self {
        Self {
            kind: "unreachable",
            message: "Run402 could not be reached to confirm the result.",
        }
    }

    fn invalid_response() -> Self {
        Self {
            kind: "invalid_response",
            message: "Run402 returned an invalid confirmation response.",
        }
    }
}

fn validate_result_url(input: &str) -> Result<Url, NostrBindResultError> {
    if input
        .strip_prefix("https://")
        .is_some_and(|authority| authority.starts_with('/'))
    {
        return Err(NostrBindResultError::unsafe_endpoint());
    }
    let url = Url::parse(input).map_err(|_| NostrBindResultError::unsafe_endpoint())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(NostrBindResultError::unsafe_endpoint());
    }
    Ok(url)
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (18..=19).contains(&b))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }

    let segments = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || segments[..6] == [0, 0, 0, 0, 0, 0]
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1)
        || (segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0)
        || (segments[0] == 0x2001 && segments[1] <= 0x01ff)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || segments[0] == 0x2002
        || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0)
        || segments[0] == 0x5f00)
}

fn is_public_endpoint_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn parse_result_response(body: &[u8]) -> Result<NostrBindResult, NostrBindResultError> {
    if body.len() > MAX_RESULT_RESPONSE_BYTES {
        return Err(NostrBindResultError::invalid_response());
    }
    let response: WireResultResponse =
        serde_json::from_slice(body).map_err(|_| NostrBindResultError::invalid_response())?;

    let valid = match response.status.as_str() {
        "pending" | "completed" | "expired" | "cancelled" => response.result_code.is_none(),
        "action_required" => response
            .result_code
            .as_deref()
            .is_some_and(|code| ACTION_REQUIRED_CODES.contains(&code)),
        _ => false,
    };
    if !valid {
        return Err(NostrBindResultError::invalid_response());
    }

    Ok(NostrBindResult {
        status: response.status,
        result_code: response.result_code,
    })
}

async fn resolve_public_addresses(url: &Url) -> Result<Vec<SocketAddr>, NostrBindResultError> {
    let host = url
        .host_str()
        .ok_or_else(NostrBindResultError::unsafe_endpoint)?;
    let port = url
        .port_or_known_default()
        .ok_or_else(NostrBindResultError::unsafe_endpoint)?;
    let resolved = tokio::time::timeout(DNS_TIMEOUT, lookup_host((host, port)))
        .await
        .map_err(|_| NostrBindResultError::unreachable())?
        .map_err(|_| NostrBindResultError::unreachable())?;
    let addresses: Vec<_> = resolved.take(MAX_RESOLVED_ADDRESSES + 1).collect();

    validate_resolved_addresses(addresses)
}

fn validate_resolved_addresses(
    addresses: Vec<SocketAddr>,
) -> Result<Vec<SocketAddr>, NostrBindResultError> {
    if addresses.is_empty()
        || addresses.len() > MAX_RESOLVED_ADDRESSES
        || addresses
            .iter()
            .any(|address| !is_public_endpoint_ip(address.ip()))
    {
        return Err(NostrBindResultError::unsafe_endpoint());
    }
    Ok(addresses)
}

async fn read_result_response(
    response: reqwest::Response,
) -> Result<NostrBindResult, NostrBindResultError> {
    if response.status() != reqwest::StatusCode::OK {
        return Err(
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                || response.status().is_server_error()
            {
                NostrBindResultError::unreachable()
            } else {
                NostrBindResultError::invalid_response()
            },
        );
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESULT_RESPONSE_BYTES as u64)
    {
        return Err(NostrBindResultError::invalid_response());
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| NostrBindResultError::invalid_response())?;
        if body.len().saturating_add(chunk.len()) > MAX_RESULT_RESPONSE_BYTES {
            return Err(NostrBindResultError::invalid_response());
        }
        body.extend_from_slice(&chunk);
    }
    parse_result_response(&body)
}

/// Post the in-memory signed approval to its issuer's result endpoint.
///
/// This command performs one bounded read. Poll timing and active-request
/// isolation remain in the frontend, while URL, DNS, redirect, and body-size
/// enforcement stay in native code where browser policy cannot be bypassed.
#[tauri::command]
pub async fn fetch_nostr_bind_result(
    result_url: String,
    owner_proof_event: Value,
) -> Result<NostrBindResult, NostrBindResultError> {
    let url = validate_result_url(&result_url)?;
    let host = url
        .host_str()
        .ok_or_else(NostrBindResultError::unsafe_endpoint)?
        .to_owned();
    let addresses = resolve_public_addresses(&url).await?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_max_idle_per_host(0)
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|_| NostrBindResultError::unreachable())?;
    let response = client
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&serde_json::json!({ "owner_proof_event": owner_proof_event }))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|_| NostrBindResultError::unreachable())?;

    read_result_response(response).await
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    use super::{
        fetch_nostr_bind_result, is_public_endpoint_ip, parse_result_response,
        read_result_response, validate_resolved_addresses, validate_result_url,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn local_http_response(wire_response: Vec<u8>) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1_024];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(&wire_response).await.unwrap();
        });
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(format!("http://{address}/result"))
            .send()
            .await
            .unwrap();
        server.await.unwrap();
        response
    }

    #[test]
    fn result_url_requires_a_credential_free_https_origin_without_query_or_fragment() {
        assert!(
            validate_result_url("https://api.run402.com/buzz-human-adoption-results/v1").is_ok()
        );

        for unsafe_url in [
            "http://api.run402.com/result",
            "https://user:pass@api.run402.com/result",
            "https://api.run402.com/result?token=secret",
            "https://api.run402.com/result#fragment",
            "https:///result",
            "not a url",
        ] {
            assert!(validate_result_url(unsafe_url).is_err(), "{unsafe_url}");
        }
    }

    #[test]
    fn result_endpoint_allows_only_publicly_routable_addresses() {
        assert!(is_public_endpoint_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(is_public_endpoint_ip(IpAddr::V6(
            "2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap()
        )));

        for unsafe_ip in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::a00:1",
            "100::1",
            "2001:db8::1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
        ] {
            let parsed = unsafe_ip.parse::<IpAddr>().unwrap();
            assert!(!is_public_endpoint_ip(parsed), "{unsafe_ip}");
        }
    }

    #[test]
    fn result_endpoint_rejects_mixed_or_oversized_dns_answers() {
        let public: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let private: SocketAddr = "127.0.0.1:443".parse().unwrap();
        assert!(validate_resolved_addresses(vec![public]).is_ok());
        assert!(validate_resolved_addresses(vec![public, private]).is_err());
        assert!(validate_resolved_addresses(vec![public; 17]).is_err());
    }

    #[test]
    fn result_response_accepts_only_the_exact_bounded_contract() {
        for body in [
            br#"{"status":"pending"}"#.as_slice(),
            br#"{"status":"completed"}"#.as_slice(),
            br#"{"status":"expired"}"#.as_slice(),
            br#"{"status":"cancelled"}"#.as_slice(),
            br#"{"status":"action_required","result_code":"STEP_UP_REQUIRED"}"#.as_slice(),
            br#"{"status":"action_required","result_code":"BUZZ_OWNER_PROOF_INVALID"}"#.as_slice(),
        ] {
            assert!(parse_result_response(body).is_ok());
        }

        for body in [
            br#"{"status":"completed","extra":true}"#.as_slice(),
            br#"{"status":"completed","result_code":"STEP_UP_REQUIRED"}"#.as_slice(),
            br#"{"status":"action_required"}"#.as_slice(),
            br#"{"status":"action_required","result_code":"INTERNAL_ERROR"}"#.as_slice(),
            br#"{"status":"unknown"}"#.as_slice(),
            br#"not json"#.as_slice(),
        ] {
            assert!(parse_result_response(body).is_err());
        }

        let oversized = vec![b' '; 4_097];
        assert!(parse_result_response(&oversized).is_err());
    }

    #[test]
    fn result_response_serializes_for_the_frontend_without_extra_state() {
        let parsed = parse_result_response(
            br#"{"status":"action_required","result_code":"BUZZ_ADOPTION_TARGET_MISMATCH"}"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::json!({
                "status": "action_required",
                "resultCode": "BUZZ_ADOPTION_TARGET_MISMATCH"
            })
        );
    }

    #[tokio::test]
    async fn result_response_rejects_redirects_and_streamed_oversize_bodies() {
        let redirect = local_http_response(
            b"HTTP/1.1 302 Found\r\nLocation: https://example.com/result\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec(),
        )
        .await;
        assert_eq!(
            read_result_response(redirect).await.unwrap_err().kind,
            "invalid_response"
        );

        let mut oversized =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                .to_vec();
        oversized.extend(std::iter::repeat_n(b' ', 4_097));
        let response = local_http_response(oversized).await;
        assert_eq!(
            read_result_response(response).await.unwrap_err().kind,
            "invalid_response"
        );
    }

    #[tokio::test]
    async fn result_command_rejects_loopback_before_contacting_it() {
        let error = fetch_nostr_bind_result(
            "https://127.0.0.1/result".to_owned(),
            serde_json::json!({ "id": "signed-event" }),
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind, "unsafe_endpoint");
    }
}
