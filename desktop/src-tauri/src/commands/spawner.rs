//! Owner side of the NIP-AS spawner attestation handshake.
//!
//! A `buzz-spawner` daemon mints an agent's secret key on the server and never
//! transmits it. But a NIP-OA auth tag binds one specific agent pubkey and is
//! signed with the *owner's* secret key, so the spawner cannot self-attest — it
//! has to ask. This module is the answer to that question.
//!
//! # Security: the auth tag is a delegation, not a formality
//!
//! Signing an attestation admits that agent pubkey to the relay under **this
//! user's** membership. A malicious spawner that gets a signature can run an
//! agent that reads the user's channels. So:
//!
//! - Signing is never automatic for an unrecognized spawner. The frontend must
//!   have recorded an explicit trust decision first; [`respond_to_spawner_attestation`]
//!   fails closed when it has not.
//! - The tag is computed only for the agent pubkey named in the *decrypted*
//!   frame, so a frame body cannot ask for a signature over some other key
//!   while displaying a benign one.
//! - The response is NIP-44 encrypted back to the same spawner that asked, so
//!   an eavesdropper on the ephemeral kind:24201 stream gets ciphertext.

use nostr::JsonUtil;
use serde::{Deserialize, Serialize};

use buzz_sdk_pkg::spawner::{build_spawner_attestation, AttestationFrame, PromptMaterial};

use crate::app_state::AppState;
use crate::managed_agents::load_managed_agents;

/// The trusted-spawner decision the frontend has already made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpawnerTrust {
    /// The user has approved this spawner pubkey.
    Trusted,
    /// The user has not approved it, or explicitly declined.
    Untrusted,
}

/// A decrypted attestation request, for display before the user decides.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerAttestationRequest {
    /// Spawner pubkey that sent the request, hex.
    pub spawner_pubkey: String,
    /// Spec slug the agent belongs to.
    pub spec_slug: String,
    /// The freshly minted agent pubkey the spawner wants attested.
    pub agent_pubkey: String,
    /// NIP-OA conditions the spawner is asking to be signed over.
    pub conditions: String,
    /// Handshake nonce, echoed back in the response.
    pub nonce: String,
}

/// Decrypt an inbound kind:24201 frame so the UI can show the user what is
/// being asked before anything is signed.
///
/// Returns `Ok(None)` for a frame that is not a request — responses and
/// rejections are the owner's own outbound traffic echoed back, and are not
/// something to prompt about.
#[tauri::command]
pub async fn decode_spawner_attestation(
    state: tauri::State<'_, AppState>,
    spawner_pubkey: String,
    encrypted_content: String,
) -> Result<Option<SpawnerAttestationRequest>, String> {
    let keys = state.signing_keys()?;
    let spawner = nostr::PublicKey::from_hex(&spawner_pubkey)
        .map_err(|e| format!("invalid spawner pubkey: {e}"))?;

    let frame = decrypt_frame(&keys, &spawner, &encrypted_content)?;

    match frame {
        AttestationFrame::Request {
            spec_slug,
            agent_pubkey,
            conditions,
            nonce,
        } => Ok(Some(SpawnerAttestationRequest {
            spawner_pubkey: spawner.to_hex(),
            spec_slug,
            agent_pubkey,
            conditions,
            nonce,
        })),
        _ => Ok(None),
    }
}

/// Build the signed answer to an attestation request: sign the NIP-OA tag when
/// `trust` is [`SpawnerTrust::Trusted`], and an explicit rejection otherwise.
///
/// Rejecting explicitly rather than staying silent matters — it lets the
/// spawner report a clear `failed` status immediately instead of leaving the
/// agent stuck in `pending_attestation` until its timeout expires.
///
/// # Why this returns the event instead of publishing it
///
/// Kind 24201 is ephemeral. The relay routes ephemeral events through its
/// WebSocket handler; `POST /events` runs the ingest path, whose per-kind scope
/// allowlist has no arm for ephemeral kinds and rejects them with
/// `restricted: unknown event kind`. Kind 24200 observer control frames solve
/// this the same way — `build_observer_control_event` signs here and the
/// renderer publishes over the live socket.
/// Outcome of answering an attestation, for the caller to act on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerAttestationResponse {
    /// The signed kind:24201 event to publish over the WebSocket.
    pub event_json: String,
    /// Set when this handover relocated a locally-managed agent.
    ///
    /// The caller MUST stop the local process for this pubkey: two runners
    /// holding one key both see every mention and both reply, so the agent
    /// answers twice and burns two turns per message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relocated_agent_pubkey: Option<String>,
}

#[tauri::command]
pub async fn respond_to_spawner_attestation(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    spawner_pubkey: String,
    encrypted_content: String,
    trust: SpawnerTrust,
    reject_reason: Option<String>,
    prompt: Option<PromptMaterial>,
) -> Result<SpawnerAttestationResponse, String> {
    let keys = state.signing_keys()?;
    let spawner = nostr::PublicKey::from_hex(&spawner_pubkey)
        .map_err(|e| format!("invalid spawner pubkey: {e}"))?;

    // Re-decrypt rather than trusting fields passed back from the frontend. The
    // signature must cover the pubkey the spawner actually asked about, not one
    // the renderer could have substituted after the user saw the prompt.
    let frame = decrypt_frame(&keys, &spawner, &encrypted_content)?;
    let AttestationFrame::Request {
        spec_slug,
        agent_pubkey,
        conditions,
        nonce,
    } = frame
    else {
        return Err("attestation frame is not a request".into());
    };

    // Relocation: when the spawner is asking about an agent this device already
    // manages, the answer carries that agent's existing key so the SAME identity
    // moves — its channels, profile, and NIP-AE memory all hang off that pubkey,
    // and a fresh key would strand every one of them. Looked up from the local
    // store rather than taken from the frame, so a spawner cannot name an
    // arbitrary pubkey and be handed a key for it.
    let mut relocated_agent_pubkey = None;
    let mut relocation_nsec = None;
    if matches!(trust, SpawnerTrust::Trusted) {
        let managed = load_managed_agents(&app)?;
        if let Some(record) = managed
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&agent_pubkey))
        {
            if record.private_key_nsec.trim().is_empty() {
                return Err(format!(
                    "cannot relocate {}: its key is unavailable on this device",
                    record.name
                ));
            }
            relocated_agent_pubkey = Some(record.pubkey.clone());
            relocation_nsec = Some(record.private_key_nsec.clone());
        }
    }

    let response = match trust {
        SpawnerTrust::Untrusted => AttestationFrame::Reject {
            spec_slug,
            agent_pubkey,
            nonce,
            reason: Some(
                reject_reason.unwrap_or_else(|| "owner has not approved this spawner".into()),
            ),
        },
        SpawnerTrust::Trusted => {
            let agent = nostr::PublicKey::from_hex(&agent_pubkey)
                .map_err(|e| format!("invalid agent pubkey in attestation request: {e}"))?;
            let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(&keys, &agent, &conditions)
                .map_err(|e| format!("failed to compute owner auth tag: {e}"))?;
            AttestationFrame::Response {
                spec_slug,
                agent_pubkey,
                nonce,
                auth_tag,
                private_key_nsec: relocation_nsec,
                // Prompt material rides the encrypted channel rather than the
                // world-readable spec, so an agent's instructions never become
                // public. Absent for a shared persona the spawner can read.
                prompt: prompt.filter(|p| !p.is_empty()),
            }
        }
    };

    let plaintext = serde_json::to_string(&response)
        .map_err(|e| format!("failed to serialize attestation response: {e}"))?;
    let ciphertext = nostr::nips::nip44::encrypt(
        keys.secret_key(),
        &spawner,
        plaintext,
        nostr::nips::nip44::Version::V2,
    )
    .map_err(|e| format!("failed to encrypt attestation response: {e}"))?;

    let event = build_spawner_attestation(&spawner.to_hex(), &ciphertext)
        .map_err(|e| format!("failed to build attestation event: {e}"))?
        .sign_with_keys(&keys)
        .map_err(|e| format!("failed to sign attestation event: {e}"))?;
    Ok(SpawnerAttestationResponse {
        event_json: event.as_json(),
        relocated_agent_pubkey,
    })
}

/// Output of [`send_spawner_prompt_update`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerPromptUpdateOut {
    /// The signed kind:24201 event to publish over the WebSocket.
    pub event_json: String,
    /// Content hash of the prompt material this update carries, for the
    /// caller to confirm against what the spawner later reports applying.
    pub prompt_hash: String,
}

/// Build a signed, NIP-44-encrypted `PromptUpdate` frame addressed to
/// `spawner_pubkey`, so a running agent can be edited without minting a new
/// identity or re-running the attestation handshake.
fn build_prompt_update_event(
    owner: &nostr::Keys,
    spawner_pubkey: &str,
    spec_slug: &str,
    agent_pubkey: &str,
    prompt: PromptMaterial,
) -> Result<SpawnerPromptUpdateOut, String> {
    let spawner = nostr::PublicKey::from_hex(spawner_pubkey)
        .map_err(|e| format!("invalid spawner pubkey: {e}"))?;

    let frame = AttestationFrame::PromptUpdate {
        spec_slug: spec_slug.to_string(),
        agent_pubkey: agent_pubkey.to_string(),
        prompt: prompt.clone(),
    };
    frame
        .validate()
        .map_err(|e| format!("malformed prompt update frame: {e}"))?;

    let plaintext = serde_json::to_string(&frame)
        .map_err(|e| format!("failed to serialize prompt update frame: {e}"))?;
    let ciphertext = nostr::nips::nip44::encrypt(
        owner.secret_key(),
        &spawner,
        plaintext,
        nostr::nips::nip44::Version::V2,
    )
    .map_err(|e| format!("failed to encrypt prompt update frame: {e}"))?;

    let event = build_spawner_attestation(&spawner.to_hex(), &ciphertext)
        .map_err(|e| format!("failed to build prompt update event: {e}"))?
        .sign_with_keys(owner)
        .map_err(|e| format!("failed to sign prompt update event: {e}"))?;

    Ok(SpawnerPromptUpdateOut {
        event_json: event.as_json(),
        prompt_hash: prompt.hash(),
    })
}

/// Build a signed prompt-update frame for a spawner-hosted agent, so its
/// system prompt / model / tool config can be edited in place.
///
/// Returns the signed event rather than publishing it, for the same reason as
/// [`respond_to_spawner_attestation`]: kind 24201 is ephemeral and must go out
/// over the live WebSocket, not `POST /events`.
#[tauri::command]
pub async fn send_spawner_prompt_update(
    state: tauri::State<'_, AppState>,
    spawner_pubkey: String,
    spec_slug: String,
    agent_pubkey: String,
    prompt: PromptMaterial,
) -> Result<SpawnerPromptUpdateOut, String> {
    let keys = state.signing_keys()?;
    build_prompt_update_event(&keys, &spawner_pubkey, &spec_slug, &agent_pubkey, prompt)
}

fn decrypt_frame(
    keys: &nostr::Keys,
    spawner: &nostr::PublicKey,
    encrypted_content: &str,
) -> Result<AttestationFrame, String> {
    let plaintext = nostr::nips::nip44::decrypt(keys.secret_key(), spawner, encrypted_content)
        .map_err(|e| format!("failed to decrypt attestation frame: {e}"))?;
    let frame: AttestationFrame = serde_json::from_str(&plaintext)
        .map_err(|e| format!("failed to parse attestation frame: {e}"))?;
    frame
        .validate()
        .map_err(|e| format!("malformed attestation frame: {e}"))?;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk_pkg::spawner::ATTESTATION_NONCE_BYTES;
    use nostr::Keys;

    fn encrypted_request(spawner: &Keys, owner: &Keys, agent_pubkey: &str) -> String {
        let frame = AttestationFrame::Request {
            spec_slug: "fizz-prod".into(),
            agent_pubkey: agent_pubkey.into(),
            conditions: String::new(),
            nonce: "ab".repeat(ATTESTATION_NONCE_BYTES),
        };
        nostr::nips::nip44::encrypt(
            spawner.secret_key(),
            &owner.public_key(),
            serde_json::to_string(&frame).unwrap(),
            nostr::nips::nip44::Version::V2,
        )
        .unwrap()
    }

    #[test]
    fn decodes_a_request_addressed_to_this_owner() {
        let owner = Keys::generate();
        let spawner = Keys::generate();
        let agent = Keys::generate();
        let ciphertext = encrypted_request(&spawner, &owner, &agent.public_key().to_hex());

        let frame = decrypt_frame(&owner, &spawner.public_key(), &ciphertext).unwrap();
        assert_eq!(frame.agent_pubkey(), agent.public_key().to_hex());
        assert_eq!(frame.spec_slug(), "fizz-prod");
    }

    #[test]
    fn cannot_decrypt_a_frame_meant_for_someone_else() {
        let owner = Keys::generate();
        let stranger = Keys::generate();
        let spawner = Keys::generate();
        let agent = Keys::generate();
        let ciphertext = encrypted_request(&spawner, &stranger, &agent.public_key().to_hex());

        assert!(decrypt_frame(&owner, &spawner.public_key(), &ciphertext).is_err());
    }

    #[test]
    fn the_signed_tag_verifies_for_the_requested_agent_only() {
        // The tag must cover exactly the pubkey in the decrypted frame. A tag
        // that verified for any other key would let a spawner get a signature
        // for an agent the user never saw.
        let owner = Keys::generate();
        let agent = Keys::generate();
        let other = Keys::generate();

        let auth_tag =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();

        assert_eq!(
            buzz_sdk_pkg::nip_oa::verify_auth_tag(&auth_tag, &agent.public_key()).unwrap(),
            owner.public_key()
        );
        assert!(buzz_sdk_pkg::nip_oa::verify_auth_tag(&auth_tag, &other.public_key()).is_err());
    }

    #[test]
    fn prompt_update_round_trips_to_the_spawner() {
        let owner = Keys::generate();
        let spawner = Keys::generate();
        let prompt = PromptMaterial {
            model: Some("claude-opus-5".into()),
            ..Default::default()
        };
        let out = build_prompt_update_event(
            &owner,
            &spawner.public_key().to_hex(),
            "honey",
            &Keys::generate().public_key().to_hex(),
            prompt.clone(),
        )
        .unwrap();
        assert_eq!(out.prompt_hash, prompt.hash());
        // Spawner can decrypt and reads back the same frame.
        let event: nostr::Event = serde_json::from_str(&out.event_json).unwrap();
        let plain = nostr::nips::nip44::decrypt(
            spawner.secret_key(),
            &owner.public_key(),
            event.content.as_str(),
        )
        .unwrap();
        let frame: AttestationFrame = serde_json::from_str(&plain).unwrap();
        match frame {
            AttestationFrame::PromptUpdate {
                spec_slug,
                prompt: p,
                ..
            } => {
                assert_eq!(spec_slug, "honey");
                assert_eq!(p, prompt);
            }
            other => panic!("wrong frame: {other:?}"),
        }
    }

    #[test]
    fn a_malformed_frame_is_refused_before_anything_is_signed() {
        let owner = Keys::generate();
        let spawner = Keys::generate();
        // Valid JSON, valid encryption, but a nonce of the wrong width.
        let frame = serde_json::json!({
            "type": "request",
            "spec_slug": "fizz-prod",
            "agent_pubkey": Keys::generate().public_key().to_hex(),
            "conditions": "",
            "nonce": "short",
        });
        let ciphertext = nostr::nips::nip44::encrypt(
            spawner.secret_key(),
            &owner.public_key(),
            frame.to_string(),
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();

        assert!(decrypt_frame(&owner, &spawner.public_key(), &ciphertext).is_err());
    }
}
