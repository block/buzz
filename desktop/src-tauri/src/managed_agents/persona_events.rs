//! Serialize `PersonaRecord` ↔ kind:30175 persona events and publish/fetch via relay.
//!
//! Persona events are NIP-33 parameterized replaceable events keyed by
//! `(pubkey, kind, d_tag)` where `d_tag` is the plaintext persona slug.

use std::collections::BTreeMap;

use buzz_core_pkg::kind::KIND_PERSONA;
use nostr::{EventBuilder, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};

use super::PersonaRecord;
use crate::app_state::AppState;

/// The slug for the per-agent persona memory engram. The agent stores a
/// snapshot of the persona it was instantiated from under this memory slot.
pub const PERSONA_ENGRAM_SLUG: &str = "mem/persona";

/// The JSON body stored in a persona event's content field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaEventContent {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
}

/// Derive the d-tag (persona slug) from a `PersonaRecord`.
///
/// Uses `source_team_persona_slug` if available, otherwise falls back to `id`.
pub fn persona_d_tag(record: &PersonaRecord) -> String {
    record
        .source_team_persona_slug
        .as_deref()
        .unwrap_or(&record.id)
        .to_string()
}

/// Build a kind:30175 event from a `PersonaRecord`.
///
/// Returns an unsigned `EventBuilder` — the caller signs and submits.
pub fn build_persona_event(record: &PersonaRecord) -> Result<EventBuilder, String> {
    let content = PersonaEventContent {
        display_name: record.display_name.clone(),
        avatar_url: record.avatar_url.clone(),
        system_prompt: record.system_prompt.clone(),
        runtime: record.runtime.clone(),
        model: record.model.clone(),
        provider: record.provider.clone(),
        name_pool: record.name_pool.clone(),
    };

    let content_json = serde_json::to_string(&content)
        .map_err(|e| format!("failed to serialize persona content: {e}"))?;

    let d_tag = persona_d_tag(record);
    let tags = vec![Tag::parse(["d", d_tag.as_str()]).map_err(|e| format!("invalid d-tag: {e}"))?];

    Ok(EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), content_json).tags(tags))
}

/// Parse a kind:30175 event back into a `PersonaRecord`.
///
/// The event's d-tag becomes the persona ID and slug.
pub fn persona_from_event(event: &nostr::Event) -> Result<PersonaRecord, String> {
    let d_tag = event
        .tags
        .iter()
        .find_map(|tag| {
            let values: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
            if values.first() == Some(&"d") {
                values.get(1).map(|s| s.to_string())
            } else {
                None
            }
        })
        .ok_or("persona event missing d-tag")?;

    let content: PersonaEventContent = serde_json::from_str(event.content.as_ref())
        .map_err(|e| format!("failed to parse persona event content: {e}"))?;

    let created_at = event.created_at.to_human_datetime();

    Ok(PersonaRecord {
        id: d_tag.clone(),
        display_name: content.display_name,
        avatar_url: content.avatar_url,
        system_prompt: content.system_prompt,
        runtime: content.runtime,
        model: content.model,
        provider: content.provider,
        name_pool: content.name_pool,
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: Some(d_tag),
        env_vars: BTreeMap::new(),
        created_at: created_at.clone(),
        updated_at: created_at,
    })
}

/// Publish a persona event to the relay.
pub async fn publish_persona_event(
    record: &PersonaRecord,
    state: &AppState,
) -> Result<String, String> {
    let builder = build_persona_event(record)?;
    let response = crate::relay::submit_event(builder, state).await?;
    Ok(response.event_id)
}

/// Fetch all persona events authored by the current user from the relay.
pub async fn fetch_persona_events(state: &AppState) -> Result<Vec<nostr::Event>, String> {
    let pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    let filter = serde_json::json!({
        "kinds": [KIND_PERSONA],
        "authors": [pubkey]
    });

    crate::relay::query_relay(state, &[filter]).await
}

/// Provenance recorded inside a persona engram's encrypted body. Identifies the
/// source persona event the agent was instantiated from and a content digest
/// used by fleet update to detect drift. Kept inside the encrypted body (not as
/// a plaintext tag) so the engram's blinding guarantee is preserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaProvenance {
    pub owner_pubkey: String,
    pub kind: u32,
    pub slug: String,
    /// SHA-256 of the canonical persona content JSON at the time of the write.
    pub source_version: String,
}

/// The decrypted body of a `mem/persona` engram: the persona snapshot plus its
/// provenance. Serialized as the engram's memory `value` string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaEngramBody {
    #[serde(flatten)]
    pub content: PersonaEventContent,
    pub provenance: PersonaProvenance,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
}

/// SHA-256 (lowercase hex) of a persona's canonical content JSON.
///
/// Fleet update compares this digest, not event timestamps, to decide whether
/// an agent's engram is stale — timestamps are fragile across clock skew and
/// export/import round-trips. `PersonaEventContent` field order is fixed by the
/// struct definition, so `serde_json` produces a stable canonical encoding.
pub fn persona_content_hash(content: &PersonaEventContent) -> String {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(content).unwrap_or_default();
    let digest = Sha256::digest(&json);
    hex::encode(digest)
}

/// Project a `PersonaRecord` onto the content fields published in persona
/// events and engrams. Centralizes the field mapping so a new persona field is
/// added in exactly one place.
pub fn persona_event_content(record: &PersonaRecord) -> PersonaEventContent {
    PersonaEventContent {
        display_name: record.display_name.clone(),
        avatar_url: record.avatar_url.clone(),
        system_prompt: record.system_prompt.clone(),
        runtime: record.runtime.clone(),
        model: record.model.clone(),
        provider: record.provider.clone(),
        name_pool: record.name_pool.clone(),
    }
}

/// Build the decrypted body for a persona engram from a `PersonaRecord`.
fn persona_engram_body(record: &PersonaRecord, owner_pubkey: &PublicKey) -> PersonaEngramBody {
    let content = persona_event_content(record);
    let source_version = persona_content_hash(&content);
    let provenance = PersonaProvenance {
        owner_pubkey: owner_pubkey.to_hex(),
        kind: KIND_PERSONA,
        slug: persona_d_tag(record),
        source_version,
    };
    PersonaEngramBody {
        content,
        provenance,
        env_vars: record.env_vars.clone(),
    }
}

/// Build a signed `mem/persona` engram (kind:30174) for an agent.
///
/// The engram is authored by the agent and addressed to the owner: its content
/// is NIP-44 encrypted under `ECDH(agent_seckey, owner_pubkey)` and the d-tag is
/// HMAC-blinded over the `mem/persona` slug. The persona snapshot and its
/// provenance live inside the encrypted body.
pub fn build_persona_engram(
    agent_keys: &nostr::Keys,
    owner_pubkey: &PublicKey,
    record: &PersonaRecord,
    created_at: u64,
) -> Result<nostr::Event, String> {
    let body = persona_engram_body(record, owner_pubkey);
    let value = serde_json::to_string(&body)
        .map_err(|e| format!("failed to serialize engram body: {e}"))?;

    let engram_body = buzz_core_pkg::engram::Body::Memory {
        slug: PERSONA_ENGRAM_SLUG.to_string(),
        value: Some(value),
    };

    buzz_core_pkg::engram::build_event(agent_keys, owner_pubkey, &engram_body, created_at)
        .map_err(|e| format!("failed to build persona engram: {e}"))
}

/// Decode a persona engram body from a relay event, validating the envelope and
/// decrypting under the agent↔owner conversation key.
pub fn persona_engram_from_event(
    event: &nostr::Event,
    agent_keys: &nostr::Keys,
    owner_pubkey: &PublicKey,
) -> Result<PersonaEngramBody, String> {
    let body = buzz_core_pkg::engram::validate_and_decrypt(
        event,
        &agent_keys.public_key(),
        owner_pubkey,
        agent_keys.secret_key(),
        owner_pubkey,
    )
    .map_err(|e| format!("failed to validate persona engram: {e}"))?;

    engram_body_from_decrypted(body)
}

/// Decode a persona engram as the owner (fleet update reads engrams using the
/// owner's secret key + agent's pubkey — the conversation key is symmetric).
pub fn persona_engram_from_event_as_owner(
    event: &nostr::Event,
    agent_pubkey: &PublicKey,
    owner_keys: &nostr::Keys,
) -> Result<PersonaEngramBody, String> {
    let body = buzz_core_pkg::engram::validate_and_decrypt(
        event,
        agent_pubkey,
        &owner_keys.public_key(),
        owner_keys.secret_key(),
        agent_pubkey,
    )
    .map_err(|e| format!("failed to validate persona engram: {e}"))?;

    engram_body_from_decrypted(body)
}

fn engram_body_from_decrypted(
    body: buzz_core_pkg::engram::Body,
) -> Result<PersonaEngramBody, String> {
    match body {
        buzz_core_pkg::engram::Body::Memory {
            value: Some(value), ..
        } => serde_json::from_str(&value)
            .map_err(|e| format!("failed to parse persona engram body: {e}")),
        buzz_core_pkg::engram::Body::Memory { value: None, .. } => {
            Err("persona engram is a tombstone".to_string())
        }
        buzz_core_pkg::engram::Body::Core { .. } => {
            Err("expected memory engram, got core".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_persona() -> PersonaRecord {
        PersonaRecord {
            id: "test-persona".to_string(),
            display_name: "Test Persona".to_string(),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            system_prompt: "You are a test assistant.".to_string(),
            runtime: Some("goose".to_string()),
            model: Some("claude-opus-4".to_string()),
            provider: Some("anthropic".to_string()),
            name_pool: vec!["Alpha".to_string(), "Beta".to_string()],
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: Some("test-slug".to_string()),
            env_vars: BTreeMap::from([("KEY".to_string(), "value".to_string())]),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn d_tag_uses_slug_when_available() {
        let record = sample_persona();
        assert_eq!(persona_d_tag(&record), "test-slug");
    }

    #[test]
    fn d_tag_falls_back_to_id() {
        let mut record = sample_persona();
        record.source_team_persona_slug = None;
        assert_eq!(persona_d_tag(&record), "test-persona");
    }

    #[test]
    fn build_persona_event_produces_correct_kind() {
        let record = sample_persona();
        let builder = build_persona_event(&record).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind.as_u16() as u32, KIND_PERSONA);
    }

    #[test]
    fn round_trip_serialization() {
        let record = sample_persona();
        let builder = build_persona_event(&record).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();

        let restored = persona_from_event(&event).unwrap();
        assert_eq!(restored.id, "test-slug");
        assert_eq!(restored.display_name, "Test Persona");
        assert_eq!(
            restored.avatar_url,
            Some("https://example.com/avatar.png".to_string())
        );
        assert_eq!(restored.system_prompt, "You are a test assistant.");
        assert_eq!(restored.runtime, Some("goose".to_string()));
        assert_eq!(restored.model, Some("claude-opus-4".to_string()));
        assert_eq!(restored.provider, Some("anthropic".to_string()));
        assert_eq!(restored.name_pool, vec!["Alpha", "Beta"]);
        // env_vars are not included in public persona events (secrets travel
        // via NIP-44-encrypted engrams only).
        assert!(restored.env_vars.is_empty());
        assert_eq!(
            restored.source_team_persona_slug,
            Some("test-slug".to_string())
        );
        assert!(!restored.is_builtin);
        assert!(restored.is_active);
    }

    #[test]
    fn round_trip_minimal_persona() {
        let record = PersonaRecord {
            id: "minimal".to_string(),
            display_name: "Minimal".to_string(),
            avatar_url: None,
            system_prompt: "Hello".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
            is_builtin: true,
            is_active: false,
            source_team: Some("team-1".to_string()),
            source_team_persona_slug: None,
            env_vars: BTreeMap::new(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };

        let builder = build_persona_event(&record).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();

        let restored = persona_from_event(&event).unwrap();
        assert_eq!(restored.id, "minimal");
        assert_eq!(restored.display_name, "Minimal");
        assert_eq!(restored.avatar_url, None);
        assert_eq!(restored.runtime, None);
        assert_eq!(restored.model, None);
        assert_eq!(restored.provider, None);
        assert!(restored.name_pool.is_empty());
        assert!(restored.env_vars.is_empty());
        // Deserialized persona is always non-builtin and active
        assert!(!restored.is_builtin);
        assert!(restored.is_active);
    }

    #[test]
    fn persona_engram_round_trip() {
        let record = sample_persona();
        let agent_keys = nostr::Keys::generate();
        let owner_keys = nostr::Keys::generate();
        let owner_pubkey = owner_keys.public_key();

        let now = 1_700_000_000u64;
        let event = build_persona_engram(&agent_keys, &owner_pubkey, &record, now).unwrap();

        // Verify it's a kind:30174 event
        assert_eq!(
            event.kind.as_u16() as u32,
            buzz_core_pkg::kind::KIND_AGENT_ENGRAM
        );
        // Verify it's authored by the agent
        assert_eq!(event.pubkey, agent_keys.public_key());

        // Decrypt and verify content
        let body = persona_engram_from_event(&event, &agent_keys, &owner_pubkey).unwrap();
        assert_eq!(body.content.display_name, "Test Persona");
        assert_eq!(body.content.system_prompt, "You are a test assistant.");
        assert_eq!(body.provenance.owner_pubkey, owner_pubkey.to_hex());
        assert_eq!(body.provenance.kind, KIND_PERSONA);
        assert_eq!(body.provenance.slug, "test-slug");
        assert!(!body.provenance.source_version.is_empty());
        assert_eq!(
            body.env_vars,
            BTreeMap::from([("KEY".to_string(), "value".to_string())])
        );
    }

    #[test]
    fn persona_content_hash_is_deterministic() {
        let content = PersonaEventContent {
            display_name: "Test".to_string(),
            avatar_url: None,
            system_prompt: "Hello".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
        };
        let hash1 = persona_content_hash(&content);
        let hash2 = persona_content_hash(&content);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn persona_content_hash_changes_on_edit() {
        let content1 = PersonaEventContent {
            display_name: "Test".to_string(),
            avatar_url: None,
            system_prompt: "Hello".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
        };
        let mut content2 = content1.clone();
        content2.system_prompt = "Goodbye".to_string();
        assert_ne!(
            persona_content_hash(&content1),
            persona_content_hash(&content2)
        );
    }
}
