//! NIP-98 HTTP Auth verification (kind:27235).
//!
//! NIP-98 is the standard Nostr HTTP Auth pattern used by Nostr.build, Blossom, and
//! other Nostr HTTP services. It is **stateless** — no WebSocket session required.
//!
//! The client signs a short-lived kind:27235 event containing the target URL, HTTP method,
//! and an optional SHA-256 hash of the request body, then sends it as:
//!
//! ```text
//! Authorization: Nostr <base64(JSON-serialized-event)>
//! ```
//!
//! ## Verification steps
//!
//! 1. Parse JSON into a `nostr::Event`
//! 2. Verify `kind == 27235` (`Kind::HttpAuth`)
//! 3. Verify Schnorr signature via `buzz_core::verify_event`
//! 4. Verify `created_at` within ±60 seconds of server time
//! 5. Verify `["u", <url>]` tag matches `expected_url` (normalised: case-insensitive
//!    scheme/host, trailing slash stripped)
//! 6. Verify `["method", <method>]` tag matches `expected_method` (case-insensitive)
//! 7. If `["payload", <hash>]` tag is present **and** `body` is `Some`: verify
//!    `SHA-256(body) == hex(payload_tag)`. This prevents body-substitution attacks.
//! 8. Return `event.pubkey` on success.

use chrono::{DateTime, Duration, Utc};
use nostr::{Alphabet, Event, Kind, SingleLetterTag, TagKind, Timestamp};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    error::AuthError, ActiveLocalBinding, AuthorizationError, AuthorizationFinalizer,
    AuthorizationInput, LocalAuthorizationPolicy, LocalBindingResolution, PreparedAuthorization,
    ProofTransport, RouteCapability, VerifiedFederatedAssertion, VerifiedNostrProof,
};

const TIMESTAMP_TOLERANCE_SECS: u64 = 60;
const GIT_SMART_HTTP_SESSION_SECS: u64 = 300;

/// Verify a NIP-98 HTTP Auth event (kind:27235).
///
/// # Parameters
///
/// - `event_json` — the raw JSON string of the Nostr event (decoded from base64 by the caller).
/// - `expected_url` — the canonical URL of the request being authenticated.
///   For reverse-proxy deployments, reconstruct from `X-Forwarded-Proto` / `X-Forwarded-Host`
///   before passing here.
/// - `expected_method` — the HTTP method (e.g. `"POST"`). Compared case-insensitively.
/// - `body` — raw request body bytes. If `Some` and a `payload` tag is present in the event,
///   the SHA-256 hash of `body` must match the tag value. If `None`, the `payload` tag is
///   ignored (clients SHOULD include it for POST requests, but it is not required).
///
/// # Returns
///
/// The authenticated `nostr::PublicKey` on success.
///
/// # Errors
///
/// Returns [`AuthError::Nip98Invalid`] with a descriptive message for any verification failure.
/// The message is safe for server logs but should not be forwarded verbatim to clients.
pub fn verify_nip98_event(
    event_json: &str,
    expected_url: &str,
    expected_method: &str,
    body: Option<&[u8]>,
) -> Result<nostr::PublicKey, AuthError> {
    // 1. Parse JSON.
    let event: Event = serde_json::from_str(event_json)
        .map_err(|e| AuthError::Nip98Invalid(format!("event JSON parse error: {e}")))?;

    // 2. Verify kind == 27235.
    if event.kind != Kind::HttpAuth {
        return Err(AuthError::Nip98Invalid(format!(
            "expected kind 27235, got {}",
            event.kind.as_u16()
        )));
    }

    // 3. Verify Schnorr signature (also verifies event ID hash).
    buzz_core::verify_event(&event)
        .map_err(|_| AuthError::Nip98Invalid("invalid Schnorr signature".to_string()))?;

    // 4. Verify created_at within ±60 seconds of now.
    let now = Timestamp::now().as_secs();
    let event_ts = event.created_at.as_secs();
    let delta = now.abs_diff(event_ts);
    if delta > TIMESTAMP_TOLERANCE_SECS {
        return Err(AuthError::Nip98Invalid(format!(
            "event timestamp outside ±{TIMESTAMP_TOLERANCE_SECS}s window (delta: {delta}s)"
        )));
    }

    // 5. Verify `u` tag matches expected_url (normalised).
    // NIP-98 uses the single-letter "u" tag, not the multi-letter "url" tag.
    let u_tag = event
        .tags
        .find(TagKind::SingleLetter(SingleLetterTag::lowercase(
            Alphabet::U,
        )))
        .and_then(|t| t.content())
        .ok_or_else(|| AuthError::Nip98Invalid("missing `u` tag".to_string()))?;

    if normalize_url(u_tag) != normalize_url(expected_url) {
        return Err(AuthError::Nip98Invalid(format!(
            "URL mismatch: event has `{u_tag}`, expected `{expected_url}`"
        )));
    }

    // 6. Verify `method` tag matches expected_method (case-insensitive).
    let method_tag = event
        .tags
        .find(TagKind::Method)
        .and_then(|t| t.content())
        .ok_or_else(|| AuthError::Nip98Invalid("missing `method` tag".to_string()))?;

    if !method_tag.eq_ignore_ascii_case(expected_method) {
        return Err(AuthError::Nip98Invalid(format!(
            "method mismatch: event has `{method_tag}`, expected `{expected_method}`"
        )));
    }

    // 7. If `payload` tag present AND body is Some: verify SHA-256(body) == payload hex.
    let payload_tag = event.tags.find(TagKind::Payload).and_then(|t| t.content());

    if let (Some(payload_hex), Some(body_bytes)) = (payload_tag, body) {
        let computed: [u8; 32] = Sha256::digest(body_bytes).into();
        let computed_hex = hex::encode(computed);
        if computed_hex != payload_hex {
            return Err(AuthError::Nip98Invalid(
                "payload tag SHA-256 mismatch: request body does not match signed hash".to_string(),
            ));
        }
    }

    // 8. Return the authenticated pubkey.
    Ok(event.pubkey)
}

/// Verify NIP-98 transport and bind it to one canonical federated assertion.
///
/// The assertion and Nostr event are verified independently, then required to
/// name byte-identical request, target, transport, and context coordinates.
/// Git Smart HTTP may select its closed session transport because one signed
/// NIP-98 credential is intentionally reused across the bounded Git exchange.
#[allow(clippy::too_many_arguments)]
pub fn verify_nip98_authorization_proof(
    event_json: &str,
    expected_url: &str,
    expected_method: &str,
    body: Option<&[u8]>,
    assertion: &VerifiedFederatedAssertion,
    transport: ProofTransport,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
) -> Result<VerifiedNostrProof, AuthError> {
    if !matches!(
        transport,
        ProofTransport::Nip98 | ProofTransport::GitSmartHttpSession
    ) {
        return Err(AuthError::Nip98Invalid(
            "invalid canonical NIP-98 transport".to_owned(),
        ));
    }
    let actor = verify_nip98_event(event_json, expected_url, expected_method, body)?;
    let event: Event = serde_json::from_str(event_json)
        .map_err(|_| AuthError::Nip98Invalid("invalid canonical NIP-98 event".to_owned()))?;
    let event_expiry_seconds = event
        .created_at
        .as_secs()
        .checked_add(match transport {
            ProofTransport::GitSmartHttpSession => GIT_SMART_HTTP_SESSION_SECS,
            ProofTransport::Nip98 => TIMESTAMP_TOLERANCE_SECS,
            _ => {
                return Err(AuthError::Nip98Invalid(
                    "invalid canonical NIP-98 transport".to_owned(),
                ))
            }
        })
        .ok_or_else(|| AuthError::Nip98Invalid("invalid canonical NIP-98 expiry".to_owned()))?;
    let event_expiry_seconds = i64::try_from(event_expiry_seconds)
        .map_err(|_| AuthError::Nip98Invalid("invalid canonical NIP-98 expiry".to_owned()))?;
    let event_expires_at = DateTime::<Utc>::from_timestamp(event_expiry_seconds, 0)
        .ok_or_else(|| AuthError::Nip98Invalid("invalid canonical NIP-98 expiry".to_owned()))?;
    let (_, assertion_expires_at) = assertion.time_bounds();
    let expires_at = event_expires_at.min(assertion_expires_at);
    let (assertion_transport, assertion_request, assertion_target, assertion_context) =
        assertion.request_binding();
    if assertion.authorization_domain().as_uuid().is_nil()
        || assertion_transport != transport
        || assertion_request != &request_fingerprint
        || assertion_target != &target_fingerprint
        || assertion_context != &transport_context_fingerprint
        || expires_at <= Utc::now()
    {
        return Err(AuthError::Nip98Invalid(
            "canonical NIP-98 binding mismatch".to_owned(),
        ));
    }
    VerifiedNostrProof::from_verifier(
        assertion.authorization_domain(),
        actor,
        transport,
        request_fingerprint,
        target_fingerprint,
        transport_context_fingerprint,
        Some(*assertion.assertion_fingerprint()),
        None,
        expires_at,
    )
    .ok_or_else(|| AuthError::Nip98Invalid("invalid canonical NIP-98 binding".to_owned()))
}

/// Exact server-derived coordinates for one body-bound invite claim.
///
/// The request fingerprint intentionally excludes event id, signature, and
/// serialization so a freshly signed proof for the same semantic claim reaches
/// the same canonical admission receipt. The transport fingerprint still binds
/// the exact host, URL, method, and payload.
pub struct Nip98InviteClaimCoordinates {
    authorization_domain: buzz_core::CommunityId,
    canonical_url: Box<str>,
    canonical_host: Box<str>,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    payload_sha256: [u8; 32],
}

impl Nip98InviteClaimCoordinates {
    /// Bind an exact POST request to one server-resolved invite target.
    pub fn new(
        authorization_domain: buzz_core::CommunityId,
        target_fingerprint: [u8; 32],
        expected_url: &str,
        body: &[u8],
    ) -> Result<Self, AuthError> {
        if authorization_domain.as_uuid().is_nil() || target_fingerprint == [0; 32] {
            return Err(AuthError::Nip98Invalid(
                "invalid invite claim coordinates".to_owned(),
            ));
        }
        let (canonical_url, canonical_host) = strict_canonical_url(expected_url)?;
        let payload_sha256: [u8; 32] = Sha256::digest(body).into();
        let request_fingerprint = framed_digest(
            b"buzz:nip-fi:nip98-invite-request:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                &target_fingerprint,
                b"POST",
                canonical_url.as_bytes(),
                &payload_sha256,
            ],
        );
        let transport_context_fingerprint = framed_digest(
            b"buzz:nip-fi:nip98-invite-transport:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                canonical_host.as_bytes(),
                b"POST",
                canonical_url.as_bytes(),
                &payload_sha256,
            ],
        );
        Ok(Self {
            authorization_domain,
            canonical_url: canonical_url.into(),
            canonical_host: canonical_host.into(),
            request_fingerprint,
            target_fingerprint,
            transport_context_fingerprint,
            payload_sha256,
        })
    }

    /// Exact request, protected target, and transport-context fingerprints.
    pub const fn request_binding(&self) -> (&[u8; 32], &[u8; 32], &[u8; 32]) {
        (
            &self.request_fingerprint,
            &self.target_fingerprint,
            &self.transport_context_fingerprint,
        )
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> buzz_core::CommunityId {
        self.authorization_domain
    }
}

impl std::fmt::Debug for Nip98InviteClaimCoordinates {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Nip98InviteClaimCoordinates([REDACTED])")
    }
}

/// Origin-sealed proof of the exact NIP-98 invite-claim purpose.
///
/// This opaque type prevents a generic NIP-98 proof produced for another
/// route from crossing the canonical invite-admission boundary.
#[derive(Clone)]
pub struct VerifiedNip98InviteClaimProof {
    proof: VerifiedNostrProof,
}

/// Exact server-derived coordinates for one moderation command submitted over
/// the HTTP event bridge.
pub struct Nip98ModerationCommandCoordinates {
    authorization_domain: buzz_core::CommunityId,
    canonical_url: Box<str>,
    canonical_host: Box<str>,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    payload_sha256: [u8; 32],
    actor: nostr::PublicKey,
    command_event_id: [u8; 32],
    command_kind: u32,
}

impl Nip98ModerationCommandCoordinates {
    /// Bind one body-authenticated `/events` request to its decoded moderation
    /// target. The target must come from the verified command, never a client
    /// side channel.
    pub fn new(
        authorization_domain: buzz_core::CommunityId,
        target_fingerprint: [u8; 32],
        expected_url: &str,
        body: &[u8],
        command: &Event,
    ) -> Result<Self, AuthError> {
        if authorization_domain.as_uuid().is_nil() || target_fingerprint == [0; 32] {
            return Err(AuthError::Nip98Invalid(
                "invalid moderation command coordinates".to_owned(),
            ));
        }
        let (canonical_url, canonical_host) = strict_canonical_url(expected_url)?;
        let payload_sha256: [u8; 32] = Sha256::digest(body).into();
        let command_kind = command.kind.as_u16() as u32;
        if !buzz_core::kind::is_moderation_command_kind(command_kind) {
            return Err(AuthError::Nip98Invalid(
                "invalid moderation command kind".to_owned(),
            ));
        }
        let command_event_id = command.id.to_bytes();
        let request_fingerprint = moderation_command_request_fingerprint(
            authorization_domain,
            command.pubkey,
            command_event_id,
            command_kind,
            target_fingerprint,
        );
        let transport_context_fingerprint = framed_digest(
            b"buzz:nip-fi:nip98-moderation-transport:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                canonical_host.as_bytes(),
                b"POST",
                canonical_url.as_bytes(),
                &payload_sha256,
            ],
        );
        Ok(Self {
            authorization_domain,
            canonical_url: canonical_url.into(),
            canonical_host: canonical_host.into(),
            request_fingerprint,
            target_fingerprint,
            transport_context_fingerprint,
            payload_sha256,
            actor: command.pubkey,
            command_event_id,
            command_kind,
        })
    }
}

impl std::fmt::Debug for Nip98ModerationCommandCoordinates {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Nip98ModerationCommandCoordinates([REDACTED])")
    }
}

/// Exact server-derived coordinates for a fresh signed moderation command
/// submitted on an authenticated WebSocket.
pub struct Nip42ModerationCommandCoordinates {
    authorization_domain: buzz_core::CommunityId,
    relay_url: Box<str>,
    event_id: [u8; 32],
    actor: nostr::PublicKey,
    command_kind: u32,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
}

impl Nip42ModerationCommandCoordinates {
    /// Bind a signed command to the tenant WebSocket origin and decoded target.
    pub fn new(
        authorization_domain: buzz_core::CommunityId,
        target_fingerprint: [u8; 32],
        relay_url: &str,
        command: &Event,
    ) -> Result<Self, AuthError> {
        if authorization_domain.as_uuid().is_nil()
            || target_fingerprint == [0; 32]
            || command.id.to_bytes() == [0; 32]
        {
            return Err(AuthError::Nip98Invalid(
                "invalid moderation command coordinates".to_owned(),
            ));
        }
        let relay_url = strict_websocket_origin(relay_url)?;
        let command_kind = command.kind.as_u16() as u32;
        if !buzz_core::kind::is_moderation_command_kind(command_kind) {
            return Err(AuthError::Nip98Invalid(
                "invalid moderation command kind".to_owned(),
            ));
        }
        let event_id = command.id.to_bytes();
        let request_fingerprint = moderation_command_request_fingerprint(
            authorization_domain,
            command.pubkey,
            event_id,
            command_kind,
            target_fingerprint,
        );
        let transport_context_fingerprint = framed_digest(
            b"buzz:nip-fi:nip42-moderation-transport:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                relay_url.as_bytes(),
                &event_id,
            ],
        );
        Ok(Self {
            authorization_domain,
            relay_url: relay_url.into(),
            event_id,
            actor: command.pubkey,
            command_kind,
            request_fingerprint,
            target_fingerprint,
            transport_context_fingerprint,
        })
    }
}

impl std::fmt::Debug for Nip42ModerationCommandCoordinates {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Nip42ModerationCommandCoordinates([REDACTED])")
    }
}

/// Purpose-sealed proof of an exact moderation command.
#[derive(Clone)]
pub struct VerifiedModerationCommandProof {
    proof: VerifiedNostrProof,
}

impl VerifiedModerationCommandProof {
    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> buzz_core::CommunityId {
        self.proof.authorization_domain()
    }

    /// Exact verified command author.
    pub const fn actor_pubkey(&self) -> nostr::PublicKey {
        self.proof.actor_pubkey()
    }

    /// Transport-neutral signed-command request identity.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        self.proof.request_fingerprint()
    }

    /// Server-decoded moderation target.
    pub const fn target_fingerprint(&self) -> &[u8; 32] {
        self.proof.target_fingerprint()
    }

    /// Exact transport context, intentionally distinct across HTTP and WS.
    pub const fn transport_context_fingerprint(&self) -> &[u8; 32] {
        self.proof.transport_context_fingerprint()
    }

    /// Exclusive proof expiry.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.proof.expires_at()
    }

    /// Finalize this purpose seal against authoritative local state.
    ///
    /// The generic proof and assertion-free binding resolution never leave
    /// this crate, and the capability is fixed here rather than selected by a
    /// caller.
    pub fn prepare_authorization(
        self,
        binding: ActiveLocalBinding,
        policy: LocalAuthorizationPolicy,
        authoritative_now: DateTime<Utc>,
    ) -> Result<PreparedAuthorization, AuthorizationError> {
        let domain = self.proof.authorization_domain();
        let input = AuthorizationInput::new(
            domain,
            uuid::Uuid::new_v4(),
            self.proof,
            RouteCapability::Moderation,
        )?;
        let resolution = LocalBindingResolution::bound_key(binding);
        AuthorizationFinalizer::prepare(input, resolution, policy, authoritative_now)
    }
}

impl std::fmt::Debug for VerifiedModerationCommandProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VerifiedModerationCommandProof([REDACTED])")
    }
}

/// Verify a strict, payload-bound NIP-98 proof for one decoded moderation
/// command.
pub fn verify_nip98_moderation_command_proof(
    event_json: &str,
    coordinates: &Nip98ModerationCommandCoordinates,
    body: &[u8],
    authoritative_now: DateTime<Utc>,
) -> Result<VerifiedModerationCommandProof, AuthError> {
    let event: Event = serde_json::from_str(event_json)
        .map_err(|_| AuthError::Nip98Invalid("invalid moderation proof JSON".to_owned()))?;
    if event.kind != Kind::HttpAuth || !event.content.is_empty() {
        return Err(AuthError::Nip98Invalid(
            "invalid moderation proof kind or content".to_owned(),
        ));
    }
    buzz_core::verify_event(&event)
        .map_err(|_| AuthError::Nip98Invalid("invalid moderation proof signature".to_owned()))?;
    validate_moderation_proof_time(&event, authoritative_now, TIMESTAMP_TOLERANCE_SECS)?;
    let signed_url = exact_tag(
        &event,
        TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::U)),
        "u",
    )?;
    let signed_method = exact_tag(&event, TagKind::Method, "method")?;
    let signed_payload = exact_tag(&event, TagKind::Payload, "payload")?;
    let (canonical_signed_url, signed_host) = strict_canonical_url(signed_url)?;
    let payload_sha256: [u8; 32] = Sha256::digest(body).into();
    let command: Event = serde_json::from_slice(body)
        .map_err(|_| AuthError::Nip98Invalid("invalid moderation command body".to_owned()))?;
    let command_kind = command.kind.as_u16() as u32;
    if canonical_signed_url != coordinates.canonical_url.as_ref()
        || signed_host != coordinates.canonical_host.as_ref()
        || signed_method != "POST"
        || payload_sha256 != coordinates.payload_sha256
        || signed_payload != hex::encode(payload_sha256)
        || event.pubkey != coordinates.actor
        || command.pubkey != coordinates.actor
        || command.id.to_bytes() != coordinates.command_event_id
        || command_kind != coordinates.command_kind
        || !buzz_core::kind::is_moderation_command_kind(command_kind)
    {
        return Err(AuthError::Nip98Invalid(
            "moderation proof request binding mismatch".to_owned(),
        ));
    }
    buzz_core::verify_event(&command)
        .map_err(|_| AuthError::Nip98Invalid("invalid moderation command signature".to_owned()))?;
    validate_moderation_proof_time(&command, authoritative_now, 120)?;
    let expires_at = proof_expiry(&event, authoritative_now, TIMESTAMP_TOLERANCE_SECS)?
        .min(proof_expiry(&command, authoritative_now, 120)?);
    let proof = VerifiedNostrProof::from_verifier(
        coordinates.authorization_domain,
        event.pubkey,
        ProofTransport::Nip98,
        coordinates.request_fingerprint,
        coordinates.target_fingerprint,
        coordinates.transport_context_fingerprint,
        None,
        None,
        expires_at,
    )
    .ok_or_else(|| AuthError::Nip98Invalid("invalid moderation proof binding".to_owned()))?;
    Ok(VerifiedModerationCommandProof { proof })
}

/// Verify a fresh signed moderation event as the operation-bound WebSocket
/// proof. Connection authentication alone is intentionally insufficient.
pub fn verify_nip42_moderation_command_proof(
    command: &Event,
    coordinates: &Nip42ModerationCommandCoordinates,
    authoritative_now: DateTime<Utc>,
) -> Result<VerifiedModerationCommandProof, AuthError> {
    if !buzz_core::kind::is_moderation_command_kind(command.kind.as_u16() as u32)
        || command.id.to_bytes() != coordinates.event_id
        || command.pubkey != coordinates.actor
        || command.kind.as_u16() as u32 != coordinates.command_kind
        || coordinates.relay_url.is_empty()
    {
        return Err(AuthError::Nip98Invalid(
            "moderation command binding mismatch".to_owned(),
        ));
    }
    buzz_core::verify_event(command)
        .map_err(|_| AuthError::Nip98Invalid("invalid moderation command signature".to_owned()))?;
    validate_moderation_proof_time(command, authoritative_now, 120)?;
    let expires_at = proof_expiry(command, authoritative_now, 120)?;
    let proof = VerifiedNostrProof::from_verifier(
        coordinates.authorization_domain,
        command.pubkey,
        ProofTransport::Nip42,
        coordinates.request_fingerprint,
        coordinates.target_fingerprint,
        coordinates.transport_context_fingerprint,
        None,
        None,
        expires_at,
    )
    .ok_or_else(|| AuthError::Nip98Invalid("invalid moderation proof binding".to_owned()))?;
    Ok(VerifiedModerationCommandProof { proof })
}

fn moderation_command_request_fingerprint(
    authorization_domain: buzz_core::CommunityId,
    actor: nostr::PublicKey,
    event_id: [u8; 32],
    kind: u32,
    target_fingerprint: [u8; 32],
) -> [u8; 32] {
    framed_digest(
        b"buzz:nip-fi:moderation-command-request:v1",
        &[
            authorization_domain.as_uuid().as_bytes(),
            actor.as_bytes(),
            &event_id,
            &kind.to_be_bytes(),
            &target_fingerprint,
        ],
    )
}

fn validate_moderation_proof_time(
    event: &Event,
    authoritative_now: DateTime<Utc>,
    tolerance_secs: u64,
) -> Result<(), AuthError> {
    let now = u64::try_from(authoritative_now.timestamp())
        .map_err(|_| AuthError::Nip98Invalid("invalid verifier time".to_owned()))?;
    if now.abs_diff(event.created_at.as_secs()) > tolerance_secs {
        return Err(AuthError::Nip98Invalid(
            "moderation proof timestamp is stale".to_owned(),
        ));
    }
    Ok(())
}

fn proof_expiry(
    event: &Event,
    authoritative_now: DateTime<Utc>,
    tolerance_secs: u64,
) -> Result<DateTime<Utc>, AuthError> {
    let expiry = DateTime::from_timestamp(
        i64::try_from(event.created_at.as_secs())
            .map_err(|_| AuthError::Nip98Invalid("invalid moderation proof time".to_owned()))?,
        0,
    )
    .and_then(|created| created.checked_add_signed(Duration::seconds(tolerance_secs as i64)))
    .ok_or_else(|| AuthError::Nip98Invalid("invalid moderation proof time".to_owned()))?;
    if authoritative_now >= expiry {
        return Err(AuthError::Nip98Invalid(
            "moderation proof is expired".to_owned(),
        ));
    }
    Ok(expiry)
}

impl VerifiedNip98InviteClaimProof {
    /// Exact actor proven by the signed invite-claim event.
    pub const fn actor_pubkey(&self) -> nostr::PublicKey {
        self.proof.actor_pubkey()
    }

    /// Consume the purpose-sealed wrapper at the canonical database boundary.
    pub fn into_verified_nostr_proof(self) -> VerifiedNostrProof {
        self.proof
    }
}

impl std::fmt::Debug for VerifiedNip98InviteClaimProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VerifiedNip98InviteClaimProof([REDACTED])")
    }
}

/// Verify one strict invite-claim proof and mint provider-free Nostr evidence.
///
/// Exactly one URL, method, and payload tag is required. The signed event and
/// independently verified assertion must name the same actor, domain, target,
/// request, and transport context before a proof can be constructed.
pub fn verify_nip98_invite_claim_proof(
    event_json: &str,
    coordinates: &Nip98InviteClaimCoordinates,
    body: &[u8],
    assertion: &VerifiedFederatedAssertion,
    authoritative_now: DateTime<Utc>,
) -> Result<VerifiedNip98InviteClaimProof, AuthError> {
    let event: Event = serde_json::from_str(event_json)
        .map_err(|_| AuthError::Nip98Invalid("invalid invite proof JSON".to_owned()))?;
    if event.kind != Kind::HttpAuth || !event.content.is_empty() {
        return Err(AuthError::Nip98Invalid(
            "invalid invite proof kind or content".to_owned(),
        ));
    }
    buzz_core::verify_event(&event)
        .map_err(|_| AuthError::Nip98Invalid("invalid invite proof signature".to_owned()))?;
    let now = u64::try_from(authoritative_now.timestamp())
        .map_err(|_| AuthError::Nip98Invalid("invalid verifier time".to_owned()))?;
    if now.abs_diff(event.created_at.as_secs()) > TIMESTAMP_TOLERANCE_SECS {
        return Err(AuthError::Nip98Invalid(
            "invite proof timestamp is stale".to_owned(),
        ));
    }

    let signed_url = exact_tag(
        &event,
        TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::U)),
        "u",
    )?;
    let signed_method = exact_tag(&event, TagKind::Method, "method")?;
    let signed_payload = exact_tag(&event, TagKind::Payload, "payload")?;
    let (canonical_signed_url, signed_host) = strict_canonical_url(signed_url)?;
    let payload_sha256: [u8; 32] = Sha256::digest(body).into();
    if canonical_signed_url != coordinates.canonical_url.as_ref()
        || signed_host != coordinates.canonical_host.as_ref()
        || signed_method != "POST"
        || payload_sha256 != coordinates.payload_sha256
        || signed_payload != hex::encode(payload_sha256)
    {
        return Err(AuthError::Nip98Invalid(
            "invite proof request binding mismatch".to_owned(),
        ));
    }

    let (transport, assertion_request, assertion_target, assertion_context) =
        assertion.request_binding();
    let (request, target, context) = coordinates.request_binding();
    let (assertion_not_before, assertion_expires_at) = assertion.time_bounds();
    if assertion.authorization_domain() != coordinates.authorization_domain
        || assertion.attested_event_author_pubkey() != Some(event.pubkey)
        || transport != ProofTransport::Nip98
        || assertion_request != request
        || assertion_target != target
        || assertion_context != context
        || authoritative_now < assertion_not_before
        || authoritative_now >= assertion_expires_at
    {
        return Err(AuthError::Nip98Invalid(
            "invite assertion binding mismatch".to_owned(),
        ));
    }

    let event_expires_at = DateTime::from_timestamp(
        i64::try_from(event.created_at.as_secs())
            .map_err(|_| AuthError::Nip98Invalid("invalid invite proof time".to_owned()))?,
        0,
    )
    .and_then(|created| {
        created.checked_add_signed(Duration::seconds(TIMESTAMP_TOLERANCE_SECS as i64))
    })
    .ok_or_else(|| AuthError::Nip98Invalid("invalid invite proof time".to_owned()))?;
    let expires_at = event_expires_at.min(assertion_expires_at);
    if authoritative_now >= expires_at {
        return Err(AuthError::Nip98Invalid(
            "invite proof is expired".to_owned(),
        ));
    }
    let proof = VerifiedNostrProof::from_verifier(
        coordinates.authorization_domain,
        event.pubkey,
        ProofTransport::Nip98,
        *request,
        *target,
        *context,
        Some(*assertion.assertion_fingerprint()),
        None,
        expires_at,
    )
    .ok_or_else(|| AuthError::Nip98Invalid("invalid invite proof binding".to_owned()))?;
    Ok(VerifiedNip98InviteClaimProof { proof })
}

fn exact_tag<'a>(event: &'a Event, kind: TagKind<'_>, name: &str) -> Result<&'a str, AuthError> {
    let mut matching = event.tags.iter().filter(|tag| tag.kind() == kind);
    let tag = matching
        .next()
        .filter(|_| matching.next().is_none())
        .ok_or_else(|| AuthError::Nip98Invalid(format!("invalid {name} tag cardinality")))?;
    tag.content()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AuthError::Nip98Invalid(format!("invalid {name} tag content")))
}

fn strict_canonical_url(raw: &str) -> Result<(String, String), AuthError> {
    let parsed =
        Url::parse(raw).map_err(|_| AuthError::Nip98Invalid("invalid canonical URL".to_owned()))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
    {
        return Err(AuthError::Nip98Invalid("invalid canonical URL".to_owned()));
    }
    if parsed.as_str() != raw {
        return Err(AuthError::Nip98Invalid("non-canonical URL".to_owned()));
    }
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| AuthError::Nip98Invalid("invalid canonical URL".to_owned()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AuthError::Nip98Invalid("invalid canonical URL".to_owned()))?;
    let authority = format!("{host}:{port}");
    Ok((parsed.to_string(), authority))
}

fn strict_websocket_origin(raw: &str) -> Result<String, AuthError> {
    let parsed = Url::parse(raw)
        .map_err(|_| AuthError::Nip98Invalid("invalid WebSocket origin".to_owned()))?;
    if !matches!(parsed.scheme(), "ws" | "wss")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
        || parsed.path() != "/"
        || parsed.as_str().trim_end_matches('/') != raw.trim_end_matches('/')
    {
        return Err(AuthError::Nip98Invalid(
            "invalid WebSocket origin".to_owned(),
        ));
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

fn framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

/// Normalize a URL for comparison.
///
/// - Lowercases scheme and host (already done by the `url` crate).
/// - Strips trailing slash from path.
///
/// **No loopback aliasing.** `localhost`, `::1`, and `127.0.0.1` are three
/// distinct hosts here. Under multi-tenant the `u`-tag host is the row-zero
/// community binding (`docs/multi-tenant-conformance.md`, NIP-98 row): if
/// `verify_nip98_event` collapses them, an event signed for `localhost`
/// would pass against a `127.0.0.1`-resolved community (or vice versa) —
/// a host-binding side door. Tests reconstruct `expected_url` from their
/// own bound host, the same shape production does.
fn normalize_url(raw: &str) -> String {
    let mut parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_lowercase(),
    };
    let path = parsed.path().trim_end_matches('/').to_string();
    parsed.set_path(&path);
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::FederatedPrincipal;
    use buzz_core::CommunityId;
    use nostr::{EventBuilder, Keys, Kind, Timestamp};
    use uuid::Uuid;

    const TEST_URL: &str = "https://relay.example.com/api/tokens";
    const TEST_METHOD: &str = "POST";

    fn make_nip98_event(
        keys: &Keys,
        url: &str,
        method: &str,
        payload_hex: Option<&str>,
        created_at: Option<Timestamp>,
    ) -> String {
        use nostr::Tag;

        let mut tags = vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
        ];
        if let Some(hex) = payload_hex {
            tags.push(Tag::parse(["payload", hex]).unwrap());
        }

        let mut builder = EventBuilder::new(Kind::HttpAuth, "").tags(tags);
        if let Some(ts) = created_at {
            builder = builder.custom_created_at(ts);
        }
        let event = builder.sign_with_keys(keys).expect("sign");
        serde_json::to_string(&event).expect("serialize")
    }

    fn invite_assertion(
        coordinates: &Nip98InviteClaimCoordinates,
        actor: nostr::PublicKey,
        now: DateTime<Utc>,
    ) -> VerifiedFederatedAssertion {
        let (request, target, context) = coordinates.request_binding();
        VerifiedFederatedAssertion::from_verifier(
            coordinates.authorization_domain(),
            FederatedPrincipal::from_verified_parts(
                "https://issuer.example".to_owned(),
                "opaque-subject".to_owned(),
            )
            .expect("test principal"),
            ProofTransport::Nip98,
            [7; 32],
            *target,
            *request,
            *context,
            now - Duration::seconds(1),
            now + Duration::minutes(5),
        )
        .expect("test assertion")
        .with_test_attested_event_author(actor)
    }

    #[test]
    fn valid_event_returns_pubkey() {
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, None, None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(result.is_ok(), "verify failed: {:?}", result.err());
        assert_eq!(result.unwrap(), keys.public_key());
    }

    #[test]
    fn wrong_kind_rejected() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        let json = serde_json::to_string(&event).unwrap();
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn expired_timestamp_rejected() {
        let keys = Keys::generate();
        let old_ts = Timestamp::from(Timestamp::now().as_secs().saturating_sub(120));
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, None, Some(old_ts));
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn url_mismatch_rejected() {
        let keys = Keys::generate();
        let json = make_nip98_event(
            &keys,
            "https://other.example.com/api/tokens",
            TEST_METHOD,
            None,
            None,
        );
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn method_mismatch_rejected() {
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, "GET", None, None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn method_case_insensitive() {
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, "post", None, None);
        let result = verify_nip98_event(&json, TEST_URL, "POST", None);
        assert!(result.is_ok());
    }

    #[test]
    fn payload_tag_correct_hash_passes() {
        let keys = Keys::generate();
        let body = b"hello world";
        let hash: [u8; 32] = Sha256::digest(body).into();
        let hash_hex = hex::encode(hash);
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, Some(&hash_hex), None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, Some(body));
        assert!(result.is_ok());
    }

    #[test]
    fn payload_tag_wrong_hash_rejected() {
        let keys = Keys::generate();
        let body = b"hello world";
        let wrong_hex = "deadbeef".repeat(8); // 64 hex chars but wrong hash
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, Some(&wrong_hex), None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, Some(body));
        assert!(matches!(result, Err(AuthError::Nip98Invalid(_))));
    }

    #[test]
    fn payload_tag_absent_with_body_passes() {
        // payload tag is optional per spec; clients SHOULD include it but it's not required
        let keys = Keys::generate();
        let json = make_nip98_event(&keys, TEST_URL, TEST_METHOD, None, None);
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, Some(b"some body"));
        assert!(result.is_ok());
    }

    #[test]
    fn trailing_slash_normalized() {
        let keys = Keys::generate();
        let url_with_slash = "https://relay.example.com/api/tokens/";
        let json = make_nip98_event(&keys, url_with_slash, TEST_METHOD, None, None);
        // expected_url without trailing slash — should still match
        let result = verify_nip98_event(&json, TEST_URL, TEST_METHOD, None);
        assert!(result.is_ok());
    }

    #[test]
    fn loopback_aliases_are_distinct_hosts() {
        // Under multi-tenant, the `u`-tag host is the row-zero community
        // binding. An event signed for `localhost` MUST NOT pass against an
        // expected URL on `127.0.0.1` (or `::1`) — collapsing the three would
        // be a host-check side door. Production reconstructs `expected_url`
        // from the community-bound host; tests do the same.
        let keys = Keys::generate();
        let localhost_url = "http://localhost:3000/api/tokens";
        let loopback_url = "http://127.0.0.1:3000/api/tokens";
        let json = make_nip98_event(&keys, localhost_url, TEST_METHOD, None, None);
        let result = verify_nip98_event(&json, loopback_url, TEST_METHOD, None);
        assert!(
            matches!(result, Err(AuthError::Nip98Invalid(_))),
            "localhost u-tag must NOT match a 127.0.0.1 expected_url; got {result:?}"
        );

        // Symmetric: signed-for-127.0.0.1 against expected localhost — same answer.
        let json2 = make_nip98_event(&keys, loopback_url, TEST_METHOD, None, None);
        let result2 = verify_nip98_event(&json2, localhost_url, TEST_METHOD, None);
        assert!(
            matches!(result2, Err(AuthError::Nip98Invalid(_))),
            "127.0.0.1 u-tag must NOT match a localhost expected_url; got {result2:?}"
        );

        // And identity still holds — same host on both sides verifies.
        let json3 = make_nip98_event(&keys, loopback_url, TEST_METHOD, None, None);
        assert!(verify_nip98_event(&json3, loopback_url, TEST_METHOD, None).is_ok());
    }

    #[test]
    fn invite_claim_proof_requires_exact_non_substitutable_coordinates() {
        let keys = Keys::generate();
        let now = Utc::now();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let body = br#"{"code":"v2.opaque"}"#;
        let coordinates = Nip98InviteClaimCoordinates::new(
            domain,
            [21; 32],
            "https://relay.example.com/api/invites/claim",
            body,
        )
        .expect("invite coordinates");
        let assertion = invite_assertion(&coordinates, keys.public_key(), now);
        let payload = hex::encode(Sha256::digest(body));
        let event = make_nip98_event(
            &keys,
            "https://relay.example.com/api/invites/claim",
            "POST",
            Some(&payload),
            Some(Timestamp::from(now.timestamp() as u64)),
        );

        let proof = verify_nip98_invite_claim_proof(&event, &coordinates, body, &assertion, now)
            .expect("exact invite proof");
        assert_eq!(proof.actor_pubkey(), keys.public_key());
        let proof = proof.into_verified_nostr_proof();
        assert_eq!(proof.authorization_domain(), domain);
        assert_eq!(proof.transport(), ProofTransport::Nip98);

        let substituted_target = Nip98InviteClaimCoordinates::new(
            domain,
            [22; 32],
            "https://relay.example.com/api/invites/claim",
            body,
        )
        .expect("substituted target coordinates");
        assert!(verify_nip98_invite_claim_proof(
            &event,
            &substituted_target,
            body,
            &assertion,
            now,
        )
        .is_err());

        let wrong_method = make_nip98_event(
            &keys,
            "https://relay.example.com/api/invites/claim",
            "PUT",
            Some(&payload),
            Some(Timestamp::from(now.timestamp() as u64)),
        );
        assert!(verify_nip98_invite_claim_proof(
            &wrong_method,
            &coordinates,
            body,
            &assertion,
            now,
        )
        .is_err());
        let wrong_url = make_nip98_event(
            &keys,
            "https://relay.example.com/api/invites/claim/",
            "POST",
            Some(&payload),
            Some(Timestamp::from(now.timestamp() as u64)),
        );
        assert!(
            verify_nip98_invite_claim_proof(&wrong_url, &coordinates, body, &assertion, now,)
                .is_err()
        );
        let malformed_duplicate = EventBuilder::new(Kind::HttpAuth, "")
            .tags(vec![
                nostr::Tag::parse(["u", "https://relay.example.com/api/invites/claim"])
                    .expect("u tag"),
                nostr::Tag::parse(["u"]).expect("malformed duplicate u tag"),
                nostr::Tag::parse(["method", "POST"]).expect("method tag"),
                nostr::Tag::parse(["payload", payload.as_str()]).expect("payload tag"),
            ])
            .custom_created_at(Timestamp::from(now.timestamp() as u64))
            .sign_with_keys(&keys)
            .expect("sign malformed duplicate event");
        let malformed_duplicate =
            serde_json::to_string(&malformed_duplicate).expect("serialize malformed event");
        assert!(verify_nip98_invite_claim_proof(
            &malformed_duplicate,
            &coordinates,
            body,
            &assertion,
            now,
        )
        .is_err());
        assert!(verify_nip98_invite_claim_proof(
            &event,
            &coordinates,
            br#"{"code":"v2.other"}"#,
            &assertion,
            now,
        )
        .is_err());
    }

    #[test]
    fn moderation_proofs_are_exact_and_cross_transport_replay_stable() {
        let keys = Keys::generate();
        let other_keys = Keys::generate();
        let now = Utc::now();
        let created_at = Timestamp::from(now.timestamp() as u64);
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let target = [41; 32];
        let target_hex = hex::encode(target);
        let command = EventBuilder::new(Kind::from(9040_u16), "")
            .tag(nostr::Tag::parse(["p", target_hex.as_str()]).expect("target tag"))
            .custom_created_at(created_at)
            .sign_with_keys(&keys)
            .expect("sign moderation command");
        let body = serde_json::to_vec(&command).expect("serialize moderation command");
        let url = "https://relay.example.com/events";
        let payload = hex::encode(Sha256::digest(&body));
        let http_coordinates =
            Nip98ModerationCommandCoordinates::new(domain, target, url, &body, &command)
                .expect("HTTP moderation coordinates");
        let http_event = make_nip98_event(&keys, url, "POST", Some(&payload), Some(created_at));
        let http_proof =
            verify_nip98_moderation_command_proof(&http_event, &http_coordinates, &body, now)
                .expect("exact HTTP moderation proof");

        let ws_coordinates = Nip42ModerationCommandCoordinates::new(
            domain,
            target,
            "wss://relay.example.com",
            &command,
        )
        .expect("WebSocket moderation coordinates");
        let ws_proof = verify_nip42_moderation_command_proof(&command, &ws_coordinates, now)
            .expect("exact WebSocket moderation proof");
        assert_eq!(
            http_proof.request_fingerprint(),
            ws_proof.request_fingerprint()
        );
        assert_eq!(
            http_proof.target_fingerprint(),
            ws_proof.target_fingerprint()
        );
        assert_eq!(http_proof.actor_pubkey(), ws_proof.actor_pubkey());
        assert_ne!(
            http_proof.transport_context_fingerprint(),
            ws_proof.transport_context_fingerprint()
        );

        let no_payload = make_nip98_event(&keys, url, "POST", None, Some(created_at));
        assert!(
            verify_nip98_moderation_command_proof(&no_payload, &http_coordinates, &body, now,)
                .is_err()
        );
        let wrong_body = serde_json::to_vec(
            &EventBuilder::new(Kind::from(9040_u16), "")
                .tag(nostr::Tag::parse(["p", target_hex.as_str()]).expect("target tag"))
                .custom_created_at(created_at)
                .sign_with_keys(&keys)
                .expect("sign substituted command"),
        )
        .expect("serialize substituted command");
        assert!(verify_nip98_moderation_command_proof(
            &http_event,
            &http_coordinates,
            &wrong_body,
            now,
        )
        .is_err());
        let wrong_signer =
            make_nip98_event(&other_keys, url, "POST", Some(&payload), Some(created_at));
        assert!(verify_nip98_moderation_command_proof(
            &wrong_signer,
            &http_coordinates,
            &body,
            now,
        )
        .is_err());
        assert!(verify_nip42_moderation_command_proof(
            &command,
            &ws_coordinates,
            now + Duration::seconds(121),
        )
        .is_err());
    }
}
