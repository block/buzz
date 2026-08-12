//! Dynamic canonical JWKS and discovery runtime.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buzz_auth::{
    CanonicalFederatedAssertionVerifier, CanonicalVerifierKeySet, CanonicalVerifierPolicy,
    CanonicalVerifierPolicyId, ProofTransport, VerifiedFederatedAssertion, VerifierKeyGeneration,
    VerifierPolicyStamp,
};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use jsonwebtoken::jwk::JwkSet;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};
use url::Url;

use super::{JwksSourceConfig, RuntimeAuthorizationError};

const MAX_JWKS_KEYS: usize = 128;

/// Bounds for one refresh attempt and admitted snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JwksRefreshPolicy {
    /// Maximum discovery or JWKS response body.
    pub max_document_bytes: usize,
    /// Network deadline for each document.
    pub request_timeout: Duration,
    /// Fresh lifetime of one successfully fetched snapshot.
    pub fresh_lifetime: Duration,
}

impl JwksRefreshPolicy {
    /// Validate finite nonzero refresh bounds.
    pub fn new(
        max_document_bytes: usize,
        request_timeout: Duration,
        fresh_lifetime: Duration,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if max_document_bytes == 0
            || max_document_bytes > 4 * 1024 * 1024
            || request_timeout.is_zero()
            || request_timeout > Duration::from_secs(30)
            || fresh_lifetime.is_zero()
            || fresh_lifetime > Duration::from_secs(24 * 60 * 60)
        {
            return Err(RuntimeAuthorizationError::InvalidConfiguration);
        }
        Ok(Self {
            max_document_bytes,
            request_timeout,
            fresh_lifetime,
        })
    }
}

/// Bounded loader for a direct JWKS or issuer-matched discovery source.
#[async_trait]
pub trait JwksDocumentLoader: Send + Sync {
    /// Return exact JWKS bytes. Redirect, address, size, and discovery issuer
    /// checks belong to the loader and fail closed.
    async fn load(
        &self,
        source: &JwksSourceConfig,
        expected_issuer: &str,
        policy: JwksRefreshPolicy,
    ) -> Result<Vec<u8>, RuntimeAuthorizationError>;
}

/// Production HTTPS loader with redirects disabled and bounded streaming.
#[derive(Clone, Copy, Default)]
pub struct ReqwestJwksDocumentLoader;

impl ReqwestJwksDocumentLoader {
    /// Construct an HTTPS-only loader. Each request is additionally bounded by
    /// the refresh policy supplied to [`JwksDocumentLoader::load`].
    pub fn new() -> Result<Self, RuntimeAuthorizationError> {
        Ok(Self)
    }

    async fn fetch(
        &self,
        endpoint: &Url,
        policy: JwksRefreshPolicy,
    ) -> Result<Vec<u8>, RuntimeAuthorizationError> {
        let host = endpoint
            .host_str()
            .ok_or(RuntimeAuthorizationError::DependencyUnavailable)?;
        let addresses = resolve_public_https_endpoint(endpoint).await?;
        // Bind this request's connector to the exact addresses that passed the
        // public-address check. Reqwest must not perform a second DNS lookup.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
        let response =
            tokio::time::timeout(policy.request_timeout, client.get(endpoint.clone()).send())
                .await
                .map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?
                .map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(RuntimeAuthorizationError::DependencyUnavailable);
        }
        if response
            .content_length()
            .is_some_and(|length| length > policy.max_document_bytes as u64)
        {
            return Err(RuntimeAuthorizationError::DependencyUnavailable);
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
            if body.len().saturating_add(chunk.len()) > policy.max_document_bytes {
                return Err(RuntimeAuthorizationError::DependencyUnavailable);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    jwks_uri: String,
}

#[async_trait]
impl JwksDocumentLoader for ReqwestJwksDocumentLoader {
    async fn load(
        &self,
        source: &JwksSourceConfig,
        expected_issuer: &str,
        policy: JwksRefreshPolicy,
    ) -> Result<Vec<u8>, RuntimeAuthorizationError> {
        match source {
            JwksSourceConfig::JwksUri(endpoint) => self.fetch(endpoint, policy).await,
            JwksSourceConfig::DiscoveryUri(endpoint) => {
                let bytes = self.fetch(endpoint, policy).await?;
                let discovery: DiscoveryDocument = serde_json::from_slice(&bytes)
                    .map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
                if discovery.issuer != expected_issuer {
                    return Err(RuntimeAuthorizationError::DependencyUnavailable);
                }
                let jwks_uri = validated_https_url(&discovery.jwks_uri)?;
                self.fetch(&jwks_uri, policy).await
            }
        }
    }
}

/// Immutable current key snapshot and its hard freshness deadline.
#[derive(Clone)]
pub struct JwksSnapshot {
    keys: Arc<CanonicalVerifierKeySet>,
    stamp: VerifierPolicyStamp,
    fresh_until: DateTime<Utc>,
    discovery_fresh_until: Option<DateTime<Utc>>,
}

impl JwksSnapshot {
    /// Stable verifier policy plus separate rotating generation.
    pub const fn stamp(&self) -> VerifierPolicyStamp {
        self.stamp
    }

    /// Exclusive hard freshness deadline.
    pub const fn fresh_until(&self) -> DateTime<Utc> {
        self.fresh_until
    }

    /// Independent discovery deadline when the key URI was discovered.
    pub const fn discovery_fresh_until(&self) -> Option<DateTime<Utc>> {
        self.discovery_fresh_until
    }
}

impl std::fmt::Debug for JwksSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JwksSnapshot([REDACTED])")
    }
}

struct SnapshotState {
    snapshot: JwksSnapshot,
    document_digest: [u8; 32],
}

/// Single canonical verifier with a single-flight rotating key cache.
pub struct DynamicVerifier {
    verifier: CanonicalFederatedAssertionVerifier,
    policy_id: CanonicalVerifierPolicyId,
    issuer: String,
    source: JwksSourceConfig,
    refresh_policy: JwksRefreshPolicy,
    loader: Arc<dyn JwksDocumentLoader>,
    state: RwLock<Option<SnapshotState>>,
    published_generation: AtomicU64,
    published_fresh_until_millis: AtomicI64,
    refresh_lock: Mutex<()>,
}

impl DynamicVerifier {
    /// Compose stable policy and dynamic-key ownership. No network request is
    /// issued until [`Self::refresh`] is called by startup or recovery.
    pub fn new(
        policy: CanonicalVerifierPolicy,
        issuer: String,
        source: JwksSourceConfig,
        refresh_policy: JwksRefreshPolicy,
        loader: Arc<dyn JwksDocumentLoader>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if issuer.trim().is_empty() {
            return Err(RuntimeAuthorizationError::InvalidConfiguration);
        }
        let verifier = CanonicalFederatedAssertionVerifier::new(policy);
        let policy_id = verifier.policy_id();
        Ok(Self {
            verifier,
            policy_id,
            issuer,
            source,
            refresh_policy,
            loader,
            state: RwLock::new(None),
            published_generation: AtomicU64::new(0),
            published_fresh_until_millis: AtomicI64::new(0),
            refresh_lock: Mutex::new(()),
        })
    }

    /// Stable policy identity, unaffected by every key refresh.
    pub const fn policy_id(&self) -> CanonicalVerifierPolicyId {
        self.policy_id
    }

    /// Current policy/generation identity available to synchronous aggregate
    /// readiness checks. A zero generation means no snapshot is published.
    pub fn current_stamp(&self) -> Option<VerifierPolicyStamp> {
        let generation =
            VerifierKeyGeneration::new(self.published_generation.load(Ordering::Acquire))?;
        Some(VerifierPolicyStamp::new(self.policy_id, generation))
    }

    /// Synchronous readiness view paired with the published key generation.
    pub fn has_current_snapshot(&self, now: DateTime<Utc>) -> bool {
        self.current_stamp().is_some()
            && now.timestamp_millis() < self.published_fresh_until_millis.load(Ordering::Acquire)
    }

    /// Fetch, bound, parse, and atomically publish one key snapshot.
    /// Byte-identical documents retain their generation; any changed admitted
    /// document advances it and forces prepared evidence to reverify.
    pub async fn refresh(
        &self,
        now: DateTime<Utc>,
    ) -> Result<JwksSnapshot, RuntimeAuthorizationError> {
        let _singleflight = self.refresh_lock.lock().await;
        let bytes = self
            .loader
            .load(&self.source, &self.issuer, self.refresh_policy)
            .await?;
        if bytes.is_empty() || bytes.len() > self.refresh_policy.max_document_bytes {
            return Err(RuntimeAuthorizationError::DependencyUnavailable);
        }
        let parsed: JwkSet = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
        validate_key_set(&parsed)?;
        let document_digest: [u8; 32] = Sha256::digest(&bytes).into();
        let fresh_until = now
            .checked_add_signed(
                chrono::Duration::from_std(self.refresh_policy.fresh_lifetime)
                    .map_err(|_| RuntimeAuthorizationError::InvalidConfiguration)?,
            )
            .ok_or(RuntimeAuthorizationError::InvalidConfiguration)?;

        let current = self.state.read().await;
        let generation = match current.as_ref() {
            Some(state) if state.document_digest == document_digest => {
                state.snapshot.stamp.key_generation()
            }
            Some(state) => VerifierKeyGeneration::new(
                state
                    .snapshot
                    .stamp
                    .key_generation()
                    .get()
                    .checked_add(1)
                    .ok_or(RuntimeAuthorizationError::StaleVerifier)?,
            )
            .ok_or(RuntimeAuthorizationError::StaleVerifier)?,
            None => {
                VerifierKeyGeneration::new(1).ok_or(RuntimeAuthorizationError::StaleVerifier)?
            }
        };
        drop(current);

        let keys = Arc::new(CanonicalVerifierKeySet::new(generation, parsed));
        let snapshot = JwksSnapshot {
            keys,
            stamp: VerifierPolicyStamp::new(self.policy_id, generation),
            fresh_until,
            discovery_fresh_until: matches!(&self.source, JwksSourceConfig::DiscoveryUri(_))
                .then_some(fresh_until),
        };
        // Withdraw any installed old-generation readiness before the new key
        // snapshot can be observed. A new immutable runtime must bind the new
        // stamp explicitly.
        *self.state.write().await = Some(SnapshotState {
            snapshot: snapshot.clone(),
            document_digest,
        });
        self.published_fresh_until_millis
            .store(fresh_until.timestamp_millis(), Ordering::Release);
        self.published_generation
            .store(generation.get(), Ordering::Release);
        Ok(snapshot)
    }

    /// Return a current hard-fresh snapshot. Missing or exact-deadline state is
    /// unavailable; stale keys are never used as a silent fallback.
    pub async fn current(
        &self,
        now: DateTime<Utc>,
    ) -> Result<JwksSnapshot, RuntimeAuthorizationError> {
        let state = self.state.read().await;
        let snapshot = state
            .as_ref()
            .map(|state| state.snapshot.clone())
            .ok_or(RuntimeAuthorizationError::StaleVerifier)?;
        if now >= snapshot.fresh_until {
            return Err(RuntimeAuthorizationError::StaleVerifier);
        }
        if snapshot
            .discovery_fresh_until
            .is_some_and(|deadline| now >= deadline)
        {
            return Err(RuntimeAuthorizationError::StaleVerifier);
        }
        Ok(snapshot)
    }

    /// Whether prepared verifier evidence still names the exact current policy
    /// and generation at an authoritative time.
    pub async fn accepts_stamp(&self, stamp: VerifierPolicyStamp, now: DateTime<Utc>) -> bool {
        self.current(now)
            .await
            .is_ok_and(|snapshot| snapshot.stamp == stamp)
    }

    /// Verify one request against exactly one current generation snapshot.
    /// The token is borrowed for this call and is never retained by the cache.
    #[allow(clippy::too_many_arguments)]
    pub async fn verify(
        &self,
        token: &str,
        authorization_domain: CommunityId,
        transport: ProofTransport,
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
        now: DateTime<Utc>,
    ) -> Result<VerifiedFederatedAssertion, RuntimeAuthorizationError> {
        let snapshot = self.current(now).await?;
        self.verifier
            .verify(
                token,
                snapshot.keys.as_ref(),
                authorization_domain,
                transport,
                target_fingerprint,
                request_fingerprint,
                transport_context_fingerprint,
            )
            .map_err(|error| match error {
                buzz_auth::CanonicalVerifierError::Expired => {
                    RuntimeAuthorizationError::AssertionExpired
                }
                _ => RuntimeAuthorizationError::AssertionDenied,
            })
    }
}

impl std::fmt::Debug for DynamicVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DynamicVerifier([REDACTED])")
    }
}

fn validate_key_set(keys: &JwkSet) -> Result<(), RuntimeAuthorizationError> {
    if keys.keys.is_empty() || keys.keys.len() > MAX_JWKS_KEYS {
        return Err(RuntimeAuthorizationError::DependencyUnavailable);
    }
    let mut key_ids = HashSet::new();
    for key in &keys.keys {
        let key_id = key
            .common
            .key_id
            .as_deref()
            .filter(|key_id| !key_id.is_empty() && key_id.len() <= 512)
            .ok_or(RuntimeAuthorizationError::DependencyUnavailable)?;
        if !key_ids.insert(key_id) {
            return Err(RuntimeAuthorizationError::DependencyUnavailable);
        }
    }
    Ok(())
}

async fn resolve_public_https_endpoint(
    endpoint: &Url,
) -> Result<Vec<SocketAddr>, RuntimeAuthorizationError> {
    if endpoint.scheme() != "https"
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(RuntimeAuthorizationError::DependencyUnavailable);
    }
    let host = endpoint
        .host_str()
        .ok_or(RuntimeAuthorizationError::DependencyUnavailable)?;
    let port = endpoint
        .port_or_known_default()
        .ok_or(RuntimeAuthorizationError::DependencyUnavailable)?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
    let mut public = Vec::new();
    let mut unique = HashSet::new();
    for address in addresses {
        if buzz_core::network::is_private_ip(&address.ip()) {
            return Err(RuntimeAuthorizationError::DependencyUnavailable);
        }
        if unique.insert(address) {
            public.push(address);
        }
    }
    if public.is_empty() {
        return Err(RuntimeAuthorizationError::DependencyUnavailable);
    }
    Ok(public)
}

fn validated_https_url(raw: &str) -> Result<Url, RuntimeAuthorizationError> {
    let url = Url::parse(raw).map_err(|_| RuntimeAuthorizationError::DependencyUnavailable)?;
    if let Some(host) = url.host() {
        if let url::Host::Ipv4(ip) = host {
            if buzz_core::network::is_private_ip(&IpAddr::V4(ip)) {
                return Err(RuntimeAuthorizationError::DependencyUnavailable);
            }
        }
        if let url::Host::Ipv6(ip) = host {
            if buzz_core::network::is_private_ip(&IpAddr::V6(ip)) {
                return Err(RuntimeAuthorizationError::DependencyUnavailable);
            }
        }
    }
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(RuntimeAuthorizationError::DependencyUnavailable);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakeLoader {
        documents: Mutex<VecDeque<Vec<u8>>>,
    }

    #[async_trait]
    impl JwksDocumentLoader for FakeLoader {
        async fn load(
            &self,
            _source: &JwksSourceConfig,
            _expected_issuer: &str,
            _policy: JwksRefreshPolicy,
        ) -> Result<Vec<u8>, RuntimeAuthorizationError> {
            self.documents
                .lock()
                .await
                .pop_front()
                .ok_or(RuntimeAuthorizationError::DependencyUnavailable)
        }
    }

    fn verifier(documents: Vec<Vec<u8>>) -> DynamicVerifier {
        let policy = CanonicalVerifierPolicy::new(
            "https://issuer.example".to_owned(),
            "buzz".to_owned(),
            "sub".to_owned(),
            None,
            0,
            300,
        )
        .unwrap();
        DynamicVerifier::new(
            policy,
            "https://issuer.example".to_owned(),
            JwksSourceConfig::JwksUri(Url::parse("https://issuer.example/keys").unwrap()),
            JwksRefreshPolicy::new(64 * 1024, Duration::from_secs(2), Duration::from_secs(300))
                .unwrap(),
            Arc::new(FakeLoader {
                documents: Mutex::new(documents.into()),
            }),
        )
        .unwrap()
    }

    fn jwks(kid: &str) -> Vec<u8> {
        format!(r#"{{"keys":[{{"kty":"RSA","kid":"{kid}","n":"AQAB","e":"AQAB"}}]}}"#).into_bytes()
    }

    #[tokio::test]
    async fn identical_document_retains_generation_and_rotation_advances_it() {
        let verifier = verifier(vec![jwks("first"), jwks("first"), jwks("second")]);
        let now = Utc::now();
        let first = verifier.refresh(now).await.unwrap();
        let same = verifier.refresh(now).await.unwrap();
        let changed = verifier.refresh(now).await.unwrap();
        assert_eq!(
            first.stamp().key_generation(),
            same.stamp().key_generation()
        );
        assert_eq!(
            changed.stamp().key_generation().get(),
            first.stamp().key_generation().get() + 1
        );
        assert_eq!(first.stamp().policy_id(), changed.stamp().policy_id());
    }

    #[tokio::test]
    async fn stale_deadline_and_old_generation_fail_closed() {
        let verifier = verifier(vec![jwks("first"), jwks("second")]);
        let now = Utc::now();
        let first = verifier.refresh(now).await.unwrap();
        assert!(verifier.has_current_snapshot(now));
        assert!(verifier.accepts_stamp(first.stamp(), now).await);
        let second = verifier.refresh(now).await.unwrap();
        assert!(!verifier.accepts_stamp(first.stamp(), now).await);
        assert!(verifier.accepts_stamp(second.stamp(), now).await);
        assert!(!verifier.has_current_snapshot(second.fresh_until()));
        assert!(verifier.current(second.fresh_until()).await.is_err());
    }

    #[tokio::test]
    async fn duplicate_or_missing_key_ids_never_publish() {
        let duplicate = br#"{"keys":[{"kty":"RSA","kid":"same","n":"AQAB","e":"AQAB"},{"kty":"RSA","kid":"same","n":"AQAB","e":"AQAB"}]}"#.to_vec();
        let verifier = verifier(vec![duplicate]);
        assert_eq!(
            verifier.refresh(Utc::now()).await.unwrap_err(),
            RuntimeAuthorizationError::DependencyUnavailable
        );
        assert!(verifier.current(Utc::now()).await.is_err());
    }
}
