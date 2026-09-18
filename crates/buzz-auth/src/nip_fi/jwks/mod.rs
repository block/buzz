//! JWKS discovery, snapshot caching, and the production [`IssuerKeySource`]
//! implementation for federated-assertion verification.
//!
//! ## Design invariants
//!
//! - **Issuer binding is sealed.** [`ProductionJwksSource`] builds each
//!   [`AssertionKeySet`] using the crate-private constructor and stores it
//!   keyed by the exact `iss` it authenticates. A caller cannot relabel one
//!   issuer's JWKS as another's — the cross-issuer bypass is closed at both
//!   the request seam (the verifier re-checks `iss`) and here.
//!
//! - **No stale-key fallback.** On fetch error the source returns the current
//!   snapshot if it is within its hard deadline, or `None`. It never serves
//!   an expired snapshot. [FI-TRACE-JWKS-REMOVE]
//!
//! - **Bounded resource acquisition.** HTTP response streaming stops at
//!   [`MAX_JWKS_RESPONSE_BYTES`] + 1 byte before any allocation for parsing.
//!   Key count is bounded by [`super::config::MAX_JWKS_KEYS`] inside
//!   [`AssertionKeySet::new`].
//!
//! - **Coalesced refresh.** A single in-flight refresh per issuer prevents
//!   thundering-herd. Concurrent callers observe the snapshot just after the
//!   racing refresh commits.
//!
//! - **No secrets or key material in errors or logs.** [`JwksFetchError`]
//!   carries only non-sensitive diagnostic codes.

use super::config::MAX_JWKS_KEYS;
use super::verifier::{AssertionKeySet, IssuerKeySource};
use buzz_core::network::is_not_global_unicast;
use chrono::{DateTime, Duration, Utc};
use futures_util::StreamExt as _;
use jsonwebtoken::jwk::JwkSet;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;
use url::Url;

/// Maximum HTTP response body for a JWKS endpoint. Streaming stops at this
/// limit before any deserialization, preventing OOM from a malicious server.
pub const MAX_JWKS_RESPONSE_BYTES: usize = 512 * 1024; // 512 KiB

/// Hard upper bound on JWKS timing fields. Values above this are rejected at
/// config construction to prevent `u64`→`i64` conversion overflow and Chrono
/// range panics when computing snapshot deadlines.
pub const MAX_JWKS_TIMING_SECONDS: u64 = 365 * 24 * 3600; // 1 year

/// Hard deadline for the complete JWKS fetch: hostname resolution, connect,
/// headers, and body streaming combined. Applied via `tokio::time::timeout`
/// so a stalled resolver cannot keep `fetch_jwks` pending indefinitely.
pub const JWKS_REQUEST_TIMEOUT_SECS: u64 = 10;

/// Validate that a JWKS URI is safe to fetch: HTTPS scheme, no credentials,
/// no fragment, and the host (if a bare IP) is not private/reserved.
/// Hostname targets are resolved and checked at every fetch in `fetch_jwks`
/// to prevent DNS rebinding — this check catches the most common
/// misconfiguration at construction time.
pub fn validate_jwks_uri(uri: &str) -> Result<(), JwksFetchError> {
    let parsed = Url::parse(uri).map_err(|_| JwksFetchError::InvalidUri)?;
    if parsed.scheme() != "https" {
        return Err(JwksFetchError::InvalidUri);
    }
    // Credentials in the URI are never legitimate for a public JWKS endpoint
    // and would be forwarded to the server, leaking material in logs.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(JwksFetchError::InvalidUri);
    }
    // Fragments are client-side only; their presence indicates a misconfigured URI.
    if parsed.fragment().is_some() {
        return Err(JwksFetchError::InvalidUri);
    }
    // Reject bare private/reserved IP targets at construction time.
    if let Some(url::Host::Ipv4(addr)) = parsed.host() {
        if is_not_global_unicast(&std::net::IpAddr::V4(addr)) {
            return Err(JwksFetchError::InvalidUri);
        }
    }
    if let Some(url::Host::Ipv6(addr)) = parsed.host() {
        if is_not_global_unicast(&std::net::IpAddr::V6(addr)) {
            return Err(JwksFetchError::InvalidUri);
        }
    }
    Ok(())
}

/// The authenticated key-source contract owned by one [`IssuerPolicy`].
///
/// Encodes the three deployment-configured fields whose change alters which
/// keys the runtime trusts and how long it trusts them:
///
/// - `jwks_uri` — selects the authenticated key source; a different endpoint
///   may serve different keys even for the same issuer.
/// - `refresh_interval_seconds` — defines bounded refresh behavior; a longer
///   interval allows stale keys to persist longer.
/// - `key_snapshot_hard_deadline_seconds` — defines the source's accepted
///   time rule; the per-snapshot absolute deadline that flows into every
///   sealed [`VerifiedAssertion`][crate::nip_fi::VerifiedAssertion]'s
///   revalidation dependencies derives from this.
///
/// This type is the single source of truth for these fields. `IssuerJwksConfig`
/// is built from it (pairing it with the bare issuer string) rather than
/// independently restating the same values. Having both types carry independent
/// copies of these fields would let them drift silently; startup validation
/// detects any mismatch that a compatibility path temporarily introduces.
///
/// All three fields are validated at construction — an invalid value is caught
/// at configuration time, not at first token verification.
///
/// ## Why these fields are contract, not mutable state
///
/// Per the settled NIP-FI spec ("Policy identity and snapshots"):
/// `assertion_policy_id` covers "authenticated key/status-source contracts"
/// and "time rules". Key additions/removals (JWKS rotation) and per-snapshot
/// deadlines remain *revalidation dependencies* — they change per-token state
/// without changing the contract. These three fields define what the contract
/// *is*; JWKS content is what the contract currently *says*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwksSourceContract {
    /// Validated JWKS endpoint URI normalized to its canonical `Url` serialization.
    /// `Url::to_string()` lowercases the scheme and host, removes the default
    /// HTTPS port, and resolves dot-segments — so equivalent URI spellings hash
    /// identically. Validated at construction; only stored after parse succeeds.
    jwks_uri: String,
    /// Positive, ≤ [`MAX_JWKS_TIMING_SECONDS`], strictly less than
    /// `key_snapshot_hard_deadline_seconds`.
    refresh_interval_seconds: u64,
    /// Positive, ≤ [`MAX_JWKS_TIMING_SECONDS`], strictly greater than
    /// `refresh_interval_seconds`.
    key_snapshot_hard_deadline_seconds: u64,
}

impl JwksSourceContract {
    /// Validate and seal the three JWKS source fields.
    ///
    /// Rejects:
    /// - `jwks_uri` that fails [`validate_jwks_uri`]
    /// - zero `refresh_interval_seconds` or `key_snapshot_hard_deadline_seconds`
    /// - `refresh_interval_seconds >= key_snapshot_hard_deadline_seconds` (the
    ///   hard deadline must be strictly greater so a snapshot is fresh for at
    ///   least one refresh cycle)
    /// - either timing field exceeding [`MAX_JWKS_TIMING_SECONDS`]
    pub fn new(
        jwks_uri: String,
        refresh_interval_seconds: u64,
        key_snapshot_hard_deadline_seconds: u64,
    ) -> Option<Self> {
        if refresh_interval_seconds == 0
            || key_snapshot_hard_deadline_seconds == 0
            || key_snapshot_hard_deadline_seconds <= refresh_interval_seconds
            || refresh_interval_seconds > MAX_JWKS_TIMING_SECONDS
            || key_snapshot_hard_deadline_seconds > MAX_JWKS_TIMING_SECONDS
        {
            return None;
        }
        // Parse once, reject via validate_jwks_uri's rule-set, then store the
        // canonical serialization produced by `Url::to_string()`. The `url`
        // crate lowercases scheme and host, removes the default HTTPS port,
        // and resolves dot-segments — guaranteeing that equivalent URI spellings
        // (e.g. uppercase host, explicit `:443`, `.///../`) produce an identical
        // stored string and therefore an identical `AssertionPolicyId` hash.
        let canonical_uri = match Url::parse(&jwks_uri) {
            Ok(parsed) => parsed.to_string(),
            Err(_) => return None,
        };
        // Re-validate on the canonical form so that any normalisation that
        // would introduce a forbidden form (e.g. port stripping that leaves
        // a bare-IP host) is caught here rather than silently stored.
        if validate_jwks_uri(&canonical_uri).is_err() {
            return None;
        }
        Some(Self {
            jwks_uri: canonical_uri,
            refresh_interval_seconds,
            key_snapshot_hard_deadline_seconds,
        })
    }

    /// The validated JWKS endpoint URI.
    pub fn jwks_uri(&self) -> &str {
        &self.jwks_uri
    }

    /// Seconds between successive JWKS refreshes.
    pub const fn refresh_interval_seconds(&self) -> u64 {
        self.refresh_interval_seconds
    }

    /// Hard upper bound (from fetch time) on how long a snapshot may be served.
    pub const fn key_snapshot_hard_deadline_seconds(&self) -> u64 {
        self.key_snapshot_hard_deadline_seconds
    }
}

/// Resolve `host:port` to IP addresses and reject if any are private/reserved.
///
/// Returns the first safe address for DNS pinning. Blocks on the OS resolver
/// via `spawn_blocking` to avoid blocking the async runtime.
///
/// Uses the `(host, port)` tuple form of `ToSocketAddrs` — not
/// `format!("{host}:{port}")` — so IPv6 literal hosts (returned without
/// brackets by `Url::host_str()`) are handled correctly without socket-address
/// ambiguity.
///
/// Rejecting *any* resolved address (not just the first) closes split-horizon
/// DNS attacks: if an attacker can cause one DNS record to resolve to a private
/// address, the entire request is blocked even when other records are public.
pub(crate) async fn resolve_and_check_ssrf(
    host: &str,
    port: u16,
) -> Result<std::net::IpAddr, JwksFetchError> {
    // Fast path: if the host is already a parsed IP literal, skip the resolver.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_not_global_unicast(&ip) {
            return Err(JwksFetchError::InvalidUri);
        }
        return Ok(ip);
    }

    // Hostname path: use the tuple form to avoid IPv6-bracket ambiguity.
    let host_owned = host.to_owned();
    let addrs: Vec<std::net::IpAddr> = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        (host_owned.as_str(), port)
            .to_socket_addrs()
            .map(|iter| iter.map(|sa| sa.ip()).collect::<Vec<_>>())
    })
    .await
    .map_err(|_| JwksFetchError::NetworkError)?
    .map_err(|_| JwksFetchError::NetworkError)?;

    if addrs.is_empty() {
        return Err(JwksFetchError::NetworkError);
    }
    for ip in &addrs {
        if is_not_global_unicast(ip) {
            return Err(JwksFetchError::InvalidUri);
        }
    }
    Ok(addrs[0])
}

#[derive(Clone)]
struct CachedSnapshot {
    key_set: AssertionKeySet,
    fetched_at: DateTime<Utc>,
    hard_deadline: DateTime<Utc>,
    /// SHA-256 of the raw JWKS bytes. Suppresses generation advances when the
    /// document is unchanged between refreshes. [FI-TRACE-JWKS-ADD/REMOVE]
    content_digest: [u8; 32],
}

struct IssuerState {
    snapshot: Option<CachedSnapshot>,
    /// Advances only when `content_digest` changes; never wraps (saturating).
    generation_counter: u64,
    /// Owned permit for in-flight refresh. Held across the complete fetch +
    /// state commit; dropped automatically if the caller future is cancelled.
    /// `try_lock_owned()` succeeds iff no refresh is in progress.
    refresh_permit: Arc<tokio::sync::Mutex<()>>,
}

impl IssuerState {
    fn new() -> Self {
        Self {
            snapshot: None,
            generation_counter: 0,
            refresh_permit: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// Per-issuer JWKS endpoint configuration. Pairs the exact `iss` value with
/// the policy-owned [`JwksSourceContract`] that was already validated at
/// [`IssuerPolicy`][super::config::IssuerPolicy] construction.
///
/// `IssuerJwksConfig` is the single combination of issuer string and contract
/// that `ProductionJwksSource` operates on. Because the contract fields are
/// sealed inside [`JwksSourceContract`] and validated there, this type carries
/// no independent copies of those values — startup validation enforces that the
/// contract embedded here matches the one carried by the corresponding policy.
#[derive(Debug, Clone)]
pub struct IssuerJwksConfig {
    /// The exact `iss` value this config authenticates. Must match the
    /// corresponding [`IssuerPolicy`][super::config::IssuerPolicy] exactly.
    pub issuer: String,
    /// The validated key-source contract owned by the matching policy. Carries
    /// the JWKS URI, refresh interval, and hard deadline — validated at
    /// [`JwksSourceContract::new`], not re-validated here.
    pub contract: JwksSourceContract,
}

/// Reason a JWKS fetch or parse operation failed. No key material, issuer
/// URLs, or raw response content appear in these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JwksFetchError {
    /// Non-HTTPS scheme, embedded credentials, fragment, bare
    /// private/reserved IP host, or DNS resolved to a private/reserved address.
    #[error("JWKS URI failed safety validation")]
    InvalidUri,
    /// Response body exceeded [`MAX_JWKS_RESPONSE_BYTES`].
    #[error("JWKS response exceeded size limit")]
    ResponseTooLarge,
    /// Network failure, TLS error, request timeout, or non-2xx status.
    #[error("JWKS HTTP request failed")]
    NetworkError,
    /// Response body was not parseable as a JWK Set.
    #[error("JWKS response was not parseable")]
    ParseError,
    /// Parsed key set was empty or exceeded [`super::config::MAX_JWKS_KEYS`].
    #[error("JWKS key set bounds violation")]
    KeyCountBoundsViolation,
}

/// Sealed injection seam for JWKS HTTP fetching. Only types inside `buzz_auth`
/// may implement it — external types cannot name the private supertrait.
///
/// Implementations MUST:
/// - validate the URI (scheme, credentials, fragment, bare private-IP host)
///   before any I/O;
/// - resolve hostname targets and reject any private/reserved resolved address;
/// - deny redirects (3xx responses rejected as `NetworkError`);
/// - enforce a finite per-fetch deadline covering resolution, connect, headers,
///   and body streaming — the entire operation must be bounded;
/// - enforce [`MAX_JWKS_RESPONSE_BYTES`] via incremental streaming;
/// - reject non-2xx responses.
pub trait JwksFetcher: super::verifier::sealed::Sealed + Send + Sync + 'static {
    /// Fetch and return the raw JSON body from the given JWKS URI.
    fn fetch_jwks<'a>(
        &'a self,
        uri: &'a str,
    ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a;
}

/// Production [`JwksFetcher`] backed by `reqwest`. Each call to `fetch_jwks`
/// builds a dedicated pinned client — no shared connection state between fetches.
///
/// Per-fetch boundary enforcement:
/// - hostname DNS is resolved and every address checked against
///   `buzz_core::network::is_not_global_unicast` before the request is sent;
/// - the request is pinned to the validated address to prevent DNS rebinding
///   TOCTOU (the OS resolver is called once per fetch, not once per URL);
/// - the complete operation (resolution, connect, headers, body streaming) is
///   bounded by [`JWKS_REQUEST_TIMEOUT_SECS`] via `tokio::time::timeout`;
/// - 3xx responses are rejected as `NetworkError` — redirects are never followed;
/// - the body is streamed incrementally and stopped at
///   [`MAX_JWKS_RESPONSE_BYTES`] + 1.
#[derive(Clone, Debug)]
pub struct HttpJwksFetcher;

impl HttpJwksFetcher {
    /// Builds a new fetcher. Security invariants are enforced per-request in
    /// `fetch_jwks` — each call constructs a dedicated pinned client.
    pub fn new() -> Self {
        Self
    }
}

impl Default for HttpJwksFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl super::verifier::sealed::Sealed for HttpJwksFetcher {}

impl JwksFetcher for HttpJwksFetcher {
    async fn fetch_jwks<'a>(&'a self, uri: &'a str) -> Result<String, JwksFetchError> {
        with_deadline(
            fetch_jwks_inner(uri),
            std::time::Duration::from_secs(JWKS_REQUEST_TIMEOUT_SECS),
        )
        .await
    }
}

/// Bound `fut` with a hard `tokio::time::timeout`. Elapsed maps to
/// `NetworkError`. Production passes `fetch_jwks_inner(uri)`; tests pass
/// `std::future::pending()` to verify the seam deterministically.
async fn with_deadline<F>(fut: F, timeout: std::time::Duration) -> Result<String, JwksFetchError>
where
    F: std::future::Future<Output = Result<String, JwksFetchError>>,
{
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| JwksFetchError::NetworkError)?
}

/// Extract the bare host string and port from a validated JWKS URI.
///
/// The host is extracted via the typed `Url::host()` accessor, **not**
/// `host_str()`. `host_str()` returns IPv6 literals with brackets (e.g.
/// `[2606:4700::1]`), which breaks `IpAddr::parse`: brackets are not valid,
/// so the fast path in `resolve_and_check_ssrf` would fail and fall through
/// to the DNS path, which may attempt to resolve `[2606:4700::1]` as a
/// hostname instead of an IP literal.
///
/// The extracted bare host string is also the correct input form for
/// `reqwest::ClientBuilder::resolve(host, addr)`, whose key must match the
/// URL authority form (bare, without brackets for IPv6). Whether the
/// connector-level pin behaves as expected under mutation is a runtime
/// boundary concern; this function's contract is that it produces the bare
/// form required as input.
///
/// This function is `pub(crate)` so tests can assert the extracted host string
/// directly and confirm the mutation (restoring `host_str()`) turns the
/// equivalence oracle red without making a live network request.
///
/// ## Mutation oracle
/// Restoring `Some(url::Host::Ipv6(addr)) => format!("[{}]", addr)` (the
/// `host_str()` form) causes the IPv6 host extraction test to fail: the
/// returned string carries brackets, `IpAddr::parse` rejects it, and the
/// extracted host no longer matches the bare URL authority form.
pub(crate) fn extract_url_host_and_port(uri: &str) -> Result<(String, u16), JwksFetchError> {
    let parsed = Url::parse(uri).map_err(|_| JwksFetchError::InvalidUri)?;
    let host = match parsed.host() {
        Some(url::Host::Ipv4(addr)) => addr.to_string(),
        // MUST use the typed accessor — `host_str()` returns `[2606:4700::1]`
        // (with brackets) for IPv6 literals, which breaks IpAddr::parse.
        Some(url::Host::Ipv6(addr)) => addr.to_string(),
        Some(url::Host::Domain(d)) => d.to_owned(),
        None => return Err(JwksFetchError::InvalidUri),
    };
    let port = parsed.port_or_known_default().unwrap_or(443);
    Ok((host, port))
}

/// Inner fetch logic. Called only by `HttpJwksFetcher::fetch_jwks` via `with_deadline`.
async fn fetch_jwks_inner(uri: &str) -> Result<String, JwksFetchError> {
    // Full URI validation first — scheme, credentials, fragment, bare
    // private-IP host. This enforces the JwksFetcher contract for direct
    // callers of HttpJwksFetcher regardless of whether ProductionJwksSource
    // pre-validated the URI.
    validate_jwks_uri(uri)?;

    let (host, port) = extract_url_host_and_port(uri)?;

    // Resolve and check every IP before sending. Pins DNS to the validated
    // address to prevent rebinding TOCTOU between check and connect.
    let safe_ip = resolve_and_check_ssrf(&host, port).await?;

    // Build a per-request client that:
    // - denies redirects (a 3xx to an internal host bypasses the URI check);
    // - has no system proxy (proxy would resolve the original hostname itself);
    // - pins this request to the validated IP.
    let pinned_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve(&host, std::net::SocketAddr::new(safe_ip, port))
        .build()
        .map_err(|_| JwksFetchError::NetworkError)?;

    let response = pinned_client
        .get(uri)
        .send()
        .await
        .map_err(|_| JwksFetchError::NetworkError)?;

    // Reject non-2xx. A 3xx here means our no-redirect policy was somehow
    // bypassed — treat as a network error.
    if !response.status().is_success() {
        return Err(JwksFetchError::NetworkError);
    }

    // Early-exit on Content-Length before streaming. A lying or absent
    // Content-Length is caught by the incremental counter below.
    if let Some(content_length) = response.content_length() {
        if content_length as usize > MAX_JWKS_RESPONSE_BYTES {
            return Err(JwksFetchError::ResponseTooLarge);
        }
    }

    // Stream incrementally; stop at MAX_JWKS_RESPONSE_BYTES + 1 so we
    // never buffer more than the limit before rejecting.
    let mut body = Vec::with_capacity(MAX_JWKS_RESPONSE_BYTES.min(64 * 1024));
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| JwksFetchError::NetworkError)?;
        if body.len().saturating_add(chunk.len()) > MAX_JWKS_RESPONSE_BYTES {
            return Err(JwksFetchError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body).map_err(|_| JwksFetchError::ParseError)
}

fn parse_and_bound_jwks(body: &str) -> Result<JwkSet, JwksFetchError> {
    let key_set: JwkSet = serde_json::from_str(body).map_err(|_| JwksFetchError::ParseError)?;
    if key_set.keys.is_empty() || key_set.keys.len() > MAX_JWKS_KEYS {
        return Err(JwksFetchError::KeyCountBoundsViolation);
    }
    Ok(key_set)
}

/// Multi-issuer JWKS cache that performs bounded periodic refresh and never
/// serves snapshots past their hard deadline.
///
/// Must be constructed at startup after
/// [`super::startup::validate_nip_fi_config`] passes. Shared across async
/// tasks via the inner `Arc<RwLock<…>>`.
///
/// ## Security
///
/// - Each issuer's JWKS is stored under its exact `iss` — no relabelling.
/// - Expired snapshots are purged on access; no stale-key fallback.
/// - Errors are logged with a stable code; no key material appears in logs.
pub struct ProductionJwksSource<F = HttpJwksFetcher> {
    configs: HashMap<String, IssuerJwksConfig>,
    states: Arc<RwLock<HashMap<String, Mutex<IssuerState>>>>,
    fetcher: Arc<F>,
    /// Clock used for `hard_deadline` computation and expiry checks. Always
    /// `Arc::new(Utc::now)` in production; tests supply a controlled clock.
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl<F: JwksFetcher> ProductionJwksSource<F> {
    /// Returns `None` when `configs` is empty or any two configs share the
    /// same `issuer` (duplicate issuers make trust configuration ambiguous).
    ///
    /// Contract fields (`jwks_uri`, `refresh_interval_seconds`,
    /// `key_snapshot_hard_deadline_seconds`) are pre-validated inside the
    /// embedded [`JwksSourceContract`] — no re-validation is performed here.
    pub fn new(configs: Vec<IssuerJwksConfig>, fetcher: F) -> Option<Self> {
        if configs.is_empty() {
            return None;
        }
        let mut config_map = HashMap::with_capacity(configs.len());
        let mut state_map = HashMap::with_capacity(configs.len());
        for c in configs {
            if config_map.contains_key(&c.issuer) {
                return None;
            }
            let issuer = c.issuer.clone();
            state_map.insert(issuer.clone(), Mutex::new(IssuerState::new()));
            config_map.insert(issuer, c);
        }
        Some(Self {
            configs: config_map,
            states: Arc::new(RwLock::new(state_map)),
            fetcher: Arc::new(fetcher),
            now_fn: Arc::new(Utc::now),
        })
    }

    /// **Test-only.** Construct with an injectable clock so tests can advance
    /// `now` past snapshot hard deadlines without wall-clock sleep.
    #[cfg(test)]
    pub(crate) fn new_with_clock(
        configs: Vec<IssuerJwksConfig>,
        fetcher: F,
        now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    ) -> Option<Self> {
        if configs.is_empty() {
            return None;
        }
        let mut config_map = HashMap::with_capacity(configs.len());
        let mut state_map = HashMap::with_capacity(configs.len());
        for c in configs {
            if config_map.contains_key(&c.issuer) {
                return None;
            }
            let issuer = c.issuer.clone();
            state_map.insert(issuer.clone(), Mutex::new(IssuerState::new()));
            config_map.insert(issuer, c);
        }
        Some(Self {
            configs: config_map,
            states: Arc::new(RwLock::new(state_map)),
            fetcher: Arc::new(fetcher),
            now_fn,
        })
    }

    async fn fetch_fresh(
        &self,
        issuer: &str,
        prev_digest: Option<[u8; 32]>,
        prev_generation: u64,
    ) -> Option<(CachedSnapshot, u64)> {
        let config = self.configs.get(issuer)?;
        let body = match self.fetcher.fetch_jwks(config.contract.jwks_uri()).await {
            Ok(b) => b,
            Err(err) => {
                warn!(error = %err, "nip-fi jwks fetch failed; will use cached snapshot if live");
                return None;
            }
        };

        let jwks = match parse_and_bound_jwks(&body) {
            Ok(k) => k,
            Err(err) => {
                warn!(error = %err, "nip-fi jwks parse failed; will use cached snapshot if live");
                return None;
            }
        };

        let content_digest: [u8; 32] = Sha256::digest(body.as_bytes()).into();

        // Advance only when the document changed so key-rotation events are
        // visible [FI-TRACE-JWKS-ADD/REMOVE] while identical refetches are
        // stable. Saturating add prevents wrap on the (unreachable) u64 ceiling.
        let generation = if Some(content_digest) == prev_digest {
            prev_generation
        } else {
            prev_generation.saturating_add(1).max(1)
        };

        let now = (self.now_fn)();
        // MAX_JWKS_TIMING_SECONDS ≤ ~31.5M < i64::MAX, so this conversion is
        // always safe for values that passed the bounds check in JwksSourceContract::new().
        let deadline_secs = i64::try_from(config.contract.key_snapshot_hard_deadline_seconds())
            .unwrap_or(i64::MAX / 2);
        let hard_deadline = now
            + Duration::try_seconds(deadline_secs)
                .unwrap_or_else(|| Duration::seconds(i64::MAX / 2));

        let key_set = AssertionKeySet::new(issuer.to_owned(), generation, jwks, hard_deadline)?;

        Some((
            CachedSnapshot {
                key_set,
                fetched_at: now,
                hard_deadline,
                content_digest,
            },
            generation,
        ))
    }

    /// Returns the cached snapshot for `issuer`, refreshing inline if stale.
    /// Returns `None` when no live snapshot is available and the fetch fails.
    ///
    /// Coalesces concurrent callers: a second call while a refresh is in
    /// flight returns the current snapshot immediately rather than starting a
    /// second fetch. The refresh permit is an RAII guard — if this future is
    /// cancelled while DNS, HTTP, or streaming is pending, the guard drops and
    /// the permit is released, so the next caller can start a new fetch.
    pub async fn get_snapshot(&self, issuer: &str) -> Option<AssertionKeySet> {
        let states = self.states.read().await;
        let state_mutex = states.get(issuer)?;
        let mut state = state_mutex.lock().await;

        let now = (self.now_fn)();
        let config = self.configs.get(issuer)?;

        if let Some(ref cached) = state.snapshot {
            if now >= cached.hard_deadline {
                state.snapshot = None;
            }
        }

        let needs_refresh = match state.snapshot {
            None => true,
            Some(ref cached) => {
                let age_secs = (now - cached.fetched_at).num_seconds().max(0) as u64;
                age_secs >= config.contract.refresh_interval_seconds()
            }
        };

        if !needs_refresh {
            return state.snapshot.as_ref().map(|c| c.key_set.clone());
        }

        // Try to acquire the per-issuer refresh permit. Failure means another
        // caller is already fetching; return the current snapshot rather than
        // starting a second fetch.
        let permit = match Arc::clone(&state.refresh_permit).try_lock_owned() {
            Ok(g) => g,
            Err(_) => return state.snapshot.as_ref().map(|c| c.key_set.clone()),
        };

        let prev_digest = state.snapshot.as_ref().map(|c| c.content_digest);
        let prev_generation = state.generation_counter;
        drop(state);
        drop(states);

        let fresh = self.fetch_fresh(issuer, prev_digest, prev_generation).await;

        // Re-acquire state to commit and release the permit atomically.
        let states = self.states.read().await;
        if let Some(state_mutex) = states.get(issuer) {
            let mut st = state_mutex.lock().await;
            if let Some((ref cached, new_generation)) = fresh {
                st.generation_counter = new_generation;
                st.snapshot = Some(cached.clone());
            }
            // Drop the permit only after the state commit is visible.
            drop(permit);
            let now2 = (self.now_fn)();
            return st
                .snapshot
                .as_ref()
                .filter(|c| now2 < c.hard_deadline)
                .map(|c| c.key_set.clone());
        }

        drop(permit);
        None
    }
}

impl<F: JwksFetcher> super::verifier::sealed::Sealed for ProductionJwksSource<F> {}

impl<F: JwksFetcher> IssuerKeySource for ProductionJwksSource<F> {
    /// Called per-request by the verifier after the cache has been warmed via
    /// [`get_snapshot`][Self::get_snapshot].
    ///
    /// Uses `try_read`/`try_lock` — safe to call from any async context.
    /// Fails closed (returns `None`) when the lock is momentarily held by an
    /// in-flight refresh, rather than blocking or panicking. [FI-INV-14]
    fn key_set(&self, issuer: &str) -> Option<AssertionKeySet> {
        let states = self.states.try_read().ok()?;
        let state_mutex = states.get(issuer)?;
        let state = state_mutex.try_lock().ok()?;
        let now = (self.now_fn)();
        state
            .snapshot
            .as_ref()
            .filter(|c| now < c.hard_deadline)
            .map(|c| c.key_set.clone())
    }
}

impl<F> std::fmt::Debug for ProductionJwksSource<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No issuer URIs or key material in debug output.
        write!(
            f,
            "ProductionJwksSource([REDACTED; {} issuers])",
            self.configs.len()
        )
    }
}

#[cfg(test)]
mod tests;
