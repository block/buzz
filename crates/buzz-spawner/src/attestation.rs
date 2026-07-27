//! The two-round NIP-OA attestation handshake, spawner side.
//!
//! The agent's secret key is minted here and never leaves the host, so the
//! spawner cannot produce its own owner attestation — a NIP-OA auth tag is
//! signed with the *owner's* secret key over the agent's pubkey. The owner must
//! therefore sign a key they did not generate, which they can only do after the
//! spawner tells them what it is.
//!
//! This module is deliberately free of I/O: [`evaluate_response`] is a pure
//! function over a stored record and an inbound frame, so every rejection path
//! below is directly testable.

use anyhow::{bail, Context, Result};
use buzz_sdk::nip_oa;
use buzz_sdk::spawner::{AttestationFrame, PromptMaterial, ATTESTATION_NONCE_BYTES};
use nostr::{Keys, PublicKey};

use crate::store::AgentRecord;

/// Generate a fresh handshake nonce.
pub fn new_nonce() -> String {
    let bytes: [u8; ATTESTATION_NONCE_BYTES] = rand::random();
    hex::encode(bytes)
}

/// What the reconciler should do with an inbound attestation frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseOutcome {
    /// The owner attested. Store `auth_tag` and start the container.
    Accept {
        /// Verified NIP-OA auth tag, as a JSON array string.
        auth_tag: String,
        /// Prompt material delivered alongside the tag, if any.
        prompt: Option<PromptMaterial>,
        /// Secret key of a relocated agent, when the spec named one.
        private_key_nsec: Option<String>,
    },
    /// The owner explicitly declined.
    Rejected {
        /// Reason to surface in the status event.
        reason: String,
    },
}

/// Decide what an inbound attestation frame means for `record`.
///
/// `sender` is the *verified* author of the kind:24201 event, taken from the
/// signed envelope rather than from the frame body — a frame cannot name its own
/// author into being.
///
/// Every check here exists because skipping it is exploitable:
///
/// - **Sender is the record's owner.** Otherwise any pubkey could attest an
///   agent to *itself*, and the relay would then admit that agent under an
///   attacker's membership.
/// - **A round is actually in flight.** Without this, a frame replayed after the
///   handshake completes could swap a live agent's auth tag.
/// - **Nonce matches.** Binds the response to this specific request, so a frame
///   captured from an earlier round for the same agent cannot be replayed.
/// - **Agent pubkey matches.** Stops a response for agent A being applied to
///   agent B when an owner has several pending at once.
/// - **The tag actually verifies, and verifies to this owner.** `verify_auth_tag`
///   recovers the owner pubkey from the signature; a tag that verifies to
///   somebody else is a delegation the owner never made.
pub fn evaluate_response(
    record: &AgentRecord,
    sender: &PublicKey,
    frame: &AttestationFrame,
) -> Result<ResponseOutcome> {
    frame.validate().context("malformed attestation frame")?;

    if sender.to_hex() != record.owner_pubkey {
        bail!(
            "attestation from {} does not own agent {}",
            sender.to_hex(),
            record.agent_pubkey
        );
    }

    let Some(pending) = record.pending_nonce.as_deref() else {
        bail!(
            "no attestation round in flight for agent {}",
            record.agent_pubkey
        );
    };
    if frame.nonce() != pending {
        bail!("attestation nonce does not match the pending request");
    }
    if frame.agent_pubkey() != record.agent_pubkey {
        bail!("attestation agent pubkey does not match the pending request");
    }
    if frame.spec_slug() != record.slug {
        bail!("attestation spec slug does not match the pending request");
    }

    match frame {
        AttestationFrame::Request { .. } => {
            bail!("received an attestation request where a response was expected")
        }
        AttestationFrame::Reject { reason, .. } => Ok(ResponseOutcome::Rejected {
            reason: reason
                .clone()
                .unwrap_or_else(|| "owner declined the attestation".into()),
        }),
        AttestationFrame::PromptUpdate { .. } => {
            // Handled separately: it opens no round, so it has no nonce to
            // match and must not be evaluated as a handshake response.
            bail!("prompt updates are not attestation responses")
        }
        AttestationFrame::CredentialUpdate { .. } | AttestationFrame::CredentialAck { .. } => {
            // Owner-scoped frames, routed in the daemon before this point.
            bail!("credential frames are not attestation responses")
        }
        AttestationFrame::Response {
            auth_tag,
            prompt,
            private_key_nsec,
            ..
        } => {
            let agent_pubkey = PublicKey::parse(&record.agent_pubkey)
                .context("stored agent pubkey is not a valid public key")?;
            let recovered = nip_oa::verify_auth_tag(auth_tag, &agent_pubkey)
                .context("auth tag failed NIP-OA verification")?;
            if recovered != *sender {
                bail!(
                    "auth tag verifies to {} but was sent by {}",
                    recovered.to_hex(),
                    sender.to_hex()
                );
            }
            // A delivered key must actually be the identity we asked about,
            // or the spawner would run a different agent than the owner named
            // and than the auth tag attests.
            if let Some(nsec) = private_key_nsec {
                let delivered =
                    Keys::parse(nsec).context("delivered private key is not a valid secret key")?;
                if delivered.public_key() != agent_pubkey {
                    bail!(
                        "delivered key is for {}, but this handshake is about {}",
                        delivered.public_key().to_hex(),
                        record.agent_pubkey
                    );
                }
            }

            Ok(ResponseOutcome::Accept {
                auth_tag: auth_tag.clone(),
                prompt: prompt.clone(),
                private_key_nsec: private_key_nsec.clone(),
            })
        }
    }
}

/// Whether a pending attestation request has aged past `timeout_secs`.
pub fn is_attestation_expired(record: &AgentRecord, now: i64, timeout_secs: i64) -> bool {
    match record.attestation_sent_at {
        Some(sent_at) => now.saturating_sub(sent_at) > timeout_secs,
        // A record with a pending nonce but no timestamp predates this field;
        // treat it as expired so it is re-requested rather than stuck forever.
        None => record.pending_nonce.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    struct Fixture {
        owner: Keys,
        agent: Keys,
        record: AgentRecord,
        nonce: String,
    }

    fn fixture() -> Fixture {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let nonce = new_nonce();
        let record = AgentRecord {
            slug: "fizz-prod".into(),
            owner_pubkey: owner.public_key().to_hex(),
            agent_pubkey: agent.public_key().to_hex(),
            private_key_nsec: "nsec1placeholder".into(),
            auth_tag: None,
            pending_nonce: Some(nonce.clone()),
            attestation_sent_at: Some(1_000),
            spec_hash: None,
            prompt: None,
            restart_count: 0,
            last_failure_at: None,
            carried_team_instructions: None,
        };
        Fixture {
            owner,
            agent,
            record,
            nonce,
        }
    }

    fn response(f: &Fixture, signer: &Keys, nonce: &str) -> AttestationFrame {
        let auth_tag = nip_oa::compute_auth_tag(signer, &f.agent.public_key(), "").unwrap();
        AttestationFrame::Response {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: f.agent.public_key().to_hex(),
            nonce: nonce.to_string(),
            auth_tag,
            prompt: None,
            private_key_nsec: None,
        }
    }

    #[test]
    fn accepts_a_well_formed_response_from_the_owner() {
        let f = fixture();
        let frame = response(&f, &f.owner, &f.nonce);
        let outcome = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap();
        assert!(matches!(outcome, ResponseOutcome::Accept { .. }));
    }

    #[test]
    fn rejects_attestation_from_a_non_owner() {
        // The core attack: a stranger attesting somebody else's agent to
        // themselves, which would admit that agent under the stranger's
        // relay membership.
        let f = fixture();
        let attacker = Keys::generate();
        let frame = response(&f, &attacker, &f.nonce);
        let err = evaluate_response(&f.record, &attacker.public_key(), &frame).unwrap_err();
        assert!(err.to_string().contains("does not own agent"));
    }

    #[test]
    fn rejects_a_tag_that_verifies_to_someone_other_than_the_sender() {
        // Owner sends the frame, but the tag inside was signed by a third
        // party. Checking only the sender would let this through.
        let f = fixture();
        let third_party = Keys::generate();
        let frame = response(&f, &third_party, &f.nonce);
        let err = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap_err();
        assert!(err.to_string().contains("verifies to"));
    }

    #[test]
    fn accepts_a_relocated_agents_own_key() {
        // Moving an existing agent to a spawner: the owner delivers the key
        // whose pubkey the handshake is already about.
        let f = fixture();
        let nsec = {
            use nostr::nips::nip19::ToBech32;
            f.agent.secret_key().to_bech32().unwrap()
        };
        let auth_tag = nip_oa::compute_auth_tag(&f.owner, &f.agent.public_key(), "").unwrap();
        let frame = AttestationFrame::Response {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: f.agent.public_key().to_hex(),
            nonce: f.nonce.clone(),
            auth_tag,
            prompt: None,
            private_key_nsec: Some(nsec.clone()),
        };
        let outcome = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap();
        assert_eq!(
            outcome,
            ResponseOutcome::Accept {
                auth_tag: match &frame {
                    AttestationFrame::Response { auth_tag, .. } => auth_tag.clone(),
                    _ => unreachable!(),
                },
                prompt: None,
                private_key_nsec: Some(nsec),
            }
        );
    }

    #[test]
    fn rejects_a_key_for_a_different_agent() {
        // Without this the spawner would run an identity the owner never named
        // and the auth tag does not attest — a confused-deputy handover.
        let f = fixture();
        let other = Keys::generate();
        let nsec = {
            use nostr::nips::nip19::ToBech32;
            other.secret_key().to_bech32().unwrap()
        };
        let auth_tag = nip_oa::compute_auth_tag(&f.owner, &f.agent.public_key(), "").unwrap();
        let frame = AttestationFrame::Response {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: f.agent.public_key().to_hex(),
            nonce: f.nonce.clone(),
            auth_tag,
            prompt: None,
            private_key_nsec: Some(nsec),
        };
        let err = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap_err();
        assert!(err.to_string().contains("delivered key is for"));
    }

    #[test]
    fn rejects_a_malformed_delivered_key() {
        let f = fixture();
        let auth_tag = nip_oa::compute_auth_tag(&f.owner, &f.agent.public_key(), "").unwrap();
        let frame = AttestationFrame::Response {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: f.agent.public_key().to_hex(),
            nonce: f.nonce.clone(),
            auth_tag,
            prompt: None,
            private_key_nsec: Some("not-an-nsec".into()),
        };
        assert!(evaluate_response(&f.record, &f.owner.public_key(), &frame).is_err());
    }

    #[test]
    fn rejects_a_replayed_nonce() {
        let f = fixture();
        let frame = response(&f, &f.owner, &new_nonce());
        let err = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap_err();
        assert!(err.to_string().contains("nonce does not match"));
    }

    #[test]
    fn rejects_a_frame_when_no_round_is_in_flight() {
        // Guards a completed agent against a late replay swapping its auth tag.
        let mut f = fixture();
        let frame = response(&f, &f.owner, &f.nonce);
        f.record.pending_nonce = None;
        let err = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap_err();
        assert!(err.to_string().contains("no attestation round in flight"));
    }

    #[test]
    fn rejects_a_response_aimed_at_a_different_agent() {
        let f = fixture();
        let other_agent = Keys::generate();
        let auth_tag = nip_oa::compute_auth_tag(&f.owner, &other_agent.public_key(), "").unwrap();
        let frame = AttestationFrame::Response {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: other_agent.public_key().to_hex(),
            nonce: f.nonce.clone(),
            auth_tag,
            prompt: None,
            private_key_nsec: None,
        };
        let err = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap_err();
        assert!(err.to_string().contains("agent pubkey does not match"));
    }

    #[test]
    fn rejects_a_garbage_auth_tag() {
        let f = fixture();
        let frame = AttestationFrame::Response {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: f.agent.public_key().to_hex(),
            nonce: f.nonce.clone(),
            auth_tag: r#"["auth","not","a","tag"]"#.into(),
            prompt: None,
            private_key_nsec: None,
        };
        assert!(evaluate_response(&f.record, &f.owner.public_key(), &frame).is_err());
    }

    #[test]
    fn surfaces_an_explicit_rejection() {
        let f = fixture();
        let frame = AttestationFrame::Reject {
            spec_slug: f.record.slug.clone(),
            agent_pubkey: f.agent.public_key().to_hex(),
            nonce: f.nonce.clone(),
            reason: Some("unknown spawner".into()),
        };
        let outcome = evaluate_response(&f.record, &f.owner.public_key(), &frame).unwrap();
        assert_eq!(
            outcome,
            ResponseOutcome::Rejected {
                reason: "unknown spawner".into()
            }
        );
    }

    #[test]
    fn expiry_needs_both_a_pending_round_and_elapsed_time() {
        let mut f = fixture();
        assert!(!is_attestation_expired(&f.record, 1_100, 600));
        assert!(is_attestation_expired(&f.record, 2_000, 600));

        f.record.pending_nonce = None;
        f.record.attestation_sent_at = None;
        assert!(!is_attestation_expired(&f.record, 9_999, 600));
    }

    #[test]
    fn nonces_are_full_width_and_distinct() {
        let a = new_nonce();
        let b = new_nonce();
        assert_eq!(a.len(), ATTESTATION_NONCE_BYTES * 2);
        assert_ne!(a, b);
    }
}
