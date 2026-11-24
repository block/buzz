//! Provider adapters: each one knows how its provider signs a delivery and
//! how to name the delivery and its event type. Adapters never see a secret
//! that the keyring has not opened for this request.
use crate::store::Provider;
use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};

/// GitHub webhook verification.
pub mod github;

/// Upper bound on a provider delivery id; longer values are malformed.
pub const MAX_DELIVERY_ID_BYTES: usize = 256;
/// Upper bound on the derived event type; matches the deliveries table.
pub const MAX_EVENT_TYPE_BYTES: usize = 128;

/// What an adapter establishes about a delivery after verifying it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub delivery_id: String,
    pub event_type: String,
    pub provider_timestamp: Option<DateTime<Utc>>,
}

/// Why a delivery was not accepted. Mapped to 400 or 401 by the intake
/// handler; the response never says which secret failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    MissingSignature,
    BadSignature,
    StaleTimestamp,
    MalformedHeaders,
    UnsupportedEvent,
}

/// Provider-specific verification and naming.
pub trait ProviderAdapter: Send + Sync {
    /// The provider this adapter serves.
    fn provider(&self) -> Provider;
    /// Header names (lowercase) worth persisting with the delivery for the
    /// forwarder and the admin timeline. Everything else is dropped.
    fn retained_headers(&self) -> &'static [&'static str];
    /// Verify `body` against `headers` with any of the plaintext `secrets`.
    /// `secrets` are ordered newest first and include retired secrets still
    /// inside their grace window.
    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        secrets: &[Vec<u8>],
        now: DateTime<Utc>,
        tolerance: Duration,
    ) -> Result<Verified, VerifyError>;
}

/// The adapter for a provider, if this build supports it.
pub fn adapter(provider: Provider) -> Option<&'static dyn ProviderAdapter> {
    match provider {
        Provider::Github => Some(&github::GitHub),
        Provider::Pagerduty | Provider::Sentry | Provider::IncidentIo => None,
    }
}

pub(crate) fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub(crate) fn valid_delivery_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DELIVERY_ID_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}
