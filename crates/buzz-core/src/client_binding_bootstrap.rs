//! Relay-authenticated connection bootstrap for client binding status.
//!
//! Kind `24245` is delivered only on the WebSocket connection whose native
//! client supplied the echoed epoch. It binds the relay's NIP-11 signing key,
//! the server-resolved authorization domain, and the authenticated event
//! author before kind `24244` status can be consumed.

use std::fmt;

use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Timestamp};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    client_binding_status::MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS,
    kind::KIND_CLIENT_BINDING_BOOTSTRAP, verify_event, CommunityId,
};

/// Integrity-protected NIP-42 tag carrying the native connection scope.
pub const CLIENT_BINDING_SCOPE_TAG: &str = "buzz_client_binding_scope";
/// Reserved exact-connection subscription id for bootstrap delivery.
pub const CLIENT_BINDING_BOOTSTRAP_SUB_ID: &str = "__buzz_client_binding_bootstrap_v1__";
/// Reserved exact-connection subscription id for status delivery.
pub const CLIENT_BINDING_STATUS_SUB_ID: &str = "__buzz_client_binding_status_v1__";
/// Bootstrap wire version accepted by this module.
pub const CLIENT_BINDING_BOOTSTRAP_VERSION: u64 = 1;
/// Maximum encoded bootstrap payload length.
pub const MAX_CLIENT_BINDING_BOOTSTRAP_PAYLOAD_BYTES: usize = 1024;

/// Opaque, native-generated canonical lowercase UUIDv4 connection epoch.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ClientBindingEpoch(String);

impl ClientBindingEpoch {
    /// Generate a fresh connection epoch from the operating system CSPRNG.
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Parse the canonical lowercase hyphenated UUIDv4 wire form.
    pub fn parse(value: &str) -> Result<Self, ClientBindingBootstrapError> {
        let parsed = Uuid::parse_str(value)
            .map_err(|_| ClientBindingBootstrapError::InvalidConnectionEpoch)?;
        let bytes = parsed.as_bytes();
        if parsed.to_string() != value || (bytes[6] >> 4) != 4 || (bytes[8] >> 6) != 2 {
            return Err(ClientBindingBootstrapError::InvalidConnectionEpoch);
        }
        Ok(Self(value.to_owned()))
    }

    /// Canonical payload and signed-tag representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Verified native connection scope carried by one signed NIP-42 AUTH event.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientBindingScopeV1 {
    connection_epoch: ClientBindingEpoch,
    relay_signer: PublicKey,
}

impl ClientBindingScopeV1 {
    /// Parse exactly one canonical v1 scope tag from a verified AUTH event.
    ///
    /// This parser does not authenticate the event. Callers must invoke it only
    /// after the ordinary NIP-42 signature, challenge, and relay checks pass.
    pub fn from_verified_auth_event(event: &Event) -> Result<Self, ClientBindingBootstrapError> {
        let mut matching = event.tags.iter().filter(|tag| {
            tag.as_slice().first().map(String::as_str) == Some(CLIENT_BINDING_SCOPE_TAG)
        });
        let tag = matching
            .next()
            .ok_or(ClientBindingBootstrapError::MissingScopeTag)?;
        if matching.next().is_some() {
            return Err(ClientBindingBootstrapError::DuplicateScopeTag);
        }
        let values = tag.as_slice();
        if values.len() != 4 || values[1] != "1" {
            return Err(ClientBindingBootstrapError::InvalidScopeTag);
        }
        let connection_epoch = ClientBindingEpoch::parse(&values[2])?;
        let relay_signer = parse_canonical_pubkey(&values[3])
            .map_err(|_| ClientBindingBootstrapError::InvalidScopeTag)?;
        Ok(Self {
            connection_epoch,
            relay_signer,
        })
    }

    /// Native-generated epoch authenticated by the NIP-42 event signature.
    pub fn connection_epoch(&self) -> &ClientBindingEpoch {
        &self.connection_epoch
    }

    /// NIP-11 relay signer pinned by native before the socket was opened.
    pub const fn relay_signer(&self) -> PublicKey {
        self.relay_signer
    }
}

impl fmt::Debug for ClientBindingScopeV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingScopeV1")
            .field("connection_epoch", &"[redacted]")
            .field("relay_signer", &"[redacted]")
            .finish()
    }
}

impl fmt::Debug for ClientBindingEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ClientBindingEpoch")
            .field(&"[redacted]")
            .finish()
    }
}

/// Validated relay-authenticated bootstrap.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientBindingBootstrapV1 {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    connection_epoch: ClientBindingEpoch,
    issued_at: u64,
}

impl ClientBindingBootstrapV1 {
    /// Server-resolved authorization domain pinned by this connection.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Authenticated event author pinned by this connection.
    pub const fn event_author_pubkey(&self) -> PublicKey {
        self.event_author_pubkey
    }

    /// Echoed native connection epoch.
    pub fn connection_epoch(&self) -> &ClientBindingEpoch {
        &self.connection_epoch
    }

    /// Relay issue time in Unix seconds.
    pub const fn issued_at(&self) -> u64 {
        self.issued_at
    }
}

impl fmt::Debug for ClientBindingBootstrapV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingBootstrapV1")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("connection_epoch", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .finish()
    }
}

/// Validated server-side signing input for one connection bootstrap.
pub struct ClientBindingBootstrapInputV1 {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    connection_epoch: ClientBindingEpoch,
    issued_at: u64,
}

impl ClientBindingBootstrapInputV1 {
    /// Bind server-resolved connection authority to a native epoch.
    pub fn new(
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
        connection_epoch: ClientBindingEpoch,
        issued_at: u64,
    ) -> Result<Self, ClientBindingBootstrapError> {
        if authorization_domain.as_uuid().is_nil() {
            return Err(ClientBindingBootstrapError::InvalidAuthorizationDomain);
        }
        if issued_at == 0 {
            return Err(ClientBindingBootstrapError::InvalidIssueTime);
        }
        Ok(Self {
            authorization_domain,
            event_author_pubkey,
            connection_epoch,
            issued_at,
        })
    }

    /// Sign the bootstrap with the relay key advertised by NIP-11 `self`.
    pub fn sign_with_relay_keys(
        self,
        relay_keys: &Keys,
    ) -> Result<Event, ClientBindingBootstrapBuildError> {
        let wire = WireClientBindingBootstrapV1 {
            version: CLIENT_BINDING_BOOTSTRAP_VERSION,
            authorization_domain: self.authorization_domain.as_uuid().to_string(),
            event_author_pubkey: self.event_author_pubkey.to_hex(),
            connection_epoch: self.connection_epoch.0,
            issued_at: self.issued_at,
        };
        let content = serde_json::to_string(&wire)
            .map_err(|_| ClientBindingBootstrapBuildError::Serialization)?;
        if content.len() > MAX_CLIENT_BINDING_BOOTSTRAP_PAYLOAD_BYTES {
            return Err(ClientBindingBootstrapBuildError::PayloadTooLarge);
        }
        EventBuilder::new(Kind::Custom(KIND_CLIENT_BINDING_BOOTSTRAP as u16), content)
            .tags([])
            .custom_created_at(Timestamp::from(wire.issued_at))
            .sign_with_keys(relay_keys)
            .map_err(|_| ClientBindingBootstrapBuildError::Signing)
    }
}

impl fmt::Debug for ClientBindingBootstrapInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingBootstrapInputV1")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("connection_epoch", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .finish()
    }
}

#[derive(Deserialize)]
struct VersionHeader {
    version: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireClientBindingBootstrapV1 {
    version: u64,
    authorization_domain: String,
    event_author_pubkey: String,
    connection_epoch: String,
    issued_at: u64,
}

/// Authenticate and validate a connection bootstrap against native authority.
pub fn validate_client_binding_bootstrap_event(
    event: &Event,
    trusted_relay_pubkey: &PublicKey,
    expected_connection_epoch: &ClientBindingEpoch,
    expected_event_author_pubkey: &PublicKey,
    now: u64,
) -> Result<ClientBindingBootstrapV1, ClientBindingBootstrapError> {
    if event.kind.as_u16() as u32 != KIND_CLIENT_BINDING_BOOTSTRAP {
        return Err(ClientBindingBootstrapError::WrongKind);
    }
    if event.content.len() > MAX_CLIENT_BINDING_BOOTSTRAP_PAYLOAD_BYTES {
        return Err(ClientBindingBootstrapError::PayloadTooLarge);
    }
    verify_event(event).map_err(|_| ClientBindingBootstrapError::UnauthenticatedEvent)?;
    if event.pubkey != *trusted_relay_pubkey {
        return Err(ClientBindingBootstrapError::UnexpectedRelay);
    }
    if !event.tags.is_empty() {
        return Err(ClientBindingBootstrapError::UnexpectedTags);
    }
    let header: VersionHeader = serde_json::from_str(&event.content)
        .map_err(|_| ClientBindingBootstrapError::MalformedPayload)?;
    if header.version != CLIENT_BINDING_BOOTSTRAP_VERSION {
        return Err(ClientBindingBootstrapError::UnsupportedVersion);
    }
    let wire: WireClientBindingBootstrapV1 = serde_json::from_str(&event.content)
        .map_err(|_| ClientBindingBootstrapError::MalformedPayload)?;
    let authorization_domain = Uuid::parse_str(&wire.authorization_domain)
        .map_err(|_| ClientBindingBootstrapError::InvalidAuthorizationDomain)?;
    if authorization_domain.is_nil()
        || authorization_domain.to_string() != wire.authorization_domain
    {
        return Err(ClientBindingBootstrapError::InvalidAuthorizationDomain);
    }
    let event_author_pubkey = parse_canonical_pubkey(&wire.event_author_pubkey)?;
    if event_author_pubkey != *expected_event_author_pubkey {
        return Err(ClientBindingBootstrapError::EventAuthorMismatch);
    }
    let connection_epoch = ClientBindingEpoch::parse(&wire.connection_epoch)?;
    if connection_epoch != *expected_connection_epoch {
        return Err(ClientBindingBootstrapError::ConnectionEpochMismatch);
    }
    if wire.issued_at == 0 {
        return Err(ClientBindingBootstrapError::InvalidIssueTime);
    }
    if event.created_at.as_secs() != wire.issued_at {
        return Err(ClientBindingBootstrapError::EventTimeMismatch);
    }
    if wire.issued_at > now {
        return Err(ClientBindingBootstrapError::NotYetValid);
    }
    if now - wire.issued_at > MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS {
        return Err(ClientBindingBootstrapError::Expired);
    }
    Ok(ClientBindingBootstrapV1 {
        authorization_domain: CommunityId::from_uuid(authorization_domain),
        event_author_pubkey,
        connection_epoch,
        issued_at: wire.issued_at,
    })
}

fn parse_canonical_pubkey(value: &str) -> Result<PublicKey, ClientBindingBootstrapError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ClientBindingBootstrapError::InvalidEventAuthorPubkey);
    }
    PublicKey::from_hex(value).map_err(|_| ClientBindingBootstrapError::InvalidEventAuthorPubkey)
}

/// Fail-closed bootstrap validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ClientBindingBootstrapError {
    /// The verified AUTH event did not opt into native status projection.
    #[error("client binding scope tag is missing")]
    MissingScopeTag,
    /// More than one connection scope tag was present.
    #[error("client binding scope tag is duplicated")]
    DuplicateScopeTag,
    /// The signed connection scope tag was not the exact canonical v1 shape.
    #[error("client binding scope tag is invalid")]
    InvalidScopeTag,
    /// Event kind was not the dedicated bootstrap kind.
    #[error("client binding bootstrap event has the wrong kind")]
    WrongKind,
    /// Payload exceeded the public bound.
    #[error("client binding bootstrap payload is too large")]
    PayloadTooLarge,
    /// Event signature or identifier was invalid.
    #[error("client binding bootstrap event is not authenticated")]
    UnauthenticatedEvent,
    /// Signer did not match NIP-11 `self`.
    #[error("client binding bootstrap signer is not the trusted relay")]
    UnexpectedRelay,
    /// Bootstrap events must have no tags.
    #[error("client binding bootstrap contains unexpected tags")]
    UnexpectedTags,
    /// Payload was not the bounded v1 shape.
    #[error("client binding bootstrap payload is malformed")]
    MalformedPayload,
    /// Wire version is unsupported.
    #[error("client binding bootstrap version is unsupported")]
    UnsupportedVersion,
    /// Authorization domain was nil or noncanonical.
    #[error("client binding bootstrap authorization domain is invalid")]
    InvalidAuthorizationDomain,
    /// Event-author key was noncanonical.
    #[error("client binding bootstrap event author is invalid")]
    InvalidEventAuthorPubkey,
    /// Authenticated author did not match native signing state.
    #[error("client binding bootstrap event author does not match")]
    EventAuthorMismatch,
    /// Connection epoch was noncanonical.
    #[error("client binding bootstrap connection epoch is invalid")]
    InvalidConnectionEpoch,
    /// Echoed epoch did not match the native signed NIP-42 scope.
    #[error("client binding bootstrap connection epoch does not match")]
    ConnectionEpochMismatch,
    /// Issue time was zero.
    #[error("client binding bootstrap issue time is invalid")]
    InvalidIssueTime,
    /// Signed timestamp did not equal the payload timestamp.
    #[error("client binding bootstrap event time does not match")]
    EventTimeMismatch,
    /// Bootstrap claims a future issue time.
    #[error("client binding bootstrap is not yet valid")]
    NotYetValid,
    /// Bootstrap exceeded the client-status maximum lifetime.
    #[error("client binding bootstrap has expired")]
    Expired,
}

/// Bootstrap serialization or signing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientBindingBootstrapBuildError {
    /// JSON serialization failed.
    #[error("client binding bootstrap serialization failed")]
    Serialization,
    /// Serialized payload exceeded its public bound.
    #[error("client binding bootstrap payload is too large")]
    PayloadTooLarge,
    /// Relay signing failed.
    #[error("client binding bootstrap signing failed")]
    Signing,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{JsonUtil, Tag};
    use serde_json::{json, Value};

    const DOMAIN: &str = "abcdefab-cdef-4abc-8def-abcdefabcdef";
    const ISSUED_AT: u64 = 1_800_000_000;

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::parse_str(DOMAIN).expect("synthetic domain is valid"))
    }

    fn epoch(byte: u8) -> ClientBindingEpoch {
        ClientBindingEpoch::parse(&format!("11111111-1111-4111-8111-{byte:012x}"))
            .expect("synthetic epoch is canonical UUIDv4")
    }

    fn signed_bootstrap(relay: &Keys, author: PublicKey) -> Event {
        ClientBindingBootstrapInputV1::new(domain(), author, epoch(0x11), ISSUED_AT)
            .expect("synthetic bootstrap input is valid")
            .sign_with_relay_keys(relay)
            .expect("synthetic bootstrap signs")
    }

    fn validate(
        event: &Event,
        relay: &Keys,
        author: &Keys,
        expected_epoch: &ClientBindingEpoch,
        now: u64,
    ) -> Result<ClientBindingBootstrapV1, ClientBindingBootstrapError> {
        validate_client_binding_bootstrap_event(
            event,
            &relay.public_key(),
            expected_epoch,
            &author.public_key(),
            now,
        )
    }

    fn resign_payload(relay: &Keys, payload: Value) -> Event {
        EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_BOOTSTRAP as u16),
            payload.to_string(),
        )
        .tags([])
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(relay)
        .expect("synthetic bootstrap variant signs")
    }

    #[test]
    fn relay_authenticated_bootstrap_roundtrips_exact_authority() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let expected_epoch = epoch(0x11);
        let event = signed_bootstrap(&relay, author.public_key());

        let bootstrap = validate(&event, &relay, &author, &expected_epoch, ISSUED_AT)
            .expect("synthetic bootstrap validates");

        assert_eq!(event.kind.as_u16() as u32, KIND_CLIENT_BINDING_BOOTSTRAP);
        assert!(event.tags.is_empty());
        assert_eq!(bootstrap.authorization_domain(), domain());
        assert_eq!(bootstrap.event_author_pubkey(), author.public_key());
        assert_eq!(bootstrap.connection_epoch(), &expected_epoch);
        assert_eq!(bootstrap.issued_at(), ISSUED_AT);
        let payload: Value = serde_json::from_str(&event.content).expect("payload parses");
        assert_eq!(
            payload
                .as_object()
                .expect("payload is an object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "authorization_domain",
                "connection_epoch",
                "event_author_pubkey",
                "issued_at",
                "version",
            ])
        );
        let debug = format!("{bootstrap:?}");
        assert!(!debug.contains(DOMAIN));
        assert!(!debug.contains(&author.public_key().to_hex()));
        assert!(!debug.contains(expected_epoch.as_str()));
    }

    #[test]
    fn bootstrap_rejects_noncanonical_epoch_and_invalid_input_bounds() {
        assert_eq!(
            ClientBindingEpoch::parse("11111111-1111-4111-8111-AAAAAAAAAAAA"),
            Err(ClientBindingBootstrapError::InvalidConnectionEpoch)
        );
        assert_eq!(
            ClientBindingEpoch::parse("11111111-1111-5111-8111-111111111111"),
            Err(ClientBindingBootstrapError::InvalidConnectionEpoch)
        );
        assert_eq!(
            ClientBindingEpoch::parse("11111111-1111-4111-7111-111111111111"),
            Err(ClientBindingBootstrapError::InvalidConnectionEpoch)
        );
        assert_eq!(
            ClientBindingBootstrapInputV1::new(
                CommunityId::from_uuid(Uuid::nil()),
                Keys::generate().public_key(),
                epoch(1),
                ISSUED_AT,
            )
            .expect_err("nil domains are invalid"),
            ClientBindingBootstrapError::InvalidAuthorizationDomain
        );
        assert_eq!(
            ClientBindingBootstrapInputV1::new(
                domain(),
                Keys::generate().public_key(),
                epoch(1),
                0,
            )
            .expect_err("zero issue time is invalid"),
            ClientBindingBootstrapError::InvalidIssueTime
        );
    }

    #[test]
    fn bootstrap_rejects_wrong_scope_tampering_and_unknown_shape() {
        let relay = Keys::generate();
        let wrong_relay = Keys::generate();
        let author = Keys::generate();
        let other_author = Keys::generate();
        let expected_epoch = epoch(0x11);
        let event = signed_bootstrap(&relay, author.public_key());

        assert_eq!(
            validate(&event, &wrong_relay, &author, &expected_epoch, ISSUED_AT),
            Err(ClientBindingBootstrapError::UnexpectedRelay)
        );
        assert_eq!(
            validate(&event, &relay, &other_author, &expected_epoch, ISSUED_AT),
            Err(ClientBindingBootstrapError::EventAuthorMismatch)
        );
        assert_eq!(
            validate(&event, &relay, &author, &epoch(0x22), ISSUED_AT),
            Err(ClientBindingBootstrapError::ConnectionEpochMismatch)
        );
        assert_eq!(
            validate(&event, &relay, &author, &expected_epoch, ISSUED_AT - 1),
            Err(ClientBindingBootstrapError::NotYetValid)
        );
        assert_eq!(
            validate(
                &event,
                &relay,
                &author,
                &expected_epoch,
                ISSUED_AT + MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS + 1,
            ),
            Err(ClientBindingBootstrapError::Expired)
        );

        let mut tampered_json: Value =
            serde_json::from_str(&event.as_json()).expect("event parses");
        tampered_json["content"] = Value::String("{}".to_string());
        let tampered =
            Event::from_json(tampered_json.to_string()).expect("tampered event still parses");
        assert_eq!(
            validate(&tampered, &relay, &author, &expected_epoch, ISSUED_AT),
            Err(ClientBindingBootstrapError::UnauthenticatedEvent)
        );

        let mut payload: Value =
            serde_json::from_str(&event.content).expect("bootstrap content parses");
        payload["synthetic_extension"] = json!(true);
        assert_eq!(
            validate(
                &resign_payload(&relay, payload),
                &relay,
                &author,
                &expected_epoch,
                ISSUED_AT,
            ),
            Err(ClientBindingBootstrapError::MalformedPayload)
        );

        let mut payload: Value =
            serde_json::from_str(&event.content).expect("bootstrap content parses");
        payload["authorization_domain"] = json!(DOMAIN.to_uppercase());
        assert_eq!(
            validate(
                &resign_payload(&relay, payload),
                &relay,
                &author,
                &expected_epoch,
                ISSUED_AT,
            ),
            Err(ClientBindingBootstrapError::InvalidAuthorizationDomain)
        );
    }

    #[test]
    fn verified_auth_scope_requires_one_exact_signed_tag() {
        let author = Keys::generate();
        let relay = Keys::generate();
        let epoch = epoch(0x11);
        let scope = vec![
            CLIENT_BINDING_SCOPE_TAG.to_string(),
            "1".to_string(),
            epoch.as_str().to_string(),
            relay.public_key().to_hex(),
        ];
        let auth = EventBuilder::new(Kind::Custom(22242), "")
            .tags([Tag::parse(scope.clone()).expect("synthetic scope tag")])
            .sign_with_keys(&author)
            .expect("synthetic AUTH signs");
        let parsed = ClientBindingScopeV1::from_verified_auth_event(&auth)
            .expect("exact signed scope parses");
        assert_eq!(parsed.connection_epoch(), &epoch);
        assert_eq!(parsed.relay_signer(), relay.public_key());

        let missing = EventBuilder::new(Kind::Custom(22242), "")
            .sign_with_keys(&author)
            .expect("synthetic AUTH signs");
        assert_eq!(
            ClientBindingScopeV1::from_verified_auth_event(&missing),
            Err(ClientBindingBootstrapError::MissingScopeTag)
        );

        let duplicate = EventBuilder::new(Kind::Custom(22242), "")
            .tags([
                Tag::parse(scope.clone()).expect("synthetic scope tag"),
                Tag::parse(scope).expect("synthetic scope tag"),
            ])
            .sign_with_keys(&author)
            .expect("synthetic AUTH signs");
        assert_eq!(
            ClientBindingScopeV1::from_verified_auth_event(&duplicate),
            Err(ClientBindingBootstrapError::DuplicateScopeTag)
        );
    }
}
