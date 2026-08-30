//! Locked (encrypted) agent-card envelope — NIP-44 v2 over the snapshot manifest.
//!
//! A locked card carries the same `buzz_agent_snapshot` tEXt chunk as a plain
//! card, but the chunk JSON is a typed outer envelope whose ciphertext
//! decrypts to the ordinary manifest. The NIP-44 v2 conversation key is
//! symmetric over the (owner, agent) pair, so BOTH the owner's and the
//! agent's nsec decrypt the card — nobody else's does (NIP-AE's scheme).
//!
//! Wire contract (agreed with Wren, buzz-agent-trading-cards thread):
//! - Plain cards keep today's exact bytes; detection dispatches once on the
//!   exact `format` discriminator and rejects unknown versions/schemes
//!   rather than falling through to manifest parsing.
//! - Key lookup is exact-endpoint only: the owner identity key when its
//!   pubkey equals `ownerPubkey`, or a hydrated local managed-agent record
//!   whose record pubkey AND derived-secret pubkey equal `agentPubkey`.
//!   No trial decryption; anything else fails closed as locked.
//! - Caps beyond the outer 10 MiB PNG gate: 65,535-byte NIP-44 plaintext
//!   limit on the serialized manifest BEFORE encryption; envelope JSON and
//!   ciphertext are capped before serde/base64/decrypt work; decrypted bytes
//!   are capped before snapshot parsing.
//! - Decrypt/auth failures return only the locked-card refusal — never
//!   partial plaintext or crypto details.

use buzz_core_pkg::engram::NIP44_PLAINTEXT_MAX;
use nostr::nips::nip44::{self, Version};
use nostr::{Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};

use super::agent_snapshot::{
    decode_snapshot_json, encode_chunk_payload_png, encode_snapshot_json, AgentSnapshot,
    MemoryLevel, FORMAT_DISCRIMINATOR,
};
use super::types::ManagedAgentRecord;

/// Discriminator for the locked envelope. Distinct from the plain manifest's
/// `buzz-agent-snapshot` so detection never guesses.
pub const LOCKED_FORMAT: &str = "buzz-agent-snapshot-encrypted";
/// Envelope schema version this module produces and accepts.
pub const LOCKED_VERSION: u32 = 1;
/// Encryption scheme identifier this module produces and accepts.
pub const LOCKED_SCHEME: &str = "nip44-v2";

/// A max-size NIP-44 v2 payload (1 version + 32 nonce + 2 len + 65,536
/// padded + 32 MAC = 65,603 bytes) base64-encodes to 87,472 chars.
/// Anything larger is rejected before base64/decrypt work.
pub const MAX_LOCKED_CIPHERTEXT_BYTES: usize = 90_000;
/// Envelope JSON = ciphertext + two pubkeys + fixed keys. Rejected before
/// typed deserialization.
pub const MAX_LOCKED_ENVELOPE_JSON_BYTES: usize = MAX_LOCKED_CIPHERTEXT_BYTES + 1024;

/// The only error a failed unlock may surface. Deliberately says nothing
/// about which key was tried or why decryption failed.
pub const LOCKED_CARD_REFUSAL: &str =
    "This card is locked to its owner and agent. Only they can import it.";

// ── Envelope types ────────────────────────────────────────────────────────────

/// Typed outer envelope stored (base64 JSON) in the `buzz_agent_snapshot`
/// chunk of a locked card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LockedSnapshotEnvelope {
    /// Always [`LOCKED_FORMAT`].
    pub format: String,
    /// Always [`LOCKED_VERSION`].
    pub version: u32,
    pub encryption: LockedEncryption,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LockedEncryption {
    /// Always [`LOCKED_SCHEME`].
    pub scheme: String,
    /// Owner identity pubkey (64 lowercase hex). Plaintext so a decryptor
    /// knows which counterparty to pair with.
    pub owner_pubkey: String,
    /// Agent instance pubkey (64 lowercase hex).
    pub agent_pubkey: String,
    /// NIP-44 v2 ciphertext (base64) of the plain manifest JSON.
    pub ciphertext: String,
}

/// Result of parsing a chunk payload: either today's plain manifest or a
/// validated locked envelope. The plain manifest is boxed because it may
/// inline a multi-KB avatar data URL, dwarfing the envelope variant.
#[derive(Debug)]
pub enum ChunkPayload {
    Plain(Box<AgentSnapshot>),
    Locked(LockedSnapshotEnvelope),
}

/// Minimal probe used to read the `format` discriminator without building a
/// full JSON tree for large plain manifests.
#[derive(Deserialize)]
struct FormatProbe {
    #[serde(default)]
    format: Option<String>,
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Canonical pubkey check: exactly 64 lowercase hex chars that parse as a
/// valid x-only pubkey. Lowercase is required so string comparisons against
/// record pubkeys (always `to_hex()` output) stay sound. Curve validation is
/// explicit: nostr's `PublicKey::from_hex` only decodes 32 bytes and defers
/// lift-x validation to `xonly()`, so a non-point like `"f" * 64` would
/// otherwise pass structurally and fail only at decrypt time.
pub(crate) fn parse_canonical_pubkey(field: &str, value: &str) -> Result<PublicKey, String> {
    if value.len() != 64
        || !value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(format!(
            "Locked card envelope has a malformed {field} (expected 64 lowercase hex chars)."
        ));
    }
    let pubkey = PublicKey::from_hex(value)
        .map_err(|_| format!("Locked card envelope has an invalid {field}."))?;
    pubkey
        .xonly()
        .map_err(|_| format!("Locked card envelope has an invalid {field} (not a curve point)."))?;
    Ok(pubkey)
}

/// Structural validation of a locked envelope: exact version + scheme,
/// canonical pubkeys, distinct endpoints, bounded ciphertext. Does no
/// key lookup or crypto.
pub fn validate_envelope(
    envelope: &LockedSnapshotEnvelope,
) -> Result<(PublicKey, PublicKey), String> {
    if envelope.format != LOCKED_FORMAT {
        return Err(format!(
            "Unsupported locked card format: {:?} (expected {LOCKED_FORMAT:?})",
            envelope.format
        ));
    }
    if envelope.version != LOCKED_VERSION {
        return Err(format!(
            "Unsupported locked card envelope version: {} (expected {LOCKED_VERSION})",
            envelope.version
        ));
    }
    if envelope.encryption.scheme != LOCKED_SCHEME {
        return Err(format!(
            "Unsupported locked card encryption scheme: {:?} (expected {LOCKED_SCHEME:?})",
            envelope.encryption.scheme
        ));
    }
    let owner = parse_canonical_pubkey("ownerPubkey", &envelope.encryption.owner_pubkey)?;
    let agent = parse_canonical_pubkey("agentPubkey", &envelope.encryption.agent_pubkey)?;
    if owner == agent {
        return Err("Locked card envelope owner and agent pubkeys must differ.".to_string());
    }
    if envelope.encryption.ciphertext.len() > MAX_LOCKED_CIPHERTEXT_BYTES {
        return Err("Locked card ciphertext exceeds the maximum size.".to_string());
    }
    if envelope.encryption.ciphertext.is_empty() {
        return Err("Locked card ciphertext is empty.".to_string());
    }
    Ok((owner, agent))
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Parse a raw chunk payload (JSON bytes from `extract_chunk_payload_png` or
/// an `.agent.json` file) and dispatch on the exact `format` discriminator.
///
/// - `buzz-agent-snapshot` → full plain-manifest decode + validation.
/// - `buzz-agent-snapshot-encrypted` → size caps, typed envelope parse,
///   structural validation. No decryption happens here.
/// - anything else (including missing `format`) → error, never a fall-through.
pub fn parse_chunk_payload(json_bytes: &[u8]) -> Result<ChunkPayload, String> {
    let probe: FormatProbe =
        serde_json::from_slice(json_bytes).map_err(|e| format!("Invalid snapshot JSON: {e}"))?;
    match probe.format.as_deref() {
        Some(f) if f == FORMAT_DISCRIMINATOR => Ok(ChunkPayload::Plain(Box::new(
            decode_snapshot_json(json_bytes)?,
        ))),
        Some(f) if f == LOCKED_FORMAT => {
            // Cap the envelope JSON before typed deserialization; a locked
            // envelope is small by construction (unlike plain manifests,
            // which may inline a multi-MB avatar).
            if json_bytes.len() > MAX_LOCKED_ENVELOPE_JSON_BYTES {
                return Err("Locked card envelope exceeds the maximum size.".to_string());
            }
            let envelope: LockedSnapshotEnvelope = serde_json::from_slice(json_bytes)
                .map_err(|e| format!("Invalid locked card envelope: {e}"))?;
            validate_envelope(&envelope)?;
            Ok(ChunkPayload::Locked(envelope))
        }
        Some(other) => Err(format!("Unsupported snapshot format: {other:?}")),
        None => Err("Snapshot payload has no format discriminator.".to_string()),
    }
}

// ── Encrypt ───────────────────────────────────────────────────────────────────

/// Encrypt a snapshot manifest into a locked envelope under the NIP-44 v2
/// conversation key for (owner secret, agent pubkey).
///
/// Fails clearly (never silently truncates) when the serialized manifest
/// exceeds the NIP-44 plaintext limit.
pub fn encrypt_snapshot_envelope(
    snapshot: &AgentSnapshot,
    owner_keys: &Keys,
    agent_pubkey: &PublicKey,
) -> Result<LockedSnapshotEnvelope, String> {
    let json_bytes = encode_snapshot_json(snapshot)?;
    if json_bytes.len() > NIP44_PLAINTEXT_MAX {
        return Err(format!(
            "Agent manifest is too large to lock ({} bytes; the encrypted \
             format caps at {NIP44_PLAINTEXT_MAX}). Reduce the avatar size \
             or mint an unlocked card.",
            json_bytes.len()
        ));
    }
    let plaintext = std::str::from_utf8(&json_bytes)
        .map_err(|e| format!("Manifest JSON was not UTF-8: {e}"))?;
    let ciphertext = nip44::encrypt(
        owner_keys.secret_key(),
        agent_pubkey,
        plaintext,
        Version::V2,
    )
    .map_err(|e| format!("Failed to encrypt card manifest: {e}"))?;

    Ok(LockedSnapshotEnvelope {
        format: LOCKED_FORMAT.to_string(),
        version: LOCKED_VERSION,
        encryption: LockedEncryption {
            scheme: LOCKED_SCHEME.to_string(),
            owner_pubkey: owner_keys.public_key().to_hex(),
            agent_pubkey: agent_pubkey.to_hex(),
            ciphertext,
        },
    })
}

/// Encode a snapshot into a LOCKED `.agent.png`: encrypt the manifest into
/// the envelope, then compose the PNG through the same chunk encoder plain
/// cards use. Mirrors `encode_snapshot_png`'s structural memory guard.
pub fn encode_locked_snapshot_png(
    snapshot: &AgentSnapshot,
    owner_keys: &Keys,
    agent_pubkey: &PublicKey,
    avatar_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if snapshot.memory.level == MemoryLevel::None && !snapshot.memory.entries.is_empty() {
        return Err(
            "Cannot write a snapshot with memory.level 'none' and non-empty memory entries."
                .to_string(),
        );
    }
    let envelope = encrypt_snapshot_envelope(snapshot, owner_keys, agent_pubkey)?;
    let envelope_json = serde_json::to_vec(&envelope)
        .map_err(|e| format!("Failed to serialize locked card envelope: {e}"))?;
    encode_chunk_payload_png(&envelope_json, avatar_bytes)
}

// ── Decrypt ───────────────────────────────────────────────────────────────────

/// Exact-endpoint key resolution (no trial decryption):
/// - the owner identity secret, only when its pubkey equals `ownerPubkey`;
/// - a hydrated local managed-agent record whose record pubkey AND
///   derived-secret pubkey both equal `agentPubkey`.
///
/// Returns `None` when neither exact endpoint exists — callers fail closed
/// with [`LOCKED_CARD_REFUSAL`].
pub fn resolve_unlock_secret(
    envelope: &LockedSnapshotEnvelope,
    owner_keys: Option<&Keys>,
    records: &[ManagedAgentRecord],
) -> Option<SecretKey> {
    if let Some(keys) = owner_keys {
        if keys.public_key().to_hex() == envelope.encryption.owner_pubkey {
            return Some(keys.secret_key().clone());
        }
    }
    let record = records
        .iter()
        .find(|r| r.pubkey == envelope.encryption.agent_pubkey)?;
    let agent_keys = Keys::parse(record.private_key_nsec.trim()).ok()?;
    if agent_keys.public_key().to_hex() != envelope.encryption.agent_pubkey {
        return None;
    }
    Some(agent_keys.secret_key().clone())
}

/// Decrypt a validated envelope with `my_secret`, which must be one of the
/// envelope's two exact endpoints (its derived pubkey selects the
/// counterparty). Returns the decoded, validated snapshot manifest.
///
/// Every auth/crypto failure maps to [`LOCKED_CARD_REFUSAL`] — nothing about
/// the failure mode leaks. Manifest decode errors after a successful decrypt
/// are surfaced normally (the caller proved key possession).
pub fn decrypt_envelope(
    envelope: &LockedSnapshotEnvelope,
    my_secret: &SecretKey,
) -> Result<AgentSnapshot, String> {
    let (owner_pub, agent_pub) = validate_envelope(envelope)?;
    let my_pub = Keys::new(my_secret.clone()).public_key();
    let counterparty = if my_pub == owner_pub {
        agent_pub
    } else if my_pub == agent_pub {
        owner_pub
    } else {
        return Err(LOCKED_CARD_REFUSAL.to_string());
    };

    let plaintext = nip44::decrypt(my_secret, &counterparty, &envelope.encryption.ciphertext)
        .map_err(|_| LOCKED_CARD_REFUSAL.to_string())?;
    if plaintext.len() > NIP44_PLAINTEXT_MAX {
        return Err(LOCKED_CARD_REFUSAL.to_string());
    }
    decode_snapshot_json(plaintext.as_bytes())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::agent_snapshot::{
        extract_chunk_payload_png, AgentSnapshotDefinition, AgentSnapshotMemory,
        AgentSnapshotProfile, FORMAT_VERSION,
    };

    fn sample_snapshot() -> AgentSnapshot {
        AgentSnapshot {
            format: FORMAT_DISCRIMINATOR.to_string(),
            version: FORMAT_VERSION,
            definition: AgentSnapshotDefinition {
                name: "Locked Test".to_string(),
                system_prompt: Some("You are a locked test agent.".to_string()),
                runtime: None,
                model: None,
                provider: None,
                parallelism: Some(1),
                respond_to: None,
                respond_to_allowlist: Vec::new(),
                name_pool: Vec::new(),
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
                source_is_builtin: false,
            },
            profile: AgentSnapshotProfile {
                display_name: "Locked Test".to_string(),
                about: None,
                avatar_data_url: None,
                avatar_url: None,
            },
            memory: AgentSnapshotMemory {
                level: MemoryLevel::None,
                entries: Vec::new(),
            },
        }
    }

    fn owner_agent_keys() -> (Keys, Keys) {
        (Keys::generate(), Keys::generate())
    }

    /// Minimal hydrated record for endpoint-resolution tests. Only the
    /// pubkey/nsec pair matters here.
    fn record_with_keys(pubkey: String, private_key_nsec: String) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey,
            name: "Locked Test".to_string(),
            persona_id: None,
            private_key_nsec,
            auth_tag: None,
            relay_url: "ws://localhost:3000".to_string(),
            avatar_url: None,
            acp_command: "buzz-acp".to_string(),
            agent_command: "goose".to_string(),
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 300,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: crate::managed_agents::types::BackendKind::Local,
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: crate::managed_agents::types::RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: Vec::new(),
            definition_parallelism: None,
            relay_mesh: None,
            effort_level: None,
            agent_command_override: None,
            persona_source_version: None,
            provider: None,
            team_catalog_source: None,
        }
    }

    fn locked_envelope() -> (LockedSnapshotEnvelope, Keys, Keys) {
        let (owner, agent) = owner_agent_keys();
        let env =
            encrypt_snapshot_envelope(&sample_snapshot(), &owner, &agent.public_key()).unwrap();
        (env, owner, agent)
    }

    #[test]
    fn owner_secret_decrypts() {
        let (env, owner, _agent) = locked_envelope();
        let decoded = decrypt_envelope(&env, owner.secret_key()).unwrap();
        assert_eq!(decoded, sample_snapshot());
    }

    #[test]
    fn agent_secret_decrypts() {
        let (env, _owner, agent) = locked_envelope();
        let decoded = decrypt_envelope(&env, agent.secret_key()).unwrap();
        assert_eq!(decoded, sample_snapshot());
    }

    #[test]
    fn unrelated_key_fails_closed_with_refusal_only() {
        let (env, _owner, _agent) = locked_envelope();
        let stranger = Keys::generate();
        let err = decrypt_envelope(&env, stranger.secret_key()).unwrap_err();
        assert_eq!(err, LOCKED_CARD_REFUSAL);
    }

    #[test]
    fn tampered_ciphertext_fails_with_refusal_only() {
        let (mut env, owner, _agent) = locked_envelope();
        // Flip a character mid-ciphertext (keep valid base64 alphabet).
        let mid = env.encryption.ciphertext.len() / 2;
        let mut bytes = env.encryption.ciphertext.into_bytes();
        bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
        env.encryption.ciphertext = String::from_utf8(bytes).unwrap();
        let err = decrypt_envelope(&env, owner.secret_key()).unwrap_err();
        assert_eq!(err, LOCKED_CARD_REFUSAL);
    }

    #[test]
    fn swapped_pubkeys_fail_closed_at_endpoint_resolution() {
        let (mut env, owner, agent) = locked_envelope();
        std::mem::swap(
            &mut env.encryption.owner_pubkey,
            &mut env.encryption.agent_pubkey,
        );
        // The NIP-44 conversation key is symmetric over the pair, so a swap
        // cannot grant a stranger anything — but it desyncs the routing
        // hints, and exact-endpoint resolution fails closed rather than
        // guessing: the owner identity no longer matches `ownerPubkey`, and
        // no local record holds the pubkey now in `agentPubkey`.
        assert!(resolve_unlock_secret(&env, Some(&owner), &[]).is_none());
        let nsec = nostr::ToBech32::to_bech32(agent.secret_key()).unwrap();
        let record = record_with_keys(agent.public_key().to_hex(), nsec);
        assert!(resolve_unlock_secret(&env, None, std::slice::from_ref(&record)).is_none());
    }

    #[test]
    fn mislabeled_pubkey_fails_decryption_with_refusal_only() {
        // Replacing `agentPubkey` with a third party's key makes the owner
        // derive the wrong conversation key — the NIP-44 MAC fails and only
        // the refusal surfaces.
        let (mut env, owner, _agent) = locked_envelope();
        env.encryption.agent_pubkey = Keys::generate().public_key().to_hex();
        let err = decrypt_envelope(&env, owner.secret_key()).unwrap_err();
        assert_eq!(err, LOCKED_CARD_REFUSAL);
    }

    #[test]
    fn malformed_pubkeys_rejected_structurally() {
        let (env, _owner, _agent) = locked_envelope();

        let mut short = env.clone();
        short.encryption.owner_pubkey = "abc123".to_string();
        assert!(validate_envelope(&short).unwrap_err().contains("malformed"));

        let mut upper = env.clone();
        upper.encryption.agent_pubkey = upper.encryption.agent_pubkey.to_uppercase();
        assert!(validate_envelope(&upper).unwrap_err().contains("malformed"));

        // A 64-hex string that is not a curve point (lift-x fails for
        // x = p-1... all-f) must be rejected STRUCTURALLY — before any key
        // lookup or decrypt work — per the wire contract.
        let mut not_a_point = env.clone();
        not_a_point.encryption.agent_pubkey = "f".repeat(64);
        assert!(validate_envelope(&not_a_point)
            .unwrap_err()
            .contains("not a curve point"));

        let mut same = env;
        same.encryption.agent_pubkey = same.encryption.owner_pubkey.clone();
        assert!(validate_envelope(&same).unwrap_err().contains("differ"));
    }

    #[test]
    fn unknown_format_version_scheme_rejected() {
        let (env, ..) = locked_envelope();

        let mut bad_version = env.clone();
        bad_version.version = 2;
        assert!(validate_envelope(&bad_version)
            .unwrap_err()
            .contains("version"));

        let mut bad_scheme = env.clone();
        bad_scheme.encryption.scheme = "nip44-v3".to_string();
        assert!(validate_envelope(&bad_scheme)
            .unwrap_err()
            .contains("scheme"));

        // Unknown top-level format never falls through to manifest parsing.
        let unknown = serde_json::json!({"format": "buzz-agent-snapshot-v9", "version": 1});
        let err = parse_chunk_payload(unknown.to_string().as_bytes()).unwrap_err();
        assert!(err.contains("Unsupported snapshot format"), "{err}");

        let missing = serde_json::json!({"version": 1});
        let err = parse_chunk_payload(missing.to_string().as_bytes()).unwrap_err();
        assert!(err.contains("no format discriminator"), "{err}");
    }

    #[test]
    fn plaintext_cap_enforced_before_encryption() {
        let (owner, agent) = owner_agent_keys();
        let mut snapshot = sample_snapshot();
        // Inflate the manifest beyond the NIP-44 plaintext limit.
        snapshot.definition.system_prompt = Some("x".repeat(NIP44_PLAINTEXT_MAX));
        let err = encrypt_snapshot_envelope(&snapshot, &owner, &agent.public_key()).unwrap_err();
        assert!(err.contains("too large to lock"), "{err}");
    }

    #[test]
    fn ciphertext_and_envelope_caps_enforced_before_crypto() {
        let (mut env, ..) = locked_envelope();
        env.encryption.ciphertext = "A".repeat(MAX_LOCKED_CIPHERTEXT_BYTES + 1);
        assert!(validate_envelope(&env)
            .unwrap_err()
            .contains("maximum size"));

        // Oversized envelope JSON is rejected before typed deserialization.
        let huge = format!(
            r#"{{"format":"{LOCKED_FORMAT}","version":1,"pad":"{}","encryption":{{}}}}"#,
            "p".repeat(MAX_LOCKED_ENVELOPE_JSON_BYTES)
        );
        let err = parse_chunk_payload(huge.as_bytes()).unwrap_err();
        assert!(err.contains("maximum size"), "{err}");
    }

    #[test]
    fn locked_png_round_trips_through_chunk_and_decrypt() {
        let (owner, agent) = owner_agent_keys();
        let snapshot = sample_snapshot();
        let png = encode_locked_snapshot_png(&snapshot, &owner, &agent.public_key(), None).unwrap();

        let payload = extract_chunk_payload_png(&png).unwrap();
        let ChunkPayload::Locked(env) = parse_chunk_payload(&payload).unwrap() else {
            panic!("locked PNG must parse as a locked envelope");
        };
        // Both endpoints decrypt to the same logical manifest (compare
        // manifests, never ciphertext — the NIP-44 nonce is random).
        assert_eq!(
            decrypt_envelope(&env, owner.secret_key()).unwrap(),
            snapshot
        );
        assert_eq!(
            decrypt_envelope(&env, agent.secret_key()).unwrap(),
            snapshot
        );
    }

    #[test]
    fn plain_manifest_dispatches_to_plain() {
        let json = encode_snapshot_json(&sample_snapshot()).unwrap();
        let ChunkPayload::Plain(decoded) = parse_chunk_payload(&json).unwrap() else {
            panic!("plain manifest must parse as Plain");
        };
        assert_eq!(*decoded, sample_snapshot());
    }

    #[test]
    fn resolve_unlock_secret_owner_exact_endpoint() {
        let (env, owner, _agent) = locked_envelope();
        let secret = resolve_unlock_secret(&env, Some(&owner), &[]).unwrap();
        assert_eq!(&secret, owner.secret_key());

        // A different identity key is NOT tried.
        let other = Keys::generate();
        assert!(resolve_unlock_secret(&env, Some(&other), &[]).is_none());
        assert!(resolve_unlock_secret(&env, None, &[]).is_none());
    }

    #[test]
    fn resolve_unlock_secret_agent_requires_record_and_derived_match() {
        let (env, _owner, agent) = locked_envelope();
        let nsec = nostr::ToBech32::to_bech32(agent.secret_key()).unwrap();

        let record = record_with_keys(agent.public_key().to_hex(), nsec);
        let secret = resolve_unlock_secret(&env, None, std::slice::from_ref(&record)).unwrap();
        assert_eq!(&secret, agent.secret_key());

        // Record pubkey matches but the stored secret derives a DIFFERENT
        // pubkey → refused (no trial decryption on mismatched material).
        let mut forged = record.clone();
        forged.private_key_nsec =
            nostr::ToBech32::to_bech32(Keys::generate().secret_key()).unwrap();
        assert!(resolve_unlock_secret(&env, None, std::slice::from_ref(&forged)).is_none());

        // Record for some other agent → not an endpoint.
        let mut unrelated = record;
        unrelated.pubkey = Keys::generate().public_key().to_hex();
        assert!(resolve_unlock_secret(&env, None, std::slice::from_ref(&unrelated)).is_none());
    }
}
