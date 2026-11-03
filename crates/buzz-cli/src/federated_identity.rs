//! Headless NIP-FI evidence: mint relay assertions from a NIP-FA adapter
//! session (`docs/nips/NIP-FA.md` §Relay assertion).
//!
//! The adapter session token (`BUZZ_FEDERATED_IDENTITY_TOKEN`) proves who the
//! user is; a NIP-98 proof proves the CLI controls the key. The adapter answers
//! with a short-lived assertion that the CLI attaches as
//! `Nostr-Federated-Identity: Bearer …` on every protected relay request.
//!
//! One assertion is cached per process and re-minted shortly before it
//! expires. The session token is sent only to the adapter's origin, never
//! through a redirect, and never appears in errors.
//!
//! A mint is a single attempt. Retryable failures (network, 429, 5xx) surface
//! as retryable errors, and the request's own bounded retry loop re-mints on
//! its next attempt, so adapter traffic never multiplies across two loops.

use std::time::Duration;

use tokio::sync::Mutex;

use crate::credentials::{
    Credentials, FEDERATED_IDENTITY_ADAPTER_ENV, FEDERATED_IDENTITY_TOKEN_ENV,
};
use crate::error::CliError;

/// NIP-FA assertion endpoint, relative to the adapter base.
const ASSERTIONS_PATH: &str = "/v1/identity/assertions";

/// Largest adapter response body the CLI reads.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Re-mint this long before expiry, or at half-life for lifetimes up to
/// twice this, so a short-lived assertion is not re-minted on every request.
const REFRESH_MARGIN_SECS: u64 = 30;

/// A NIP-FA adapter session that mints NIP-FI assertions.
pub struct AdapterSession {
    endpoint: url::Url,
    token: String,
    http: reqwest::Client,
    cached: Mutex<Option<CachedAssertion>>,
}

struct CachedAssertion {
    relay_url: String,
    assertion: String,
    refresh_at: u64,
}

impl std::fmt::Debug for AdapterSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterSession")
            .field("endpoint", &self.endpoint.as_str())
            .finish_non_exhaustive()
    }
}

#[derive(serde::Deserialize)]
struct AssertionResponse {
    assertion: String,
    nostr_pubkey: String,
    expires_at: u64,
}

#[derive(serde::Deserialize)]
struct DenialBody {
    error: String,
}

/// The NIP-FA canonical relay URL: `wss://host[:port]`, default port omitted,
/// no trailing slash, path, query or fragment.
pub fn canonical_relay_url(relay_url: &str) -> Result<String, CliError> {
    let authority = buzz_core::tenant::relay_url_authority(relay_url);
    if authority.is_empty() {
        return Err(CliError::Usage(format!("invalid relay URL: {relay_url}")));
    }
    Ok(format!("wss://{authority}"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn refresh_at(now: u64, expires_at: u64) -> u64 {
    let lifetime = expires_at.saturating_sub(now);
    let margin = if lifetime > 2 * REFRESH_MARGIN_SECS {
        REFRESH_MARGIN_SECS
    } else {
        lifetime / 2
    };
    expires_at - margin
}

/// Read at most `MAX_RESPONSE_BYTES` of the body; `None` when it is larger.
async fn bounded_body(mut resp: reqwest::Response) -> Result<Option<Vec<u8>>, reqwest::Error> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

fn refused(detail: impl std::fmt::Display) -> CliError {
    CliError::Auth(format!(
        "the identity adapter refused the assertion request ({detail})"
    ))
}

/// Map a NIP-FA denial to the error the CLI reports (NIP-FA §Denials).
/// Retryable statuses (429, 5xx) are handled before this runs.
fn denial(status: u16, code: Option<&str>, relay_url: &str) -> CliError {
    match (status, code) {
        (401, Some("session_required" | "session_expired")) => CliError::Auth(format!(
            "the identity adapter session is missing, invalid or expired ({}): sign in again \
             and update {FEDERATED_IDENTITY_TOKEN_ENV}",
            code.unwrap_or_default()
        )),
        (403, Some("authorization_denied")) => CliError::Auth(format!(
            "the identity adapter denied an assertion for this key on {relay_url} \
             (authorization_denied)"
        )),
        (403, Some("binding_mismatch")) => CliError::Auth(
            "the identity adapter has bound this user to a different Nostr key \
             (binding_mismatch)"
                .into(),
        ),
        (403, Some("invalid_proof")) => {
            CliError::Auth("the identity adapter rejected the NIP-98 proof (invalid_proof)".into())
        }
        (400, Some(code @ "invalid_request")) | (413, Some(code @ "request_too_large")) => {
            refused(code)
        }
        (status, _) => refused(format!("HTTP {status}")),
    }
}

impl AdapterSession {
    /// A session for the adapter at `adapter_base`. The base must be `https`
    /// (or `http` on a loopback host, for local development) with no query,
    /// fragment or credentials.
    pub fn new(adapter_base: &str, token: &str) -> Result<Self, CliError> {
        let invalid = |why: &str| CliError::Auth(format!("{FEDERATED_IDENTITY_ADAPTER_ENV} {why}"));
        let base = url::Url::parse(adapter_base.trim()).map_err(|_| invalid("is not a URL"))?;
        let loopback = match base.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(name)) => name == "localhost",
            None => false,
        };
        match base.scheme() {
            "https" => {}
            "http" if loopback => {}
            _ => return Err(invalid("must use https")),
        }
        if base.query().is_some()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(invalid("must not carry a query, fragment or credentials"));
        }
        let endpoint = url::Url::parse(&format!(
            "{}{ASSERTIONS_PATH}",
            base.as_str().trim_end_matches('/')
        ))
        .map_err(|_| invalid("is not a URL"))?;
        let token = token.trim();
        if token.is_empty() || token.contains(['\r', '\n']) {
            return Err(CliError::Auth(format!(
                "{FEDERATED_IDENTITY_TOKEN_ENV} is not a usable token"
            )));
        }
        let http = reqwest::Client::builder()
            // NIP-FA: never replay the session token or proof to a redirect target.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            endpoint,
            token: token.to_owned(),
            http,
            cached: Mutex::new(None),
        })
    }

    /// The assertion for `relay_url`, minted on first use and re-minted
    /// shortly before it expires. Concurrent callers share one mint.
    pub async fn assertion(
        &self,
        relay_url: &str,
        credentials: &Credentials,
    ) -> Result<String, CliError> {
        let relay_url = canonical_relay_url(relay_url)?;
        let mut cached = self.cached.lock().await;
        if let Some(entry) = cached.as_ref() {
            if entry.relay_url == relay_url && unix_now() < entry.refresh_at {
                return Ok(entry.assertion.clone());
            }
        }
        let (assertion, expires_at) = self.mint(&relay_url, credentials).await?;
        *cached = Some(CachedAssertion {
            relay_url,
            assertion: assertion.clone(),
            refresh_at: refresh_at(unix_now(), expires_at),
        });
        Ok(assertion)
    }

    async fn mint(
        &self,
        relay_url: &str,
        credentials: &Credentials,
    ) -> Result<(String, u64), CliError> {
        let pubkey = credentials.public_key().to_hex();
        let body = serde_json::to_vec(&serde_json::json!({
            "relay_url": relay_url,
            "nostr_pubkey": pubkey,
        }))
        .map_err(|e| CliError::Other(format!("assertion request serialization failed: {e}")))?;

        let proof = credentials
            .nip98("POST", self.endpoint.as_str(), Some(&body))
            .await?;
        let resp = self
            .http
            .post(self.endpoint.clone())
            .bearer_auth(&self.token)
            .header("Nostr-Authorization", format!("Nostr {proof}"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| CliError::Network(e.without_url()))?;
        let status = resp.status().as_u16();
        if status == 429 || resp.status().is_server_error() {
            return Err(CliError::Relay {
                status,
                body: "the identity adapter could not issue an assertion; retry later".into(),
            });
        }
        let body = bounded_body(resp)
            .await
            .map_err(|e| CliError::Network(e.without_url()))?;
        if status != 200 {
            let code = body
                .as_deref()
                .and_then(|b| serde_json::from_slice::<DenialBody>(b).ok())
                .map(|d| d.error);
            return Err(denial(status, code.as_deref(), relay_url));
        }
        accept(body.as_deref(), &pubkey)
    }
}

/// Validate a 200 response (NIP-FA: a rejected 200 is a refusal).
fn accept(body: Option<&[u8]>, pubkey: &str) -> Result<(String, u64), CliError> {
    let response: AssertionResponse = body
        .and_then(|b| serde_json::from_slice(b).ok())
        .ok_or_else(|| refused("unreadable assertion response"))?;
    if response.nostr_pubkey != pubkey {
        return Err(refused("assertion names a different key"));
    }
    if response.expires_at <= unix_now() {
        return Err(refused("assertion already expired"));
    }
    let compact_jws = response.assertion.split('.').count() == 3
        && response
            .assertion
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !compact_jws {
        return Err(refused("assertion is not a compact JWS"));
    }
    Ok((response.assertion, response.expires_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_relay_url_follows_nip_fa() {
        for (input, expected) in [
            ("https://relay.example.com", "wss://relay.example.com"),
            ("https://Relay.Example.com/", "wss://relay.example.com"),
            ("https://relay.example.com:443", "wss://relay.example.com"),
            (
                "https://relay.example.com:8443/x?y",
                "wss://relay.example.com:8443",
            ),
            ("http://localhost:3000", "wss://localhost:3000"),
            ("https://[2001:db8::1]:8443", "wss://[2001:db8::1]:8443"),
        ] {
            assert_eq!(canonical_relay_url(input).unwrap(), expected, "{input}");
        }
        assert!(canonical_relay_url("not a url").is_err());
    }

    #[test]
    fn refresh_keeps_a_margin_without_reminting_short_lifetimes_every_call() {
        assert_eq!(refresh_at(1_000, 1_300), 1_270);
        assert_eq!(refresh_at(1_000, 1_061), 1_031);
        assert_eq!(refresh_at(1_000, 1_060), 1_030);
        assert_eq!(refresh_at(1_000, 1_020), 1_010);
        assert_eq!(refresh_at(1_000, 1_001), 1_001);
    }

    #[test]
    fn adapter_base_must_be_https_or_loopback_http() {
        for ok in [
            "https://adapter.example",
            "https://adapter.example/base/",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
            "http://localhost:8080",
        ] {
            assert!(AdapterSession::new(ok, "token").is_ok(), "{ok}");
        }
        for bad in [
            "http://adapter.example",
            "ftp://adapter.example",
            "https://adapter.example/?q=1",
            "https://adapter.example/#f",
            "https://user:pw@adapter.example",
            "adapter.example",
        ] {
            assert!(AdapterSession::new(bad, "token").is_err(), "{bad}");
        }
        assert!(AdapterSession::new("https://adapter.example", " ").is_err());
        assert!(AdapterSession::new("https://adapter.example", "a\nb").is_err());
        let session = AdapterSession::new("https://adapter.example/base/", "token").unwrap();
        assert_eq!(
            session.endpoint.as_str(),
            "https://adapter.example/base/v1/identity/assertions"
        );
    }

    // -----------------------------------------------------------------------
    // Mint against a mock NIP-FA adapter
    // -----------------------------------------------------------------------

    use std::sync::{Arc, Mutex as StdMutex};

    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use base64::Engine;
    use nostr::{JsonUtil, Keys};

    const TOKEN: &str = "adapter-session-token";
    const ASSERTION: &str = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ1In0.c2ln";

    /// What the adapter saw: request headers and the JSON body.
    type Seen = Arc<StdMutex<Vec<(HeaderMap, serde_json::Value)>>>;

    /// A mock adapter answering every assertion request with `respond(seen
    /// request body)`. Returns the adapter base URL and the request log.
    async fn adapter<F>(respond: F) -> (String, Seen)
    where
        F: Fn(&serde_json::Value) -> (StatusCode, String) + Send + Sync + 'static,
    {
        let seen: Seen = Arc::default();
        let log = seen.clone();
        let respond = Arc::new(respond);
        let app = axum::Router::new().route(
            ASSERTIONS_PATH,
            axum::routing::post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let log = log.clone();
                let respond = respond.clone();
                async move {
                    let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    let (status, body) = respond(&json);
                    log.lock().unwrap().push((headers, json));
                    (status, [("content-type", "application/json")], body).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    fn issued(pubkey: &str, expires_at: u64, assertion: &str) -> (StatusCode, String) {
        (
            StatusCode::OK,
            serde_json::json!({
                "assertion": assertion,
                "nostr_pubkey": pubkey,
                "expires_at": expires_at,
            })
            .to_string(),
        )
    }

    fn denied(status: StatusCode, code: &str) -> (StatusCode, String) {
        (
            status,
            serde_json::json!({"error": code, "message": "denied"}).to_string(),
        )
    }

    #[tokio::test]
    async fn mint_sends_the_nip_fa_request_and_caches_per_relay() {
        let (base, seen) = adapter(|req| {
            issued(
                req["nostr_pubkey"].as_str().unwrap(),
                unix_now() + 300,
                ASSERTION,
            )
        })
        .await;
        let keys = Keys::generate();
        let credentials = Credentials::from_keys(keys.clone());
        let session = AdapterSession::new(&base, TOKEN).unwrap();

        for _ in 0..3 {
            let assertion = session
                .assertion("https://relay.example.com/", &credentials)
                .await
                .unwrap();
            assert_eq!(assertion, ASSERTION);
        }
        assert_eq!(seen.lock().unwrap().len(), 1, "one mint serves the process");

        let (headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(
            body,
            serde_json::json!({
                "relay_url": "wss://relay.example.com",
                "nostr_pubkey": keys.public_key().to_hex(),
            })
        );
        assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
        // NIP-98 proof bound to the endpoint, method and body.
        let proof = headers["nostr-authorization"]
            .to_str()
            .unwrap()
            .strip_prefix("Nostr ")
            .unwrap()
            .to_owned();
        let proof = base64::engine::general_purpose::STANDARD
            .decode(proof)
            .unwrap();
        let proof = nostr::Event::from_json(proof).unwrap();
        proof.verify().unwrap();
        assert_eq!(proof.kind, nostr::Kind::HttpAuth);
        assert_eq!(proof.pubkey, keys.public_key());
        let tag = |name: &str| {
            proof
                .tags
                .iter()
                .find(|t| t.kind().to_string() == name)
                .and_then(|t| t.content())
                .map(str::to_owned)
        };
        assert_eq!(tag("u").unwrap(), format!("{base}{ASSERTIONS_PATH}"));
        assert_eq!(tag("method").unwrap(), "POST");
        let sent = serde_json::to_vec(&body).unwrap();
        assert_eq!(
            tag("payload").unwrap(),
            hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&sent))
        );

        // A different relay needs its own assertion.
        session
            .assertion("https://other.example.com", &credentials)
            .await
            .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_assertion_near_expiry_is_reminted() {
        // A 2 s lifetime refreshes at half-life, so the second call re-mints.
        let (base, seen) = adapter(|req| {
            issued(
                req["nostr_pubkey"].as_str().unwrap(),
                unix_now() + 2,
                ASSERTION,
            )
        })
        .await;
        let credentials = Credentials::from_keys(Keys::generate());
        let session = AdapterSession::new(&base, TOKEN).unwrap();
        session
            .assertion("https://relay.example.com", &credentials)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        session
            .assertion("https://relay.example.com", &credentials)
            .await
            .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    /// NIP-FA §Denials: every status/code the adapter can answer, plus the
    /// 200 responses a client must refuse. Retryable answers stay retryable
    /// (`CliError::Relay` with the status) for the request's retry loop;
    /// everything else is an auth error. No error ever carries the token.
    #[tokio::test]
    async fn denial_table() {
        let other = Keys::generate().public_key().to_hex();
        type Respond = Box<dyn Fn(&str) -> (StatusCode, String) + Send + Sync>;
        let cases: Vec<(&str, Respond, Option<u16>, &str)> = vec![
            (
                "invalid_request",
                Box::new(|_| denied(StatusCode::BAD_REQUEST, "invalid_request")),
                None,
                "invalid_request",
            ),
            (
                "session_required",
                Box::new(|_| denied(StatusCode::UNAUTHORIZED, "session_required")),
                None,
                FEDERATED_IDENTITY_TOKEN_ENV,
            ),
            (
                "session_expired",
                Box::new(|_| denied(StatusCode::UNAUTHORIZED, "session_expired")),
                None,
                FEDERATED_IDENTITY_TOKEN_ENV,
            ),
            (
                "authorization_denied",
                Box::new(|_| denied(StatusCode::FORBIDDEN, "authorization_denied")),
                None,
                "authorization_denied",
            ),
            (
                "invalid_proof",
                Box::new(|_| denied(StatusCode::FORBIDDEN, "invalid_proof")),
                None,
                "invalid_proof",
            ),
            (
                "binding_mismatch",
                Box::new(|_| denied(StatusCode::FORBIDDEN, "binding_mismatch")),
                None,
                "binding_mismatch",
            ),
            (
                "request_too_large",
                Box::new(|_| denied(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large")),
                None,
                "request_too_large",
            ),
            (
                "unknown 4xx",
                Box::new(|_| (StatusCode::IM_A_TEAPOT, "{}".into())),
                None,
                "HTTP 418",
            ),
            (
                "rate_limited",
                Box::new(|_| denied(StatusCode::TOO_MANY_REQUESTS, "rate_limited")),
                Some(429),
                "retry later",
            ),
            (
                "issuance_unavailable",
                Box::new(|_| denied(StatusCode::SERVICE_UNAVAILABLE, "issuance_unavailable")),
                Some(503),
                "retry later",
            ),
            (
                "wrong pubkey",
                Box::new(move |_| issued(&other, unix_now() + 300, ASSERTION)),
                None,
                "different key",
            ),
            (
                "expired",
                Box::new(|pk| issued(pk, unix_now() - 1, ASSERTION)),
                None,
                "expired",
            ),
            (
                "not a compact JWS",
                Box::new(|pk| issued(pk, unix_now() + 300, "not-a-jws")),
                None,
                "compact JWS",
            ),
            (
                "unreadable",
                Box::new(|_| (StatusCode::OK, "{\"assertion\":".into())),
                None,
                "unreadable",
            ),
            (
                "oversized",
                Box::new(|_| (StatusCode::OK, "x".repeat(MAX_RESPONSE_BYTES + 1))),
                None,
                "unreadable",
            ),
        ];
        for (name, respond, retry_status, expected) in cases {
            let (base, seen) =
                adapter(move |req| respond(req["nostr_pubkey"].as_str().unwrap_or_default())).await;
            let credentials = Credentials::from_keys(Keys::generate());
            let session = AdapterSession::new(&base, TOKEN).unwrap();
            let err = session
                .assertion("https://relay.example.com", &credentials)
                .await
                .unwrap_err();
            let message = err.to_string();
            assert!(message.contains(expected), "{name}: {message}");
            assert!(!message.contains(TOKEN), "{name}: token leaked: {message}");
            match (retry_status, &err) {
                (Some(want), CliError::Relay { status, .. }) => assert_eq!(*status, want, "{name}"),
                (None, CliError::Auth(_)) => {}
                _ => panic!("{name}: unexpected error class {err:?}"),
            }
            assert_eq!(
                seen.lock().unwrap().len(),
                1,
                "{name}: one attempt per mint"
            );
            // A failed mint is not cached: the next call asks again.
            let _ = session
                .assertion("https://relay.example.com", &credentials)
                .await;
            assert_eq!(seen.lock().unwrap().len(), 2, "{name}: failure not cached");
        }
    }

    /// NIP-FA: the token and proof never follow a redirect.
    #[tokio::test]
    async fn a_redirect_is_refused_and_never_followed() {
        let (target, target_seen) = adapter(|req| {
            issued(
                req["nostr_pubkey"].as_str().unwrap(),
                unix_now() + 300,
                ASSERTION,
            )
        })
        .await;
        let location = format!("{target}{ASSERTIONS_PATH}");
        let app = axum::Router::new().route(
            ASSERTIONS_PATH,
            axum::routing::post(move || {
                let location = location.clone();
                async move {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", location)],
                        String::new(),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let session = AdapterSession::new(&base, TOKEN).unwrap();
        let err = session
            .assertion(
                "https://relay.example.com",
                &Credentials::from_keys(Keys::generate()),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CliError::Auth(m) if m.contains("HTTP 307")),
            "{err:?}"
        );
        assert!(target_seen.lock().unwrap().is_empty(), "redirect followed");
    }

    #[tokio::test]
    async fn an_unreachable_adapter_is_a_retryable_network_error_without_the_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let session = AdapterSession::new(&base, TOKEN).unwrap();
        let err = session
            .assertion(
                "https://relay.example.com",
                &Credentials::from_keys(Keys::generate()),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CliError::Network(e) if e.is_connect()),
            "{err:?}"
        );
        assert!(!format!("{err:?}").contains(TOKEN));
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let session = AdapterSession::new("https://adapter.example", "s3cr3t-token").unwrap();
        assert!(!format!("{session:?}").contains("s3cr3t-token"));
    }
}
