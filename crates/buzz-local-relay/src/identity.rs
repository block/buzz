//! Laptop cryptographic identity adapter.
//!
//! NIP-42 and NIP-98 establish Nostr principals. A small fail-closed policy
//! binds direct writes to their authors, protects private read kinds, and binds
//! configured replication streams to stable relay-node principals.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use buzz_auth::{verify_nip42_event_at, verify_nip98_event_at, AuthError};
use buzz_core::filter::reader_authorized_for_event;
use buzz_core::identity::{
    AppendAuthorizer, AppendContext, AuthenticatedPrincipal, AuthenticationMethod,
    AuthorizationDecision, IdentityAuthenticator, IdentityDenialCode, Principal, ReadAuthorizer,
    ReadOperation, ReplicationPeerAuthenticator, ReplicationPeerBinding,
};
use buzz_core::kind::{
    event_kind_u32, AUTHOR_ONLY_KINDS, KIND_AUTH, KIND_GIFT_WRAP, KIND_HTTP_AUTH, P_GATED_KINDS,
};
use buzz_core::replication::ReplicationSourceId;
use nostr::{Alphabet, Event, EventId, Filter, PublicKey, SingleLetterTag, TagKind, Timestamp};
use thiserror::Error;

const LOCAL_DESTINATION_SCOPE: &str = "local";

/// How long a consumed proof ID is retained. Anything older is unusable by
/// freshness alone, so retention only needs to comfortably exceed the
/// NIP-42/NIP-98 timestamp tolerance.
const PROOF_RETENTION_SECS: u64 = 600;

/// Authentication evidence understood by the laptop adapter.
#[derive(Debug, Clone)]
pub enum LocalAuthenticationEvidence {
    /// NIP-42 challenge-response for one WebSocket connection.
    Nip42 {
        /// Signed kind `22242` authentication event.
        event: Event,
        /// Challenge issued by this relay connection.
        challenge: String,
    },
    /// NIP-98 authentication for one HTTP request.
    Nip98 {
        /// JSON-encoded signed kind `27235` event.
        event_json: String,
        /// HTTP method bound into the proof.
        method: String,
        /// Exact request body covered by the payload tag.
        body: Vec<u8>,
    },
}

/// NIP-42-style proof from a configured relay peer.
#[derive(Debug, Clone)]
pub struct LocalPeerEvidence {
    /// Signed challenge-response event.
    pub event: Event,
    /// Destination-issued challenge.
    pub challenge: String,
}

/// Destination-controlled trust for one replication source.
#[derive(Debug, Clone)]
pub struct RelayPeerTrust {
    source: ReplicationSourceId,
    relay_node_principal: String,
    verification_methods: HashMap<PublicKey, String>,
}

impl RelayPeerTrust {
    /// Binds a source and stable node principal to active Nostr verification keys.
    pub fn new(
        source: ReplicationSourceId,
        relay_node_principal: impl Into<String>,
        verification_methods: impl IntoIterator<Item = (PublicKey, String)>,
    ) -> Self {
        Self {
            source,
            relay_node_principal: relay_node_principal.into(),
            verification_methods: verification_methods.into_iter().collect(),
        }
    }
}

/// Fail-closed identity adapter error.
#[derive(Debug, Error)]
pub enum LocalIdentityError {
    /// Cryptographic evidence or local policy denied the operation.
    #[error("{code}: identity operation denied")]
    Denied {
        /// Stable denial category.
        code: IdentityDenialCode,
    },
    /// Adapter state could not be read safely.
    #[error("identity adapter failed: {0}")]
    Internal(String),
}

impl LocalIdentityError {
    /// Creates a fail-closed denial.
    pub const fn denied(code: IdentityDenialCode) -> Self {
        Self::Denied { code }
    }

    /// Returns the stable denial code when this is a policy rejection.
    pub const fn denial_code(&self) -> Option<IdentityDenialCode> {
        match self {
            Self::Denied { code } => Some(*code),
            Self::Internal(_) => None,
        }
    }
}

/// Consumed-proof replay state, optionally persisted beside the event log.
///
/// The persisted record contains proof IDs and consumption times only —
/// never challenges, signatures, or other reusable material — and is
/// separate security state, not event history.
struct ProofLedger {
    consumed: HashMap<EventId, u64>,
    store_path: Option<PathBuf>,
}

impl ProofLedger {
    fn load(store_path: Option<PathBuf>) -> Result<Self, LocalIdentityError> {
        let mut consumed = HashMap::new();
        if let Some(path) = store_path.as_ref() {
            if path.exists() {
                let contents = std::fs::read_to_string(path)
                    .map_err(|error| LocalIdentityError::Internal(error.to_string()))?;
                for line in contents.lines().filter(|line| !line.is_empty()) {
                    let (id, timestamp) = line.split_once(' ').ok_or_else(|| {
                        LocalIdentityError::Internal("malformed proof store line".to_string())
                    })?;
                    let id = EventId::from_hex(id)
                        .map_err(|error| LocalIdentityError::Internal(error.to_string()))?;
                    let timestamp = timestamp
                        .parse::<u64>()
                        .map_err(|error| LocalIdentityError::Internal(error.to_string()))?;
                    consumed.insert(id, timestamp);
                }
            }
        }
        Ok(Self {
            consumed,
            store_path,
        })
    }

    fn consume(
        &mut self,
        event_id: EventId,
        evaluation_time: Timestamp,
    ) -> Result<(), LocalIdentityError> {
        let now = evaluation_time.as_secs();
        self.consumed
            .retain(|_, consumed_at| *consumed_at + PROOF_RETENTION_SECS >= now);
        if self.consumed.contains_key(&event_id) {
            return Err(LocalIdentityError::denied(
                IdentityDenialCode::ReplayDetected,
            ));
        }
        // The durable record must exist before the proof is accepted, so a
        // crash or restart inside the freshness window still fails closed.
        if let Some(path) = self.store_path.as_ref() {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| LocalIdentityError::Internal(error.to_string()))?;
            file.write_all(format!("{} {now}\n", event_id.to_hex()).as_bytes())
                .and_then(|()| file.sync_data())
                .map_err(|error| LocalIdentityError::Internal(error.to_string()))?;
        }
        self.consumed.insert(event_id, now);
        Ok(())
    }
}

/// Laptop reference implementation of portable identity ports.
///
/// Peer trust is in-memory deployment configuration. Consumed-proof replay
/// state is in-memory by default and durable when a proof store path is
/// supplied, so a relay restart within the proof freshness window still
/// rejects replayed evidence. No database, external identity provider, or
/// DID resolver is introduced.
pub struct LocalIdentityAdapter {
    consumed_proofs: Mutex<ProofLedger>,
    peer_trust: HashMap<ReplicationSourceId, RelayPeerTrust>,
}

impl LocalIdentityAdapter {
    /// Creates a secured local adapter with no trusted replication peers.
    pub fn new() -> Self {
        Self {
            consumed_proofs: Mutex::new(ProofLedger {
                consumed: HashMap::new(),
                store_path: None,
            }),
            peer_trust: HashMap::new(),
        }
    }

    /// Creates an adapter whose consumed-proof state persists at `path`.
    pub fn with_proof_store(path: impl AsRef<Path>) -> Result<Self, LocalIdentityError> {
        Self::with_peer_trust_and_proof_store([], Some(path))
    }

    /// Creates an adapter with peer bindings and optional durable proof state.
    pub fn with_peer_trust_and_proof_store(
        peer_trust: impl IntoIterator<Item = RelayPeerTrust>,
        proof_store: Option<impl AsRef<Path>>,
    ) -> Result<Self, LocalIdentityError> {
        Ok(Self {
            consumed_proofs: Mutex::new(ProofLedger::load(
                proof_store.map(|path| path.as_ref().to_path_buf()),
            )?),
            peer_trust: peer_trust
                .into_iter()
                .map(|trust| (trust.source.clone(), trust))
                .collect(),
        })
    }

    /// Binds an already-authenticated verification key to a source stream.
    ///
    /// Used by transports whose request evidence (for example payload-bound
    /// NIP-98) has independently proven control of `pubkey`; this applies the
    /// destination-controlled trust configuration only.
    pub fn bind_peer_key(
        &self,
        source: &ReplicationSourceId,
        pubkey: &PublicKey,
    ) -> Result<ReplicationPeerBinding, LocalIdentityError> {
        let trust = self
            .peer_trust
            .get(source)
            .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::PeerUnbound))?;
        let verification_method = trust
            .verification_methods
            .get(pubkey)
            .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::SourceMismatch))?;
        Ok(ReplicationPeerBinding {
            source: source.clone(),
            relay_node_principal: trust.relay_node_principal.clone(),
            verification_method: verification_method.clone(),
        })
    }

    /// Creates an adapter with explicit destination-controlled peer bindings.
    pub fn with_peer_trust(peer_trust: impl IntoIterator<Item = RelayPeerTrust>) -> Self {
        Self {
            consumed_proofs: Mutex::new(ProofLedger {
                consumed: HashMap::new(),
                store_path: None,
            }),
            peer_trust: peer_trust
                .into_iter()
                .map(|trust| (trust.source.clone(), trust))
                .collect(),
        }
    }

    /// Authenticates evidence at an explicit evaluation time.
    pub fn authenticate_at(
        &self,
        evidence: LocalAuthenticationEvidence,
        audience: &str,
        evaluation_time: Timestamp,
    ) -> Result<AuthenticatedPrincipal, LocalIdentityError> {
        match evidence {
            LocalAuthenticationEvidence::Nip42 { event, challenge } => {
                verify_nip42_event_at(&event, &challenge, audience, evaluation_time)
                    .map_err(map_nip42_error)?;
                self.consume_proof(event.id, evaluation_time)?;
                Ok(authenticated_nostr_principal(
                    event.pubkey,
                    AuthenticationMethod::Nip42,
                    audience,
                    evaluation_time,
                    event.id,
                ))
            }
            LocalAuthenticationEvidence::Nip98 {
                event_json,
                method,
                body,
            } => {
                let event: Event = serde_json::from_str(&event_json)
                    .map_err(|_| LocalIdentityError::denied(IdentityDenialCode::InvalidEvidence))?;
                if event.tags.find(TagKind::Payload).is_none() {
                    return Err(LocalIdentityError::denied(
                        IdentityDenialCode::InvalidEvidence,
                    ));
                }
                let pubkey = verify_nip98_event_at(
                    &event_json,
                    audience,
                    &method,
                    Some(&body),
                    evaluation_time,
                )
                .map_err(map_nip98_error)?;
                self.consume_proof(event.id, evaluation_time)?;
                Ok(authenticated_nostr_principal(
                    pubkey,
                    AuthenticationMethod::Nip98,
                    audience,
                    evaluation_time,
                    event.id,
                ))
            }
        }
    }

    /// Authenticates and binds a relay peer at an explicit evaluation time.
    pub fn authenticate_peer_at(
        &self,
        evidence: LocalPeerEvidence,
        audience: &str,
        source: &ReplicationSourceId,
        evaluation_time: Timestamp,
    ) -> Result<ReplicationPeerBinding, LocalIdentityError> {
        let trust = self
            .peer_trust
            .get(source)
            .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::PeerUnbound))?;
        verify_nip42_event_at(
            &evidence.event,
            &evidence.challenge,
            audience,
            evaluation_time,
        )
        .map_err(map_nip42_error)?;
        let verification_method = trust
            .verification_methods
            .get(&evidence.event.pubkey)
            .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::SourceMismatch))?;
        self.consume_proof(evidence.event.id, evaluation_time)?;
        Ok(ReplicationPeerBinding {
            source: source.clone(),
            relay_node_principal: trust.relay_node_principal.clone(),
            verification_method: verification_method.clone(),
        })
    }

    /// Applies direct-append policy for the local destination scope.
    pub fn authorize_direct(
        &self,
        principal: &AuthenticatedPrincipal,
        event: &Event,
    ) -> AuthorizationDecision {
        self.authorize_append(
            principal,
            &AppendContext::Direct,
            event,
            LOCAL_DESTINATION_SCOPE,
        )
    }

    /// Applies request-level read policy for the local destination scope.
    pub fn authorize_local_query(
        &self,
        principal: &AuthenticatedPrincipal,
        operation: ReadOperation,
        filters: &[Filter],
    ) -> AuthorizationDecision {
        self.authorize_query(principal, operation, filters, LOCAL_DESTINATION_SCOPE)
    }

    /// Applies result-level disclosure policy for the local destination scope.
    pub fn authorize_local_event(
        &self,
        principal: &AuthenticatedPrincipal,
        operation: ReadOperation,
        event: &Event,
    ) -> AuthorizationDecision {
        self.authorize_event_delivery(principal, operation, event, LOCAL_DESTINATION_SCOPE)
    }

    fn consume_proof(
        &self,
        event_id: EventId,
        evaluation_time: Timestamp,
    ) -> Result<(), LocalIdentityError> {
        let mut ledger = self
            .consumed_proofs
            .lock()
            .map_err(|error| LocalIdentityError::Internal(error.to_string()))?;
        ledger.consume(event_id, evaluation_time)
    }
}

impl Default for LocalIdentityAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentityAuthenticator for LocalIdentityAdapter {
    type Evidence = LocalAuthenticationEvidence;
    type Error = LocalIdentityError;

    async fn authenticate(
        &self,
        evidence: Self::Evidence,
        audience: &str,
    ) -> Result<AuthenticatedPrincipal, Self::Error> {
        self.authenticate_at(evidence, audience, Timestamp::now())
    }
}

impl AppendAuthorizer for LocalIdentityAdapter {
    fn authorize_append(
        &self,
        principal: &AuthenticatedPrincipal,
        context: &AppendContext,
        event: &Event,
        _destination_scope: &str,
    ) -> AuthorizationDecision {
        match context {
            AppendContext::Direct => {
                let Some(pubkey) = principal.principal.nostr_pubkey() else {
                    return AuthorizationDecision::denied(IdentityDenialCode::AuthorMismatch);
                };
                let kind = event_kind_u32(event);
                if kind == KIND_AUTH || kind == KIND_HTTP_AUTH {
                    return AuthorizationDecision::denied(IdentityDenialCode::ScopeDenied);
                }
                if event.pubkey == *pubkey
                    || (kind == KIND_GIFT_WRAP && event_has_valid_recipient(event))
                {
                    AuthorizationDecision::Allowed
                } else {
                    AuthorizationDecision::denied(IdentityDenialCode::AuthorMismatch)
                }
            }
            AppendContext::Replication { .. } => {
                if principal.principal.relay_node_id().is_some() {
                    AuthorizationDecision::Allowed
                } else {
                    AuthorizationDecision::denied(IdentityDenialCode::PeerUnbound)
                }
            }
            AppendContext::System => AuthorizationDecision::denied(IdentityDenialCode::ScopeDenied),
        }
    }
}

impl ReadAuthorizer for LocalIdentityAdapter {
    fn authorize_query(
        &self,
        _principal: &AuthenticatedPrincipal,
        _operation: ReadOperation,
        _filters: &[Filter],
        _destination_scope: &str,
    ) -> AuthorizationDecision {
        AuthorizationDecision::Allowed
    }

    fn authorize_event_delivery(
        &self,
        principal: &AuthenticatedPrincipal,
        _operation: ReadOperation,
        event: &Event,
        _destination_scope: &str,
    ) -> AuthorizationDecision {
        let Some(reader) = principal.principal.nostr_pubkey() else {
            return AuthorizationDecision::denied(IdentityDenialCode::EventDisclosureDenied);
        };
        let kind = event_kind_u32(event);
        if AUTHOR_ONLY_KINDS.contains(&kind) && event.pubkey != *reader {
            return AuthorizationDecision::denied(IdentityDenialCode::EventDisclosureDenied);
        }
        let reader_hex = reader.to_hex();
        if P_GATED_KINDS.contains(&kind) && !event_has_recipient(event, &reader_hex) {
            return AuthorizationDecision::denied(IdentityDenialCode::EventDisclosureDenied);
        }
        if !reader_authorized_for_event(event, &reader_hex) {
            return AuthorizationDecision::denied(IdentityDenialCode::EventDisclosureDenied);
        }
        AuthorizationDecision::Allowed
    }
}

impl ReplicationPeerAuthenticator for LocalIdentityAdapter {
    type Evidence = LocalPeerEvidence;
    type Error = LocalIdentityError;

    async fn authenticate_peer(
        &self,
        evidence: Self::Evidence,
        audience: &str,
        source: &ReplicationSourceId,
    ) -> Result<ReplicationPeerBinding, Self::Error> {
        self.authenticate_peer_at(evidence, audience, source, Timestamp::now())
    }
}

fn authenticated_nostr_principal(
    pubkey: PublicKey,
    method: AuthenticationMethod,
    audience: &str,
    evaluation_time: Timestamp,
    proof_id: EventId,
) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal: Principal::Nostr { pubkey },
        method,
        audience: audience.to_string(),
        authenticated_at: evaluation_time.as_secs(),
        expires_at: None,
        proof_id: Some(proof_id.to_hex()),
    }
}

fn event_has_recipient(event: &Event, reader_pubkey_hex: &str) -> bool {
    let p = SingleLetterTag::lowercase(Alphabet::P);
    event
        .tags
        .filter(TagKind::SingleLetter(p))
        .any(|tag| tag.content() == Some(reader_pubkey_hex))
}

fn event_has_valid_recipient(event: &Event) -> bool {
    let p = SingleLetterTag::lowercase(Alphabet::P);
    event
        .tags
        .filter(TagKind::SingleLetter(p))
        .filter_map(|tag| tag.content())
        .any(|value| PublicKey::from_hex(value).is_ok())
}

fn map_nip42_error(error: AuthError) -> LocalIdentityError {
    let code = match error {
        AuthError::EventExpired => IdentityDenialCode::EvidenceExpired,
        AuthError::ChallengeMismatch | AuthError::RelayUrlMismatch => {
            IdentityDenialCode::AudienceMismatch
        }
        AuthError::InvalidSignature | AuthError::PubkeyMismatch => {
            IdentityDenialCode::InvalidEvidence
        }
        _ => IdentityDenialCode::InvalidEvidence,
    };
    LocalIdentityError::denied(code)
}

fn map_nip98_error(error: AuthError) -> LocalIdentityError {
    let code = match error {
        AuthError::Nip98Replay => IdentityDenialCode::ReplayDetected,
        AuthError::Nip98Invalid(message) if message.contains("timestamp") => {
            IdentityDenialCode::EvidenceExpired
        }
        AuthError::Nip98Invalid(message)
            if message.contains("URL mismatch") || message.contains("method mismatch") =>
        {
            IdentityDenialCode::AudienceMismatch
        }
        _ => IdentityDenialCode::InvalidEvidence,
    };
    LocalIdentityError::denied(code)
}

#[cfg(test)]
mod tests {
    use buzz_core::identity::{AppendContext, AuthorizationDecision};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    #[test]
    fn direct_policy_binds_event_author_but_allows_gift_wrap_indirection() {
        let adapter = LocalIdentityAdapter::new();
        let caller = Keys::generate();
        let other = Keys::generate();
        let principal = AuthenticatedPrincipal {
            principal: Principal::Nostr {
                pubkey: caller.public_key(),
            },
            method: AuthenticationMethod::TrustedLocal,
            audience: "local".to_string(),
            authenticated_at: 0,
            expires_at: None,
            proof_id: None,
        };
        let own = EventBuilder::new(Kind::TextNote, "own")
            .sign_with_keys(&caller)
            .expect("event signs");
        let mismatched = EventBuilder::new(Kind::TextNote, "other")
            .sign_with_keys(&other)
            .expect("event signs");
        let gift_wrap = EventBuilder::new(Kind::GiftWrap, "ciphertext")
            .tag(Tag::public_key(caller.public_key()))
            .sign_with_keys(&other)
            .expect("gift wrap signs");
        let malformed_gift_wrap = EventBuilder::new(Kind::GiftWrap, "ciphertext")
            .sign_with_keys(&other)
            .expect("gift wrap signs");

        assert!(adapter
            .authorize_append(&principal, &AppendContext::Direct, &own, "local")
            .is_allowed());
        assert_eq!(
            adapter.authorize_append(&principal, &AppendContext::Direct, &mismatched, "local"),
            AuthorizationDecision::denied(IdentityDenialCode::AuthorMismatch)
        );
        assert!(adapter
            .authorize_append(&principal, &AppendContext::Direct, &gift_wrap, "local")
            .is_allowed());
        assert_eq!(
            adapter.authorize_append(
                &principal,
                &AppendContext::Direct,
                &malformed_gift_wrap,
                "local",
            ),
            AuthorizationDecision::denied(IdentityDenialCode::AuthorMismatch)
        );
    }

    #[test]
    fn private_recipient_gate_blocks_known_id_bypass() {
        let adapter = LocalIdentityAdapter::new();
        let recipient = Keys::generate();
        let attacker = Keys::generate();
        let event = EventBuilder::new(Kind::GiftWrap, "ciphertext")
            .tag(Tag::public_key(recipient.public_key()))
            .sign_with_keys(&Keys::generate())
            .expect("gift wrap signs");
        let principal = |keys: &Keys| AuthenticatedPrincipal {
            principal: Principal::Nostr {
                pubkey: keys.public_key(),
            },
            method: AuthenticationMethod::TrustedLocal,
            audience: "local".to_string(),
            authenticated_at: 0,
            expires_at: None,
            proof_id: None,
        };

        assert!(adapter
            .authorize_local_event(&principal(&recipient), ReadOperation::Query, &event)
            .is_allowed());
        assert_eq!(
            adapter.authorize_local_event(&principal(&attacker), ReadOperation::Query, &event),
            AuthorizationDecision::denied(IdentityDenialCode::EventDisclosureDenied)
        );
    }
}
