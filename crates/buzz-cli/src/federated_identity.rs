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
//! A mint retries network failures, 429 and any 5xx with bounded backoff
//! (NIP-FA §Denials), then fails as [`CliError::Adapter`]: a pre-send failure
//! that relay retry loops never repeat and never report as possibly
//! delivered. A refusal is never retried, even when its body is unreadable.

use std::time::Duration;

use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use sha2::{Digest, Sha256};

use crate::cached_assertion::{unix_now, CachedAssertion};
use crate::error::CliError;

/// Environment variable carrying a NIP-FA adapter session token.
pub const FEDERATED_IDENTITY_TOKEN_ENV: &str = "BUZZ_FEDERATED_IDENTITY_TOKEN";
/// Environment variable carrying the NIP-FA adapter base URL.
pub const FEDERATED_IDENTITY_ADAPTER_ENV: &str = "BUZZ_FEDERATED_IDENTITY_ADAPTER";

/// NIP-FA assertion endpoint, relative to the adapter base.
const ASSERTIONS_PATH: &str = "/v1/identity/assertions";

/// Largest adapter response body the CLI reads.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Attempts per mint, and the backoff ceiling before each retry.
const MINT_ATTEMPTS: usize = 3;
const MINT_BACKOFF: [Duration; MINT_ATTEMPTS - 1] =
    [Duration::from_millis(250), Duration::from_millis(750)];
/// The longest adapter `Retry-After` the CLI waits out before a retry.
const MINT_RETRY_AFTER_MAX: Duration = Duration::from_secs(30);

/// A NIP-FA adapter session that mints NIP-FI assertions.
pub struct AdapterSession {
    endpoint: url::Url,
    /// The relay it mints for, in NIP-FA form, bound at creation.
    relay_url: String,
    token: String,
    http: reqwest::Client,
    cached: CachedAssertion,
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
    // `wss://host` means 443, so an explicit `:80` would silently alias it;
    // NIP-FA has no relay on port 80. The URL parser drops `:80` for
    // `http`/`ws`, so read the authority as written.
    if explicit_port(relay_url) == Some(80) {
        return Err(CliError::Usage(format!(
            "NIP-FA does not support a relay on port 80: {relay_url}"
        )));
    }
    let authority = buzz_core::tenant::relay_url_authority(relay_url);
    if authority.is_empty() {
        return Err(CliError::Usage(format!("invalid relay URL: {relay_url}")));
    }
    Ok(format!("wss://{authority}"))
}

/// The port written in `url`'s authority, as a number, if any.
fn explicit_port(url: &str) -> Option<u16> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = host_port.rsplit_once(':')?;
    if host.is_empty() || port.contains(']') {
        return None;
    }
    port.parse().ok()
}

/// An adapter `Retry-After`: delta-seconds or an HTTP-date in any of the
/// three RFC 9110 formats.
fn parse_retry_after(value: &str, now: u64) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse() {
        return Some(Duration::from_secs(secs));
    }
    let at = httpdate::parse_http_date(value)
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Duration::from_secs(at.saturating_sub(now)))
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
    /// A session minting for `relay_url` from the adapter at `adapter_base`.
    /// The base must be `https` (or `http` on a loopback host, for local
    /// development) with no query, fragment or credentials.
    pub fn new(adapter_base: &str, token: &str, relay_url: &str) -> Result<Self, CliError> {
        let relay_url = canonical_relay_url(relay_url)?;
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
            relay_url,
            token: token.to_owned(),
            http,
            cached: CachedAssertion::default(),
        })
    }

    /// The assertion for its relay, minted with `keys` on first use and
    /// re-minted at its refresh point. Concurrent callers share one mint.
    pub async fn assertion(&self, keys: &Keys) -> Result<Option<String>, CliError> {
        self.cached
            .get(async { self.mint(&self.relay_url, keys).await.map(Some) })
            .await
    }

    /// Mint with bounded retries; a refusal or a definitive error returns at once.
    async fn mint(&self, relay_url: &str, keys: &Keys) -> Result<String, CliError> {
        let mut attempt = 0;
        loop {
            match self.mint_once(relay_url, keys).await? {
                Ok(issued) => return Ok(issued),
                Err(transient) if attempt + 1 == MINT_ATTEMPTS => {
                    return Err(CliError::Adapter(format!(
                        "the identity adapter could not issue an assertion after \
                         {MINT_ATTEMPTS} attempts ({transient}); retry later"
                    )));
                }
                Err(transient) => {
                    let wait = transient.retry_after.map_or_else(
                        || MINT_BACKOFF[attempt].mul_f64(rand::random::<f64>()),
                        |d| d.min(MINT_RETRY_AFTER_MAX),
                    );
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
            }
        }
    }

    /// One mint attempt. The outer `Err` ends the mint; the inner `Err` is a
    /// transient failure worth another attempt.
    async fn mint_once(
        &self,
        relay_url: &str,
        keys: &Keys,
    ) -> Result<Result<String, Transient>, CliError> {
        let pubkey = keys.public_key().to_hex();
        let body = serde_json::to_vec(&serde_json::json!({
            "relay_url": relay_url,
            "nostr_pubkey": pubkey,
        }))
        .map_err(|e| CliError::Other(format!("assertion request serialization failed: {e}")))?;

        // A fresh proof per attempt: NIP-FA proofs are single-use.
        let proof = nip98_event("POST", self.endpoint.as_str(), Some(&body))?
            .sign_with_keys(keys)
            .map_err(|e| CliError::Other(format!("NIP-98 signing failed: {e}")))?;
        use base64::Engine;
        let proof = base64::engine::general_purpose::STANDARD.encode(proof.as_json());
        let mut proof = reqwest::header::HeaderValue::try_from(format!("Nostr {proof}"))
            .map_err(|_| CliError::Other("NIP-98 proof is not a valid header value".into()))?;
        proof.set_sensitive(true);
        let sent = self
            .http
            .post(self.endpoint.clone())
            .bearer_auth(&self.token)
            .header("Nostr-Authorization", proof)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await;
        let resp = match sent {
            Ok(resp) => resp,
            Err(e) if e.is_builder() => return Err(CliError::Network(e.without_url())),
            Err(e) => return Ok(Err(Transient::from(e))),
        };
        let status = resp.status().as_u16();
        if status == 429 || resp.status().is_server_error() {
            // NIP-FA: honor `Retry-After` (delta-seconds) on 429 and 503.
            let retry_after = matches!(status, 429 | 503)
                .then(|| resp.headers().get(reqwest::header::RETRY_AFTER))
                .flatten()
                .and_then(|v| parse_retry_after(v.to_str().ok()?, unix_now()));
            return Ok(Err(Transient {
                why: format!("HTTP {status}"),
                retry_after,
            }));
        }
        let body = bounded_body(resp).await;
        if status != 200 {
            // The status is the answer; a lost or unreadable body only loses the code.
            let code = body
                .ok()
                .flatten()
                .and_then(|b| serde_json::from_slice::<DenialBody>(&b).ok())
                .map(|d| d.error);
            return Err(denial(status, code.as_deref(), relay_url));
        }
        match body {
            Ok(body) => accept(body.as_deref(), &pubkey).map(Ok),
            // A 200 whose body never arrived is a transport failure, not a rejected 200.
            Err(e) => Ok(Err(Transient::from(e))),
        }
    }
}

/// A mint failure worth another attempt, after `retry_after` when the
/// adapter gave one.
struct Transient {
    why: String,
    retry_after: Option<Duration>,
}

impl From<reqwest::Error> for Transient {
    fn from(e: reqwest::Error) -> Self {
        Self {
            why: crate::error::fmt_reqwest_error(&e.without_url()),
            retry_after: None,
        }
    }
}

impl std::fmt::Display for Transient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.why)
    }
}

/// Validate a 200 response (NIP-FA: a rejected 200 is a refusal).
fn accept(body: Option<&[u8]>, pubkey: &str) -> Result<String, CliError> {
    let response: AssertionResponse = body
        .and_then(|b| serde_json::from_slice(b).ok())
        .ok_or_else(|| refused("unreadable assertion response"))?;
    if response.nostr_pubkey != pubkey {
        return Err(refused("assertion names a different key"));
    }
    if response.expires_at <= unix_now() {
        return Err(refused("assertion already expired"));
    }
    match crate::cached_assertion::jws_exp(&response.assertion) {
        None => return Err(refused("assertion is not a compact JWS")),
        Some(exp) if exp != response.expires_at => {
            return Err(refused("assertion exp does not match expires_at"));
        }
        Some(_) => {}
    }
    Ok(response.assertion)
}

/// The NIP-98 HTTP auth event (kind:27235) for `method` + `url`: `u`,
/// `method`, a nonce (so rapid identical requests don't trip replay guards)
/// and, with a body, its `payload` hash. Mints and relay requests sign it.
pub(crate) fn nip98_event(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<EventBuilder, CliError> {
    let tag = |parts: &[&str]| {
        Tag::parse(parts.iter().copied()).map_err(|e| CliError::Other(format!("tag error: {e}")))
    };
    let mut tags = vec![
        tag(&["u", url])?,
        tag(&["method", method])?,
        tag(&["nonce", &uuid::Uuid::new_v4().to_string()])?,
    ];
    if let Some(body) = body {
        tags.push(tag(&["payload", &hex::encode(Sha256::digest(body))])?);
    }
    Ok(EventBuilder::new(Kind::HttpAuth, "").tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAY: &str = "https://relay.example.com/";

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
        for port_80 in [
            "wss://relay.example.com:80",
            "https://relay.example.com:80/x",
            "http://relay.example.com:80",
            "ws://user@relay.example.com:80",
            "http://[::1]:80",
            "http://relay.example.com:080",
        ] {
            assert!(
                matches!(canonical_relay_url(port_80), Err(CliError::Usage(_))),
                "{port_80}"
            );
        }
        assert!(canonical_relay_url("http://[::80]").is_ok());
        assert!(canonical_relay_url("http://relay.example.com:8080").is_ok());
    }

    #[test]
    fn retry_after_takes_seconds_or_an_http_date() {
        let now = 784_111_777; // Sun, 06 Nov 1994 08:49:37 GMT
        assert_eq!(parse_retry_after(" 7 ", now), Some(Duration::from_secs(7)));
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:50:07 GMT", now),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:00 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after("Sunday, 06-Nov-94 08:50:07 GMT", now),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after("Sun Nov  6 08:50:07 1994", now),
            Some(Duration::from_secs(30))
        );
        assert_eq!(parse_retry_after("soon", now), None);
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
            assert!(AdapterSession::new(ok, "token", RELAY).is_ok(), "{ok}");
        }
        for bad in [
            "http://adapter.example",
            "ftp://adapter.example",
            "https://adapter.example/?q=1",
            "https://adapter.example/#f",
            "https://user:pw@adapter.example",
            "adapter.example",
        ] {
            assert!(AdapterSession::new(bad, "token", RELAY).is_err(), "{bad}");
        }
        assert!(AdapterSession::new("https://adapter.example", " ", RELAY).is_err());
        assert!(AdapterSession::new("https://adapter.example", "a\nb", RELAY).is_err());
        let session = AdapterSession::new("https://adapter.example/base/", "token", RELAY).unwrap();
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

    use super::test_support::{self, count, raw_adapter};

    const TOKEN: &str = "adapter-session-token";

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

    /// A realistic NIP-FI assertion for `pubkey` expiring at `expires_at`.
    fn issued(pubkey: &str, expires_at: u64) -> (StatusCode, String) {
        issued_as(
            pubkey,
            expires_at,
            &test_support::assertion(pubkey, expires_at),
        )
    }

    fn issued_as(pubkey: &str, expires_at: u64, assertion: &str) -> (StatusCode, String) {
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
        let (base, seen) =
            adapter(|req| issued(req["nostr_pubkey"].as_str().unwrap(), unix_now() + 300)).await;
        let keys = Keys::generate();
        let credentials = keys.clone();
        let session = AdapterSession::new(&base, TOKEN, RELAY).unwrap();

        let first = session.assertion(&credentials).await.unwrap();
        assert!(first
            .as_deref()
            .and_then(crate::cached_assertion::jws_exp)
            .is_some());
        for _ in 0..2 {
            let assertion = session.assertion(&credentials).await.unwrap();
            assert_eq!(assertion, first);
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
    }

    #[tokio::test]
    async fn an_assertion_near_expiry_is_reminted() {
        // A 2 s lifetime refreshes at half-life, so the second call re-mints.
        let (base, seen) =
            adapter(|req| issued(req["nostr_pubkey"].as_str().unwrap(), unix_now() + 2)).await;
        let credentials = Keys::generate();
        let session = AdapterSession::new(&base, TOKEN, RELAY).unwrap();
        session.assertion(&credentials).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        session.assertion(&credentials).await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn short_and_long_assertions_refresh_at_the_earlier_point() {
        // 20 s refreshes at half-life (10 s); 300 s at half-life (150 s),
        // earlier than 30 s before expiry.
        for (lifetime, refresh) in [(20, 10), (300, 150)] {
            let (base, seen) = adapter(move |req| {
                issued(req["nostr_pubkey"].as_str().unwrap(), unix_now() + lifetime)
            })
            .await;
            let keys = Keys::generate();
            let session = AdapterSession::new(&base, TOKEN, RELAY).unwrap();
            session.assertion(&keys).await.unwrap();
            crate::cached_assertion::test_clock::advance(refresh - 1);
            session.assertion(&keys).await.unwrap();
            assert_eq!(seen.lock().unwrap().len(), 1, "lifetime {lifetime}");
            crate::cached_assertion::test_clock::advance(1);
            session.assertion(&keys).await.unwrap();
            assert_eq!(seen.lock().unwrap().len(), 2, "lifetime {lifetime}");
        }
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
                "HTTP 429",
            ),
            (
                "internal error",
                Box::new(|_| (StatusCode::INTERNAL_SERVER_ERROR, String::new())),
                Some(500),
                "HTTP 500",
            ),
            (
                "not implemented",
                Box::new(|_| (StatusCode::NOT_IMPLEMENTED, "<html>".into())),
                Some(501),
                "HTTP 501",
            ),
            (
                "issuance_unavailable",
                Box::new(|_| denied(StatusCode::SERVICE_UNAVAILABLE, "issuance_unavailable")),
                Some(503),
                "HTTP 503",
            ),
            (
                "exp disagrees with expires_at",
                Box::new(|pk| {
                    let exp = unix_now() + 300;
                    issued_as(pk, exp, &test_support::assertion(pk, exp + 60))
                }),
                None,
                "does not match expires_at",
            ),
            (
                "empty signature",
                Box::new(|pk| {
                    let exp = unix_now() + 300;
                    let jws = test_support::assertion(pk, exp);
                    let unsigned = &jws[..=jws.rfind('.').unwrap()];
                    issued_as(pk, exp, unsigned)
                }),
                None,
                "compact JWS",
            ),
            (
                "four segments",
                Box::new(|pk| {
                    let exp = unix_now() + 300;
                    issued_as(
                        pk,
                        exp,
                        &format!("{}.c2ln", test_support::assertion(pk, exp)),
                    )
                }),
                None,
                "compact JWS",
            ),
            (
                "header is not JSON",
                Box::new(|pk| issued_as(pk, unix_now() + 300, "bm90LWpzb24.e30.c2ln")),
                None,
                "compact JWS",
            ),
            (
                "wrong pubkey",
                Box::new(move |_| issued(&other, unix_now() + 300)),
                None,
                "different key",
            ),
            (
                "expired",
                Box::new(|pk| issued(pk, unix_now() - 1)),
                None,
                "expired",
            ),
            (
                "not a compact JWS",
                Box::new(|pk| issued_as(pk, unix_now() + 300, "not-a-jws")),
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
            let credentials = Keys::generate();
            let session = AdapterSession::new(&base, TOKEN, RELAY).unwrap();
            let err = session.assertion(&credentials).await.unwrap_err();
            let message = err.to_string();
            assert!(message.contains(expected), "{name}: {message}");
            assert!(!message.contains(TOKEN), "{name}: token leaked: {message}");
            // Transient answers are retried, then fail as a retryable
            // adapter error; refusals are a single attempt.
            let attempts = match (retry_status, &err) {
                (Some(_), CliError::Adapter(_)) => MINT_ATTEMPTS,
                (None, CliError::Auth(_)) => 1,
                _ => panic!("{name}: unexpected error class {err:?}"),
            };
            assert_eq!(
                crate::error::is_retryable_error(&err),
                retry_status.is_some(),
                "{name}"
            );
            assert_eq!(seen.lock().unwrap().len(), attempts, "{name}: attempts");
            // A failed mint is not cached: the next call asks again.
            let _ = session.assertion(&credentials).await;
            assert_eq!(
                seen.lock().unwrap().len(),
                2 * attempts,
                "{name}: failure not cached"
            );
        }
    }

    /// NIP-FA: the token and proof never follow a redirect.
    #[tokio::test]
    async fn a_redirect_is_refused_and_never_followed() {
        let (target, target_seen) =
            adapter(|req| issued(req["nostr_pubkey"].as_str().unwrap(), unix_now() + 300)).await;
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

        let session = AdapterSession::new(&base, TOKEN, RELAY).unwrap();
        let err = session.assertion(&Keys::generate()).await.unwrap_err();
        assert!(
            matches!(&err, CliError::Auth(m) if m.contains("HTTP 307")),
            "{err:?}"
        );
        assert!(target_seen.lock().unwrap().is_empty(), "redirect followed");
    }

    #[tokio::test]
    async fn an_unreachable_adapter_is_retried_then_a_retryable_adapter_error_without_the_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let session = AdapterSession::new(&base, TOKEN, RELAY).unwrap();
        let err = session.assertion(&Keys::generate()).await.unwrap_err();
        assert!(
            matches!(&err, CliError::Adapter(m) if m.contains("3 attempts")),
            "{err:?}"
        );
        assert!(crate::error::is_retryable_error(&err));
        assert!(!format!("{err:?}").contains(TOKEN));
    }

    /// A refusal whose body is cut off is still a refusal: one attempt, not
    /// retryable. A cut-off 200 is a transport failure and is retried.
    #[tokio::test]
    async fn a_truncated_body_keeps_the_status_meaning() {
        let credentials = Keys::generate();
        let (base, seen) = raw_adapter(test_support::TRUNCATED_403).await;
        let err = AdapterSession::new(&base, TOKEN, RELAY)
            .unwrap()
            .assertion(&credentials)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CliError::Auth(m) if m.contains("HTTP 403")),
            "{err:?}"
        );
        assert!(!crate::error::is_retryable_error(&err));
        assert_eq!(count(&seen), 1, "a refusal is not retried");

        let (base, seen) = raw_adapter(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 200\r\nconnection: close\r\n\r\n{\"assert",
        )
        .await;
        let err = AdapterSession::new(&base, TOKEN, RELAY)
            .unwrap()
            .assertion(&credentials)
            .await
            .unwrap_err();
        assert!(matches!(&err, CliError::Adapter(_)), "{err:?}");
        assert_eq!(count(&seen), MINT_ATTEMPTS);
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let session =
            AdapterSession::new("https://adapter.example", "s3cr3t-token", RELAY).unwrap();
        assert!(!format!("{session:?}").contains("s3cr3t-token"));
    }
}

/// Fixtures shared with the relay-client tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A NIP-FI human assertion shaped like a real adapter's: ES256 header
    /// with `typ: nip-fi+jwt`, the NIP-FI claims, and a 64-byte signature.
    /// The CLI never verifies the signature; the relay does.
    pub(crate) fn assertion(pubkey: &str, exp: u64) -> String {
        let segment = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!(
            "{}.{}.{}",
            segment(serde_json::json!({"alg": "ES256", "typ": "nip-fi+jwt", "kid": "k1"})),
            segment(serde_json::json!({
                "iss": "https://adapter.example",
                "sub": "user-1",
                "aud": "https://relay.example.com",
                "iat": exp.saturating_sub(300),
                "exp": exp,
                "jti": "a1b2c3",
                "nostr_pubkey": pubkey,
            })),
            URL_SAFE_NO_PAD.encode([7u8; 64]),
        )
    }

    /// An adapter that answers every connection with the raw HTTP bytes
    /// `response` and then closes it, so a short body is a truncated one.
    /// Returns the base URL and the number of requests it answered.
    pub(crate) async fn raw_adapter(response: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                seen.fetch_add(1, Ordering::SeqCst);
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                // Read the head and the (small) body before answering.
                while let Ok(n) = socket.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let length = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= head_end + 4 + length {
                            break;
                        }
                    }
                }
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (base, count)
    }

    /// Raw responses for [`raw_adapter`].
    pub(crate) const HTTP_500: &str =
        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    pub(crate) const HTTP_503: &str =
        "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    /// A 403 whose body is cut off after a few bytes.
    pub(crate) const TRUNCATED_403: &str = "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: 200\r\nconnection: close\r\n\r\n{\"error\":\"auth";

    pub(crate) fn count(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }
}
