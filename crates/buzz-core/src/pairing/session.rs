//! NIP-AB pairing session state machine.
//!
//! A [`PairingSession`] tracks the protocol state for one side of a device
//! pairing exchange. It is pure computation — no I/O, no async. The caller
//! is responsible for relay communication and user interaction.
//!
//! # Protocol flow
//!
//! ```text
//! Source                              Target
//! ──────                              ──────
//! new_source(relay)                   (scan QR)
//!   → (session, qr_payload)          new_target(&qr)
//!                                      → (session, offer_event)
//! handle_offer(&event)
//!   → sas_code (display it)          (display sas_code from session)
//!
//! [user confirms SAS match]
//!
//! confirm_sas()
//!   → sas_confirm_event              handle_sas_confirm(&event)
//!                                      → sas_code (verify it)
//! send_payload(type, data)
//!   → payload_event                  handle_payload(&event)
//!                                      → (type, data)
//!                                    send_complete()
//! handle_complete(&event)              → complete_event
//! ```

use std::collections::HashSet;
use std::time::{Duration, Instant};

use nostr::nips::nip44;
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag};
use zeroize::{Zeroize, Zeroizing};

use super::crypto::{ct_eq, derive_sas, derive_session_id, derive_transcript_hash, format_sas};
use super::qr::{self, QrPayload};
use super::types::{AbortReason, PairingMessage, PayloadType};
use super::PairingError;

/// Default session timeout: 120 seconds from creation.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// NIP-AB event kind (from the kind registry).
const PAIRING_KIND: u16 = crate::kind::KIND_PAIRING as u16;

// ── Public types ──────────────────────────────────────────────────────────────

/// Which role this device plays in the pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The device that holds the secret and initiates pairing.
    Source,
    /// The device that scans the QR code and receives the secret.
    Target,
}

/// Protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session created, QR displayed (source) or offer sent (target).
    Waiting,
    /// SAS code displayed, awaiting user confirmation (source side).
    Confirming,
    /// Target received `sas-confirm`, awaiting explicit user approval.
    /// The target must call [`PairingSession::confirm_target_sas`] to proceed.
    AwaitingConfirmation,
    /// SAS confirmed, payload in transit.
    Transferring,
    /// Payload has been sent (source) or received (target); awaiting completion.
    PayloadExchanged,
    /// Protocol completed successfully.
    Completed,
    /// Session aborted by either side.
    Aborted,
}

/// A NIP-AB device pairing session.
///
/// Tracks protocol state for one side of the exchange. All methods that
/// produce [`Event`]s return them for the caller to publish; all methods
/// that consume events take a reference. No I/O happens inside.
pub struct PairingSession {
    role: Role,
    state: SessionState,
    /// Ephemeral keypair for this session (discarded after).
    keys: Keys,
    /// 32-byte session secret from the QR code.
    session_secret: [u8; 32],
    /// Relay URLs for this session.
    relay_urls: Vec<String>,
    /// Peer's ephemeral public key.
    /// Source learns this from the offer; target learns it from the QR code.
    peer_pubkey: Option<PublicKey>,
    /// Derived session ID (HKDF of session_secret).
    session_id: [u8; 32],
    /// SAS code (set after ECDH + HKDF).
    sas_code: Option<u32>,
    /// Raw SAS input bytes (needed for transcript hash).
    sas_input: Option<[u8; 32]>,
    /// Event IDs already processed in this session (NIP-AB §Duplicate Event Handling).
    /// Duplicates are silently discarded to handle relay re-delivery.
    processed_ids: HashSet<[u8; 32]>,
    /// When the session was created.
    created_at: Instant,
    /// Maximum session lifetime.
    timeout: Duration,
}

// ── Source-side constructors and methods ───────────────────────────────────────

impl PairingSession {
    /// Create a new source session. Returns the session and a QR payload
    /// to display to the user.
    pub fn new_source(relay_url: String) -> (Self, QrPayload) {
        let keys = Keys::generate();
        let mut session_secret = [0u8; 32];
        rand::fill(&mut session_secret);

        let session_id = derive_session_id(&session_secret);

        let qr = QrPayload {
            version: 1,
            source_pubkey: keys.public_key(),
            session_secret,
            relays: vec![relay_url.clone()],
        };

        let session = Self {
            role: Role::Source,
            state: SessionState::Waiting,
            keys,
            session_secret,
            relay_urls: vec![relay_url],
            peer_pubkey: None,
            session_id,
            sas_code: None,
            sas_input: None,
            processed_ids: HashSet::new(),
            created_at: Instant::now(),
            timeout: DEFAULT_TIMEOUT,
        };

        (session, qr)
    }

    /// (Source) Process an incoming offer event from the target.
    ///
    /// Validates the session ID, computes ECDH + SAS, and returns the
    /// formatted SAS code to display. After this call the session is in
    /// [`SessionState::Confirming`].
    pub fn handle_offer(&mut self, event: &Event) -> Result<String, PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::Waiting)?;
        self.expect_role(Role::Source)?;
        self.validate_event_basics(event)?;

        let msg = self.decrypt_message(event)?;
        let (session_id_hex, version) = match &msg {
            PairingMessage::Offer {
                session_id,
                version,
            } => (session_id.clone(), *version),
            other => return Err(unexpected("offer", other)),
        };

        // Reject unsupported protocol versions (NIP-AB §Versions).
        if version != 1 {
            return Err(PairingError::UnexpectedMessage {
                expected: "version 1".into(),
                got: format!("version {version}"),
            });
        }

        // Verify session_id matches our derivation (constant-time).
        let received_id = hex::decode(&session_id_hex)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());
        match received_id {
            Some(ref id) if ct_eq(id, &self.session_id) => {}
            _ => return Err(PairingError::InvalidSessionId),
        }

        // Lock to this peer.
        let peer = event.pubkey;
        self.peer_pubkey = Some(peer);

        // Compute ECDH and SAS. Zero the ECDH shared secret after derivation.
        let mut ecdh = nostr::util::generate_shared_key(self.keys.secret_key(), &peer)
            .map_err(|e| PairingError::SigningError(e.to_string()))?;
        let (code, sas_input) = derive_sas(&ecdh, &self.session_secret);
        ecdh.zeroize();
        self.sas_code = Some(code);
        self.sas_input = Some(sas_input);
        self.state = SessionState::Confirming;
        self.record_event(event);

        Ok(format_sas(code))
    }

    /// (Source) User confirmed the SAS codes match. Build the `sas-confirm`
    /// event to publish.
    pub fn confirm_sas(&mut self) -> Result<Event, PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::Confirming)?;
        self.expect_role(Role::Source)?;

        let sas_input = self.sas_input.ok_or(PairingError::SasMismatch)?;
        let peer = self
            .peer_pubkey
            .ok_or(PairingError::InvalidPubkey("no peer".into()))?;

        let transcript_hash = derive_transcript_hash(
            &self.session_id,
            &self.keys.public_key().to_bytes(),
            &peer.to_bytes(),
            &sas_input,
            &self.session_secret,
        );

        let msg = PairingMessage::SasConfirm {
            transcript_hash: hex::encode(transcript_hash),
        };
        let event = self.build_event(&msg)?;
        self.state = SessionState::Transferring;
        Ok(event)
    }

    /// (Source) Build the payload event carrying the secret.
    pub fn send_payload(
        &mut self,
        payload_type: PayloadType,
        payload: Zeroizing<String>,
    ) -> Result<Event, PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::Transferring)?;
        self.expect_role(Role::Source)?;

        let mut msg = PairingMessage::Payload {
            payload_type,
            payload: (*payload).clone(),
        };
        // Defer `?` so the transient clone is zeroized on both success and error.
        let result = self.build_event(&msg);
        if let PairingMessage::Payload {
            ref mut payload, ..
        } = msg
        {
            payload.zeroize();
        }
        let event = result?;
        self.state = SessionState::PayloadExchanged;
        Ok(event)
    }

    /// (Source) Process the `complete` event from the target.
    pub fn handle_complete(&mut self, event: &Event) -> Result<(), PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::PayloadExchanged)?;
        self.expect_role(Role::Source)?;
        self.validate_event_from_peer(event)?;

        let msg = self.decrypt_message(event)?;
        match msg {
            PairingMessage::Complete { success: true } => {
                self.state = SessionState::Completed;
                self.record_event(event);
                Ok(())
            }
            PairingMessage::Complete { success: false } => {
                self.state = SessionState::Aborted;
                // Not recorded: the message was received but not "successfully
                // processed" per NIP-AB §Duplicate Event Handling. The session
                // is terminal (Aborted) so no future handler can accept events.
                Err(PairingError::UnexpectedMessage {
                    expected: "complete(success=true)".into(),
                    got: "complete(success=false)".into(),
                })
            }
            other => Err(unexpected("complete", &other)),
        }
    }
}

// ── Target-side constructors and methods ──────────────────────────────────────

impl PairingSession {
    /// Create a new target session from a scanned QR payload.
    ///
    /// Returns the session and the `offer` event to publish.
    pub fn new_target(qr: &QrPayload) -> Result<(Self, Event), PairingError> {
        let keys = Keys::generate();
        let session_id = derive_session_id(&qr.session_secret);

        // Compute ECDH and SAS immediately (target knows source pubkey from QR).
        // Zero the ECDH shared secret after derivation.
        let mut ecdh = nostr::util::generate_shared_key(keys.secret_key(), &qr.source_pubkey)
            .map_err(|e| PairingError::SigningError(e.to_string()))?;
        let (code, sas_input) = derive_sas(&ecdh, &qr.session_secret);
        ecdh.zeroize();

        let mut session = Self {
            role: Role::Target,
            state: SessionState::Waiting,
            keys,
            session_secret: qr.session_secret,
            relay_urls: qr.relays.clone(),
            peer_pubkey: Some(qr.source_pubkey),
            session_id,
            sas_code: Some(code),
            sas_input: Some(sas_input),
            processed_ids: HashSet::new(),
            created_at: Instant::now(),
            timeout: DEFAULT_TIMEOUT,
        };

        // Build and return the offer event.
        let msg = PairingMessage::Offer {
            session_id: hex::encode(session_id),
            version: 1,
        };
        let event = session.build_event(&msg)?;
        session.state = SessionState::Confirming;

        Ok((session, event))
    }

    /// (Target) Process the `sas-confirm` event from the source.
    ///
    /// Verifies the transcript hash and returns the SAS code for the user
    /// to visually confirm. The session moves to [`SessionState::AwaitingConfirmation`]
    /// — the caller **must** call [`confirm_target_sas`] after the user approves
    /// before any payload can be received.
    pub fn handle_sas_confirm(&mut self, event: &Event) -> Result<String, PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::Confirming)?;
        self.expect_role(Role::Target)?;
        self.validate_event_from_peer(event)?;

        let msg = self.decrypt_message(event)?;
        let received_hash = match &msg {
            PairingMessage::SasConfirm { transcript_hash } => transcript_hash.clone(),
            other => return Err(unexpected("sas-confirm", other)),
        };

        // Compute our own transcript hash and compare.
        let sas_input = self.sas_input.ok_or(PairingError::SasMismatch)?;
        let peer = self
            .peer_pubkey
            .ok_or(PairingError::InvalidPubkey("no peer".into()))?;

        // Source pubkey is the peer (we're target).
        let expected_hash = derive_transcript_hash(
            &self.session_id,
            &peer.to_bytes(),
            &self.keys.public_key().to_bytes(),
            &sas_input,
            &self.session_secret,
        );

        // Constant-time comparison to prevent timing side-channels.
        let received_bytes = hex::decode(&received_hash)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());
        let matches = received_bytes
            .as_ref()
            .is_some_and(|rb| ct_eq(rb, &expected_hash));
        if !matches {
            self.state = SessionState::Aborted;
            return Err(PairingError::TranscriptMismatch);
        }

        self.state = SessionState::AwaitingConfirmation;
        self.record_event(event);
        let code = self.sas_code.ok_or(PairingError::SasMismatch)?;
        Ok(format_sas(code))
    }

    /// (Target) User confirmed the SAS codes match. Transitions to
    /// [`SessionState::Transferring`] so payloads can be received.
    pub fn confirm_target_sas(&mut self) -> Result<(), PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::AwaitingConfirmation)?;
        self.expect_role(Role::Target)?;
        self.state = SessionState::Transferring;
        Ok(())
    }

    /// (Target) Process the payload event from the source.
    ///
    /// Only one payload is accepted per session — after this call the state
    /// advances to [`SessionState::PayloadExchanged`].
    pub fn handle_payload(
        &mut self,
        event: &Event,
    ) -> Result<(PayloadType, Zeroizing<String>), PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::Transferring)?;
        self.expect_role(Role::Target)?;
        self.validate_event_from_peer(event)?;

        let msg = self.decrypt_message(event)?;
        match msg {
            PairingMessage::Payload {
                payload_type,
                payload,
            } => {
                self.state = SessionState::PayloadExchanged;
                self.record_event(event);
                Ok((payload_type, Zeroizing::new(payload)))
            }
            other => Err(unexpected("payload", &other)),
        }
    }

    /// (Target) Build the `complete` event to publish.
    pub fn send_complete(&mut self) -> Result<Event, PairingError> {
        self.check_expired()?;
        self.expect_state(SessionState::PayloadExchanged)?;
        self.expect_role(Role::Target)?;

        let msg = PairingMessage::Complete { success: true };
        let event = self.build_event(&msg)?;
        self.state = SessionState::Completed;
        Ok(event)
    }
}

// ── Shared methods ────────────────────────────────────────────────────────────

impl PairingSession {
    /// Build an abort event. Returns `None` if no peer is known yet
    /// (nothing to encrypt to), but still transitions to [`SessionState::Aborted`].
    ///
    /// Rejects calls from terminal states ([`SessionState::Completed`] /
    /// [`SessionState::Aborted`]) — a finished session cannot be regressed.
    pub fn abort(&mut self, reason: AbortReason) -> Result<Option<Event>, PairingError> {
        if matches!(self.state, SessionState::Completed | SessionState::Aborted) {
            return Err(PairingError::UnexpectedMessage {
                expected: "non-terminal state".into(),
                got: format!("state {:?}", self.state),
            });
        }
        if self.peer_pubkey.is_none() {
            self.state = SessionState::Aborted;
            return Ok(None);
        }
        let msg = PairingMessage::Abort { reason };
        let event = self.build_event(&msg)?;
        self.state = SessionState::Aborted;
        Ok(Some(event))
    }

    /// Process an abort event from the peer.
    pub fn handle_abort(&mut self, event: &Event) -> Result<AbortReason, PairingError> {
        // Terminal states are final — ignore late aborts.
        if matches!(self.state, SessionState::Completed | SessionState::Aborted) {
            return Err(PairingError::UnexpectedMessage {
                expected: "non-terminal state".into(),
                got: format!("state {:?}", self.state),
            });
        }
        // Require a known peer — an anonymous abort before the offer is
        // accepted could let any relay observer kill the session.
        if self.peer_pubkey.is_none() {
            return Err(PairingError::InvalidPubkey(
                "cannot accept abort before peer is known".into(),
            ));
        }
        self.validate_event_from_peer(event)?;
        let msg = self.decrypt_message(event)?;
        match msg {
            PairingMessage::Abort { reason } => {
                self.state = SessionState::Aborted;
                self.record_event(event);
                Ok(reason)
            }
            other => Err(unexpected("abort", &other)),
        }
    }

    /// Check if the session has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.timeout
    }

    /// Current protocol state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// This device's role.
    pub fn role(&self) -> Role {
        self.role
    }

    /// This session's ephemeral public key.
    pub fn pubkey(&self) -> PublicKey {
        self.keys.public_key()
    }

    /// Relay URLs for this session.
    pub fn relay_urls(&self) -> &[String] {
        &self.relay_urls
    }

    /// The SAS code, if computed.
    pub fn sas_code(&self) -> Option<String> {
        self.sas_code.map(format_sas)
    }

    /// Sign an arbitrary event builder with this session's ephemeral keys.
    ///
    /// Useful for relay-level operations like NIP-42 authentication, where
    /// the relay requires events to be signed by the same key that
    /// authenticated the connection.
    pub fn sign_event(&self, builder: EventBuilder) -> Result<Event, PairingError> {
        builder
            .sign_with_keys(&self.keys)
            .map_err(|e| PairingError::SigningError(e.to_string()))
    }

    /// The QR URI for this session (source only).
    pub fn qr_uri(&self) -> Option<String> {
        if self.role != Role::Source {
            return None;
        }
        Some(qr::encode_qr(&QrPayload {
            version: 1,
            source_pubkey: self.keys.public_key(),
            session_secret: self.session_secret,
            relays: self.relay_urls.clone(),
        }))
    }
}

// ── Test-only accessors ───────────────────────────────────────────────────────

#[cfg(test)]
impl PairingSession {
    /// Returns `true` if the given event ID has been recorded as processed.
    ///
    /// Test-only: allows assertions about the dedup set without exposing
    /// `processed_ids` through the public API.
    fn has_processed(&self, event: &Event) -> bool {
        self.processed_ids.contains(&event.id.to_bytes())
    }

    /// Override the session timeout for testing.
    fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

impl PairingSession {
    /// Encrypt a message and wrap it in a signed kind:24134 event.
    ///
    /// # Secret handling
    ///
    /// The serialized JSON plaintext is explicitly zeroized after encryption.
    /// The caller's `Zeroizing<String>` zeros on drop. The transient clone
    /// inside `PairingMessage::Payload` is zeroized by `send_payload` after
    /// this method returns.
    ///
    /// Residual transient copies that cannot be zeroized:
    /// 1. `serde_json::to_string` may create intermediate buffers during serialization
    /// 2. `nip44::encrypt` reads the plaintext but does not zero its internal copy
    ///
    /// These are inherent to Rust's heap allocator and third-party crate internals.
    fn build_event(&self, message: &PairingMessage) -> Result<Event, PairingError> {
        let peer = self
            .peer_pubkey
            .ok_or_else(|| PairingError::InvalidPubkey("no peer pubkey set".into()))?;

        let mut plaintext = serde_json::to_string(message)?;
        let encrypted = nip44::encrypt(
            self.keys.secret_key(),
            &peer,
            &plaintext,
            nip44::Version::V2,
        )?;
        plaintext.zeroize(); // Zero serialized JSON before drop

        // NIP-AB §: Implementations SHOULD set created_at to the current time
        // minus a random value between 0 and 30 seconds for metadata privacy.
        let now = nostr::Timestamp::now().as_secs();
        let jitter = rand::random::<u64>() % 31; // 0-30s jitter per NIP-AB §Metadata Privacy
        let ts = nostr::Timestamp::from(now.saturating_sub(jitter));

        EventBuilder::new(Kind::Custom(PAIRING_KIND), &encrypted)
            .tags([Tag::public_key(peer)])
            .custom_created_at(ts)
            .sign_with_keys(&self.keys)
            .map_err(|e| PairingError::SigningError(e.to_string()))
    }

    /// Decrypt and parse a NIP-44 encrypted pairing message from an event.
    ///
    /// NIP-AB §Event Validation: `content` MUST be a valid NIP-44 v2 payload
    /// (base64, 132–87472 characters). Reject before attempting decryption.
    fn decrypt_message(&self, event: &Event) -> Result<PairingMessage, PairingError> {
        // NIP-AB §Event Validation step 5: reject content outside NIP-44 size range.
        let content_len = event.content.len();
        if !(132..=87472).contains(&content_len) {
            return Err(PairingError::UnexpectedMessage {
                expected: "NIP-44 content (132–87472 chars)".into(),
                got: format!("{content_len} chars"),
            });
        }

        let mut decrypted = nip44::decrypt(
            self.keys.secret_key(),
            &event.pubkey,
            event.content.as_str(),
        )?;

        // NIP-AB §Payload: decrypted plaintext MUST NOT exceed 65,535 bytes.
        if decrypted.len() > 65_535 {
            decrypted.zeroize();
            return Err(PairingError::UnexpectedMessage {
                expected: "plaintext ≤ 65535 bytes".into(),
                got: format!("{} bytes", decrypted.len()),
            });
        }

        // Defer `?` so decrypted plaintext is zeroized on both success and parse failure.
        let result = serde_json::from_str(&decrypted);
        decrypted.zeroize();
        Ok(result?)
    }

    /// Validate basic event properties: kind, p-tag, and duplicate ID.
    ///
    /// NIP-AB §Duplicate Event Handling: silently discard events whose `id`
    /// has already been processed in this session. The set is bounded by the
    /// session lifetime (120 s max, ~6 events in a normal flow).
    ///
    /// This method only *checks* for duplicates — it does not record the ID.
    /// Call [`record_event`] after the message is fully accepted.
    fn validate_event_basics(&self, event: &Event) -> Result<(), PairingError> {
        // NIP-01 §: Validate the event id and sig.
        event
            .verify()
            .map_err(|e| PairingError::InvalidPubkey(format!("event verification failed: {e}")))?;

        // Duplicate event ID check (NIP-AB §Duplicate Event Handling).
        if self.processed_ids.contains(&event.id.to_bytes()) {
            return Err(PairingError::UnexpectedMessage {
                expected: "new event".into(),
                got: "duplicate event id".into(),
            });
        }

        if event.kind != Kind::Custom(PAIRING_KIND) {
            return Err(PairingError::UnexpectedMessage {
                expected: format!("kind {PAIRING_KIND}"),
                got: format!("kind {}", event.kind.as_u16()),
            });
        }

        // Check p-tag points to us.
        let our_pk = self.keys.public_key();
        let has_p_tag = event.tags.iter().any(|t| {
            t.as_slice().first().map(|s| s.as_str()) == Some("p")
                && t.as_slice()
                    .get(1)
                    .map(|s| s.as_str() == our_pk.to_hex().as_str())
                    .unwrap_or(false)
        });
        if !has_p_tag {
            return Err(PairingError::InvalidPubkey(
                "event p-tag does not match our ephemeral pubkey".into(),
            ));
        }

        Ok(())
    }

    /// Record an event ID as successfully processed.
    ///
    /// Called by each handler only after the message has been fully validated,
    /// decrypted, type-checked, and accepted. This ensures that speculative
    /// probes (e.g., `handle_abort` used to detect aborts) do not poison the
    /// duplicate set for subsequent handlers.
    fn record_event(&mut self, event: &Event) {
        self.processed_ids.insert(event.id.to_bytes());
    }

    /// Validate that the event is from the expected peer.
    fn validate_event_from_peer(&self, event: &Event) -> Result<(), PairingError> {
        self.validate_event_basics(event)?;

        if let Some(expected) = self.peer_pubkey {
            if event.pubkey != expected {
                return Err(PairingError::InvalidPubkey(format!(
                    "event from {} but expected {}",
                    event.pubkey.to_hex(),
                    expected.to_hex()
                )));
            }
        }

        Ok(())
    }

    /// Check that the session hasn't expired.
    fn check_expired(&self) -> Result<(), PairingError> {
        if self.is_expired() {
            return Err(PairingError::SessionExpired);
        }
        Ok(())
    }

    /// Check that we're in the expected state.
    fn expect_state(&self, expected: SessionState) -> Result<(), PairingError> {
        if self.state != expected {
            return Err(PairingError::UnexpectedMessage {
                expected: format!("state {:?}", expected),
                got: format!("state {:?}", self.state),
            });
        }
        Ok(())
    }

    /// Check that we're playing the expected role.
    fn expect_role(&self, expected: Role) -> Result<(), PairingError> {
        if self.role != expected {
            return Err(PairingError::UnexpectedMessage {
                expected: format!("role {:?}", expected),
                got: format!("role {:?}", self.role),
            });
        }
        Ok(())
    }
}

/// Zero sensitive fields on drop using `zeroize` to prevent dead-store
/// elimination by the compiler. Ephemeral private keys are separately
/// zeroed by `nostr::SecretKey::Drop` (which uses `write_volatile`).
impl Drop for PairingSession {
    fn drop(&mut self) {
        self.session_secret.zeroize();
        self.session_id.zeroize();
        if let Some(ref mut input) = self.sas_input {
            input.zeroize();
        }
    }
}

/// Helper to build an UnexpectedMessage error from a PairingMessage variant.
fn unexpected(expected: &str, got: &PairingMessage) -> PairingError {
    let got_name = match got {
        PairingMessage::Offer { .. } => "offer",
        PairingMessage::SasConfirm { .. } => "sas-confirm",
        PairingMessage::Payload { .. } => "payload",
        PairingMessage::Complete { .. } => "complete",
        PairingMessage::Abort { .. } => "abort",
    };
    PairingError::UnexpectedMessage {
        expected: expected.into(),
        got: got_name.into(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Full happy-path: source creates → target joins → SAS match → payload → complete.
    #[test]
    fn happy_path_full_protocol() {
        // Source creates session.
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        assert_eq!(source.state(), SessionState::Waiting);
        assert_eq!(source.role(), Role::Source);

        // Target scans QR and creates session + offer event.
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target creation");
        assert_eq!(target.state(), SessionState::Confirming);
        assert_eq!(target.role(), Role::Target);

        // Source processes offer.
        let source_sas = source.handle_offer(&offer_event).expect("handle offer");
        assert_eq!(source.state(), SessionState::Confirming);

        // Target already has SAS from construction.
        let target_sas = target.sas_code().expect("target should have SAS");

        // SAS codes must match (proves no MITM).
        assert_eq!(source_sas, target_sas, "SAS codes must match");
        assert_eq!(source_sas.len(), 6, "SAS must be 6 digits");

        // Source confirms SAS → sends sas-confirm event.
        let sas_confirm_event = source.confirm_sas().expect("confirm SAS");
        assert_eq!(source.state(), SessionState::Transferring);

        // Target verifies sas-confirm — enters AwaitingConfirmation.
        let target_sas_verify = target
            .handle_sas_confirm(&sas_confirm_event)
            .expect("handle sas-confirm");
        assert_eq!(target_sas_verify, target_sas);
        assert_eq!(target.state(), SessionState::AwaitingConfirmation);

        // Target user confirms the SAS.
        target.confirm_target_sas().expect("target confirms SAS");
        assert_eq!(target.state(), SessionState::Transferring);

        // Source sends payload.
        let payload_event = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1test".into()))
            .expect("send payload");
        assert_eq!(source.state(), SessionState::PayloadExchanged);

        // Target receives payload.
        let (pt, data) = target
            .handle_payload(&payload_event)
            .expect("handle payload");
        assert_eq!(pt, PayloadType::Nsec);
        assert_eq!(*data, "nsec1test");
        assert_eq!(target.state(), SessionState::PayloadExchanged);

        // Target sends complete.
        let complete_event = target.send_complete().expect("send complete");
        assert_eq!(target.state(), SessionState::Completed);

        // Source handles complete.
        source
            .handle_complete(&complete_event)
            .expect("handle complete");
        assert_eq!(source.state(), SessionState::Completed);
    }

    /// State machine rejects out-of-order operations.
    #[test]
    fn reject_out_of_order_operations() {
        let (mut source, _qr) = PairingSession::new_source("wss://relay.test".into());

        // Can't confirm SAS before receiving offer.
        assert!(source.confirm_sas().is_err());

        // Can't send payload before confirming SAS.
        assert!(source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1x".into()))
            .is_err());
    }

    /// Abort from either side.
    #[test]
    fn abort_flow() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // Source must first learn the peer pubkey (from the offer) to send an abort.
        let _sas = source.handle_offer(&offer_event).expect("handle offer");

        // Source aborts.
        let abort_event = source
            .abort(AbortReason::UserDenied)
            .expect("source abort")
            .expect("should have event since peer is known");
        assert_eq!(source.state(), SessionState::Aborted);

        // Target handles abort.
        let reason = target.handle_abort(&abort_event).expect("handle abort");
        assert_eq!(reason, AbortReason::UserDenied);
        assert_eq!(target.state(), SessionState::Aborted);
    }

    /// Abort before peer is known returns None (no event to send).
    #[test]
    fn abort_without_peer_returns_none() {
        let (mut source, _qr) = PairingSession::new_source("wss://relay.test".into());
        let result = source.abort(AbortReason::Timeout).expect("abort");
        assert!(result.is_none(), "no event when peer is unknown");
        assert_eq!(source.state(), SessionState::Aborted);
    }

    /// Local abort() cannot regress a Completed session.
    #[test]
    fn local_abort_after_completed_is_rejected() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer) = PairingSession::new_target(&qr).expect("target");
        let _ = source.handle_offer(&offer).expect("offer");
        let sas_confirm = source.confirm_sas().expect("confirm");
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm");
        target.confirm_target_sas().expect("target confirm");
        let payload = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("x".into()))
            .expect("payload");
        let _ = target.handle_payload(&payload).expect("handle payload");
        let complete = target.send_complete().expect("complete");
        source.handle_complete(&complete).expect("handle complete");

        assert_eq!(source.state(), SessionState::Completed);
        // Local abort must be rejected.
        let result = source.abort(AbortReason::UserDenied);
        assert!(result.is_err(), "abort after Completed must fail");
        assert_eq!(
            source.state(),
            SessionState::Completed,
            "state must not regress"
        );
    }

    /// handle_abort() before peer is known is rejected (prevents relay-observer DoS).
    #[test]
    fn reject_handle_abort_before_peer_known() {
        let (mut source, _qr) = PairingSession::new_source("wss://relay.test".into());
        // Build a fake abort event from an unknown sender.
        let rogue = Keys::generate();
        let msg = PairingMessage::Abort {
            reason: AbortReason::Timeout,
        };
        let plaintext = serde_json::to_string(&msg).unwrap();
        let encrypted = nip44::encrypt(
            rogue.secret_key(),
            &source.pubkey(),
            &plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let fake_abort =
            EventBuilder::new(Kind::Custom(crate::kind::KIND_PAIRING as u16), &encrypted)
                .tags([Tag::public_key(source.pubkey())])
                .sign_with_keys(&rogue)
                .unwrap();

        // Source has no peer yet — must reject.
        let result = source.handle_abort(&fake_abort);
        assert!(result.is_err(), "abort before peer known must be rejected");
        assert_eq!(
            source.state(),
            SessionState::Waiting,
            "state must not change"
        );
    }

    /// Late abort after session is completed is rejected.
    #[test]
    fn reject_abort_after_completed() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // Run the full happy path to completion.
        let _ = source.handle_offer(&offer_event).expect("offer");
        let sas_confirm = source.confirm_sas().expect("confirm");
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm");
        target.confirm_target_sas().expect("target confirm");
        let payload = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1test".into()))
            .expect("payload");
        let _ = target.handle_payload(&payload).expect("handle payload");
        let complete = target.send_complete().expect("complete");
        source.handle_complete(&complete).expect("handle complete");

        assert_eq!(source.state(), SessionState::Completed);
        assert_eq!(target.state(), SessionState::Completed);

        // Build a fake abort event from the target to the source.
        let abort_event = {
            let keys = Keys::generate();
            let msg = PairingMessage::Abort {
                reason: AbortReason::Timeout,
            };
            let plaintext = serde_json::to_string(&msg).unwrap();
            let encrypted = nip44::encrypt(
                keys.secret_key(),
                &source.pubkey(),
                &plaintext,
                nip44::Version::V2,
            )
            .unwrap();
            EventBuilder::new(Kind::Custom(crate::kind::KIND_PAIRING as u16), &encrypted)
                .tags([Tag::public_key(source.pubkey())])
                .sign_with_keys(&keys)
                .unwrap()
        };

        // Source should reject the late abort.
        let result = source.handle_abort(&abort_event);
        assert!(
            result.is_err(),
            "late abort after Completed must be rejected"
        );
        // State must remain Completed.
        assert_eq!(source.state(), SessionState::Completed);
    }

    /// Invalid session_id in offer is rejected.
    #[test]
    fn reject_invalid_session_id() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());

        // Create a target with a DIFFERENT session secret (simulates attacker).
        let mut fake_qr = qr.clone();
        fake_qr.session_secret = [0xff; 32];
        let (_, fake_offer) = PairingSession::new_target(&fake_qr).expect("fake target");

        // Source should reject the offer (session_id won't match).
        let result = source.handle_offer(&fake_offer);
        assert!(
            matches!(result, Err(PairingError::InvalidSessionId)),
            "expected InvalidSessionId, got {result:?}"
        );
    }

    /// Event from wrong pubkey is rejected.
    #[test]
    fn reject_event_from_wrong_pubkey() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // Source accepts the legitimate offer.
        let _ = source.handle_offer(&offer_event).expect("handle offer");
        let sas_confirm = source.confirm_sas().expect("confirm");

        // Create a rogue session that tries to send a fake sas-confirm.
        let rogue_keys = Keys::generate();
        let fake_msg = PairingMessage::SasConfirm {
            transcript_hash: "00".repeat(32),
        };
        let plaintext = serde_json::to_string(&fake_msg).unwrap();
        let encrypted = nip44::encrypt(
            rogue_keys.secret_key(),
            &target.pubkey(),
            &plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let fake_event = EventBuilder::new(Kind::Custom(PAIRING_KIND), &encrypted)
            .tags([Tag::public_key(target.pubkey())])
            .sign_with_keys(&rogue_keys)
            .unwrap();

        // Target should reject (wrong author).
        let result = target.handle_sas_confirm(&fake_event);
        assert!(
            matches!(result, Err(PairingError::InvalidPubkey(_))),
            "expected InvalidPubkey, got {result:?}"
        );

        // But the legitimate sas-confirm should work.
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("legit sas-confirm");
    }

    /// QR URI round-trip through session constructors.
    #[test]
    fn qr_uri_round_trip() {
        let (source, qr) = PairingSession::new_source("wss://relay.test".into());
        let uri = source.qr_uri().expect("source should have QR URI");
        let decoded = qr::decode_qr(&uri).expect("decode QR URI");
        assert_eq!(decoded.source_pubkey, qr.source_pubkey);
        assert_eq!(decoded.session_secret, qr.session_secret);
        assert_eq!(decoded.relays, qr.relays);
    }

    /// Target cannot receive payload without explicit SAS confirmation.
    #[test]
    fn target_must_confirm_sas_before_payload() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        let _ = source.handle_offer(&offer_event).expect("offer");
        let sas_confirm_event = source.confirm_sas().expect("confirm");

        // Target receives sas-confirm → AwaitingConfirmation.
        let _ = target
            .handle_sas_confirm(&sas_confirm_event)
            .expect("sas-confirm");
        assert_eq!(target.state(), SessionState::AwaitingConfirmation);

        // Source sends payload.
        let payload_event = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1test".into()))
            .expect("payload");

        // Target tries to handle payload WITHOUT confirming SAS first → error.
        let result = target.handle_payload(&payload_event);
        assert!(
            result.is_err(),
            "should reject payload before SAS confirmation"
        );

        // Now confirm, then payload works.
        target.confirm_target_sas().expect("confirm");
        let (pt, _) = target
            .handle_payload(&payload_event)
            .expect("payload after confirm");
        assert_eq!(pt, PayloadType::Nsec);
    }

    /// Only one payload per session — duplicate sends/receives are rejected.
    #[test]
    fn reject_duplicate_payload() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        let _ = source.handle_offer(&offer_event).expect("offer");
        let sas_confirm = source.confirm_sas().expect("confirm");
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm");
        target.confirm_target_sas().expect("target confirm");

        // First payload succeeds.
        let payload1 = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1first".into()))
            .expect("first payload");

        // Second payload from source is rejected (state already advanced).
        let result = source.send_payload(PayloadType::Nsec, Zeroizing::new("nsec1second".into()));
        assert!(result.is_err(), "duplicate send_payload should fail");

        // Target receives first payload.
        let _ = target.handle_payload(&payload1).expect("receive first");

        // Target trying to receive again is rejected.
        let result = target.handle_payload(&payload1);
        assert!(result.is_err(), "duplicate handle_payload should fail");
    }

    /// Secrets are zeroed on drop.
    #[test]
    fn secrets_zeroed_on_drop() {
        let (session, _qr) = PairingSession::new_source("wss://relay.test".into());
        // We can't directly inspect after drop, but we verify the Drop impl
        // compiles and runs without panic.
        drop(session);
    }

    /// Expired sessions reject all operations with `SessionExpired`.
    #[test]
    fn expired_session_rejects_operations() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        source.set_timeout(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));

        assert!(source.is_expired());

        // Every handler that calls check_expired should fail.
        let (_, offer_event) = PairingSession::new_target(&qr).expect("target");
        let result = source.handle_offer(&offer_event);
        assert!(
            matches!(result, Err(PairingError::SessionExpired)),
            "expected SessionExpired, got {result:?}"
        );
    }

    /// Duplicate event IDs are silently discarded (NIP-AB §Duplicate Event Handling).
    #[test]
    fn duplicate_event_id_is_rejected() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // First offer succeeds.
        let _ = source.handle_offer(&offer_event).expect("first offer");

        // Run through the rest of the protocol so we can test duplicate
        // complete events on the source side.
        let sas_confirm = source.confirm_sas().expect("confirm");
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm");
        target.confirm_target_sas().expect("target confirm");
        let payload = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1test".into()))
            .expect("payload");
        let _ = target.handle_payload(&payload).expect("handle payload");
        let complete_event = target.send_complete().expect("complete");

        // First complete succeeds.
        source
            .handle_complete(&complete_event)
            .expect("first complete");

        // Second delivery of the same complete event (same event ID) — must
        // be rejected because the state has already advanced to Completed.
        let result = source.handle_complete(&complete_event);
        assert!(result.is_err(), "duplicate event ID must be rejected");
    }

    /// Speculative `handle_abort` on a non-abort event must NOT poison the
    /// duplicate set — the real handler must still accept the event.
    ///
    /// This mirrors the CLI's `check_for_abort()` pattern: every inbound
    /// event is first probed via `handle_abort()`, which fails for non-abort
    /// messages. The subsequent real handler must still see the event as new.
    #[test]
    fn speculative_abort_does_not_poison_dedup() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // Source accepts the offer (learns peer).
        let _ = source.handle_offer(&offer_event).expect("offer");
        let sas_confirm = source.confirm_sas().expect("confirm");

        // Target: speculative abort probe on the sas-confirm event.
        // This must fail (it's not an abort) WITHOUT recording the event ID.
        let probe = target.handle_abort(&sas_confirm);
        assert!(probe.is_err(), "sas-confirm is not an abort");

        // Target: real handler must still accept the same event.
        let sas = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm must succeed after speculative abort probe");
        assert_eq!(sas.len(), 6);
    }

    /// A wrong-type message that passes validation but fails at type-dispatch
    /// must NOT be recorded, so the event ID remains available for future use.
    ///
    /// Scenario: target is in `Transferring` (waiting for payload). Source
    /// accidentally sends a `complete` message instead. The target's
    /// `handle_payload` rejects it (wrong type), but the event ID must not
    /// be poisoned — the session should still accept the real payload.
    #[test]
    fn wrong_type_message_not_recorded() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // Drive to Transferring on both sides.
        let _ = source.handle_offer(&offer_event).expect("offer");
        let sas_confirm = source.confirm_sas().expect("confirm");
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm");
        target.confirm_target_sas().expect("target confirm");

        // Source sends the real payload (we'll use it later).
        let payload_event = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1test".into()))
            .expect("payload");

        // Build a wrong-type event: a `complete` message from source to target.
        // This passes kind/p-tag/peer validation but fails at type-dispatch
        // inside handle_payload (expects "payload", gets "complete").
        let wrong_type_msg = PairingMessage::Complete { success: true };
        let wrong_plaintext = serde_json::to_string(&wrong_type_msg).unwrap();
        let wrong_encrypted = nip44::encrypt(
            source.keys.secret_key(),
            &target.pubkey(),
            &wrong_plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let wrong_event = EventBuilder::new(Kind::Custom(PAIRING_KIND), &wrong_encrypted)
            .tags([Tag::public_key(target.pubkey())])
            .sign_with_keys(&source.keys)
            .unwrap();

        // Target tries to handle as payload — fails (wrong type).
        let result = target.handle_payload(&wrong_event);
        assert!(result.is_err(), "wrong-type message must be rejected");
        assert_eq!(
            target.state(),
            SessionState::Transferring,
            "state must not advance on wrong-type"
        );

        // The real payload must still be accepted (its ID was never recorded).
        let (pt, data) = target
            .handle_payload(&payload_event)
            .expect("real payload must succeed after wrong-type rejection");
        assert_eq!(pt, PayloadType::Nsec);
        assert_eq!(*data, "nsec1test");
    }

    /// `complete(success: false)` transitions to Aborted and does NOT
    /// record the event ID (the message was not "successfully processed"
    /// per NIP-AB §Duplicate Event Handling).
    #[test]
    fn complete_failure_aborts_without_recording() {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (mut target, offer_event) = PairingSession::new_target(&qr).expect("target");

        // Drive to PayloadExchanged on source side.
        let _ = source.handle_offer(&offer_event).expect("offer");
        let sas_confirm = source.confirm_sas().expect("confirm");
        let _ = target
            .handle_sas_confirm(&sas_confirm)
            .expect("sas-confirm");
        target.confirm_target_sas().expect("target confirm");
        let payload = source
            .send_payload(PayloadType::Nsec, Zeroizing::new("nsec1test".into()))
            .expect("payload");
        let _ = target.handle_payload(&payload).expect("handle payload");

        // Build a complete(success: false) event from target to source.
        let fail_msg = PairingMessage::Complete { success: false };
        let fail_plaintext = serde_json::to_string(&fail_msg).unwrap();
        let fail_encrypted = nip44::encrypt(
            target.keys.secret_key(),
            &source.pubkey(),
            &fail_plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let fail_event = EventBuilder::new(Kind::Custom(PAIRING_KIND), &fail_encrypted)
            .tags([Tag::public_key(source.pubkey())])
            .sign_with_keys(&target.keys)
            .unwrap();

        // Source handles complete(false) — should error and abort.
        let result = source.handle_complete(&fail_event);
        assert!(result.is_err(), "complete(false) must return error");
        assert_eq!(
            source.state(),
            SessionState::Aborted,
            "state must be Aborted after complete(false)"
        );

        // The failed event must NOT be in the processed set.
        assert!(
            !source.has_processed(&fail_event),
            "complete(false) must not record the event ID"
        );
    }
}
