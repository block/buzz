//! Owner NIP44 operations on the captured native signer; no owner secret export.
use super::*;
use crate::active_user_signer::ActiveUserSigner;
use nostr::NostrSigner;

/// Encrypt with the exact captured owner and a validated, distinct agent peer.
pub(crate) async fn encrypt_snapshot_envelope_with_signer(
    snapshot: &AgentSnapshot,
    owner: &ActiveUserSigner,
    agent: &PublicKey,
) -> Result<LockedSnapshotEnvelope, String> {
    let mut envelope = LockedSnapshotEnvelope {
        format: LOCKED_FORMAT.into(),
        version: LOCKED_VERSION,
        encryption: LockedEncryption {
            scheme: LOCKED_SCHEME.into(),
            owner_pubkey: owner.public_key().to_hex(),
            agent_pubkey: agent.to_hex(),
            // Validate endpoints before crypto, without weakening the empty-body guard.
            ciphertext: "pending".into(),
        },
    };
    validate_envelope(&envelope)?;
    let json = encode_snapshot_json(snapshot)?;
    if json.len() > NIP44_PLAINTEXT_MAX {
        return Err(format!(
            "Agent manifest is too large to lock ({} bytes; the encrypted \
             format caps at {NIP44_PLAINTEXT_MAX}). Reduce the avatar size \
             or mint an unlocked card.",
            json.len()
        ));
    }
    let plaintext =
        std::str::from_utf8(&json).map_err(|e| format!("Manifest JSON was not UTF-8: {e}"))?;
    envelope.encryption.ciphertext = owner
        .nip44_encrypt(agent, plaintext)
        .await
        .map_err(|e| format!("Failed to encrypt card manifest: {e}"))?;
    owner.check_valid()?;
    validate_envelope(&envelope)?;
    Ok(envelope)
}

/// Unlock only the owner endpoint. Agent-endpoint fallback remains record-bound
/// in the importer; an active identity must never be relabeled as an agent.
pub(crate) async fn decrypt_envelope_with_signer(
    envelope: &LockedSnapshotEnvelope,
    owner: &ActiveUserSigner,
) -> Result<AgentSnapshot, String> {
    let (owner_pubkey, agent) = validate_envelope(envelope)?;
    if owner.public_key() != owner_pubkey {
        return Err(LOCKED_CARD_REFUSAL.into());
    }
    let result = owner
        .nip44_decrypt(&agent, &envelope.encryption.ciphertext)
        .await;
    owner.check_valid()?;
    // Keep the local locked-card refusal; remote session/transport/response
    // failures must propagate, never masquerade as a malformed card or fallback.
    let plaintext = result.map_err(|e| {
        if owner.generation().is_some() {
            format!("Failed to decrypt card manifest: {e}")
        } else {
            LOCKED_CARD_REFUSAL.into()
        }
    })?;
    if plaintext.len() > NIP44_PLAINTEXT_MAX {
        return Err(LOCKED_CARD_REFUSAL.into());
    }
    decode_snapshot_json(plaintext.as_bytes())
}

/// Encode a validated manifest with the captured owner capability.
pub(crate) async fn encode_locked_snapshot_png_with_signer(
    snapshot: &AgentSnapshot,
    owner: &ActiveUserSigner,
    agent: &PublicKey,
    avatar_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if snapshot.memory.level == MemoryLevel::None && !snapshot.memory.entries.is_empty() {
        return Err(
            "Cannot write a snapshot with memory.level 'none' and non-empty memory entries.".into(),
        );
    }
    let envelope = encrypt_snapshot_envelope_with_signer(snapshot, owner, agent).await?;
    let json = serde_json::to_vec(&envelope)
        .map_err(|e| format!("Failed to serialize locked card envelope: {e}"))?;
    if json.len() > MAX_LOCKED_ENVELOPE_JSON_BYTES {
        return Err("Locked card envelope exceeds the maximum size.".into());
    }
    let png = encode_chunk_payload_png(&json, avatar_bytes)?;
    owner.check_valid()?;
    Ok(png)
}
