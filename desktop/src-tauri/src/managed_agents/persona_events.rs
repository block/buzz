//! Serialize `PersonaRecord` ↔ kind:30175 persona events and publish/fetch via relay.
//!
//! Persona events are NIP-33 parameterized replaceable events keyed by
//! `(pubkey, kind, d_tag)` where `d_tag` is the plaintext persona slug.

use std::collections::BTreeMap;

use buzz_core_pkg::kind::KIND_PERSONA;
use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use super::PersonaRecord;
use crate::app_state::AppState;

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

/// Build a NIP-09 deletion (kind:5) targeting a persona's kind:30175 event.
///
/// Carries a single `a`-tag with the NIP-33 coordinate `30175:<owner>:<d_tag>`
/// and no `e`-tag: an `e`-tag routes the relay to the event-id deletion path,
/// which leaves the parameterized-replaceable coordinate live. The coordinate
/// delete removes the persona for every client and across reboots.
pub fn build_persona_delete(d_tag: &str, owner_pubkey_hex: &str) -> Result<EventBuilder, String> {
    let coord = format!("{KIND_PERSONA}:{owner_pubkey_hex}:{d_tag}");
    let tag = Tag::parse(["a", coord.as_str()]).map_err(|e| format!("invalid a-tag: {e}"))?;
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(vec![tag]))
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

/// Drain every `pending_sync` event from the retention store to the relay.
///
/// Each writer (UI create/edit, delete tombstone, launch reconcile) retains a
/// signed event with `pending_sync = 1`; this loop is the sole publisher.
///
/// Per row, the last synchronous read before the network `.await` is a fresh
/// `get_retained_event` re-check — the connection holds no `Mutex` across the
/// await, so a concurrent edit or delete is observed here:
/// - gone (deleted): skip, nothing to publish.
/// - newer `created_at` or different `content`: skip; the newer row is itself
///   `pending_sync` and publishes on its own pass.
///
/// Only a row that still matches what we read is published, then cleared via
/// `mark_synced` on the exact `created_at`+`content` the relay accepted — so an
/// edit landing between publish and clear is never falsely marked synced.
///
/// Returns the number of events the relay accepted. Best-effort: a relay
/// failure on one row leaves it pending for the next sweep and does not abort
/// the remaining rows.
pub async fn flush_pending_events(
    db_path: &std::path::Path,
    state: &AppState,
) -> Result<u32, String> {
    use crate::managed_agents::retention::{
        get_pending_sync, get_retained_event, mark_synced, open_retention_db,
    };
    use nostr::JsonUtil;

    let pending = {
        let conn = open_retention_db(db_path)?;
        get_pending_sync(&conn)?
    }; // connection dropped before any .await

    let mut flushed = 0u32;
    for row in pending {
        // Re-read immediately before publishing; the row may have been edited
        // or deleted since the pending snapshot above.
        let current = {
            let conn = open_retention_db(db_path)?;
            get_retained_event(&conn, row.kind, &row.pubkey, &row.d_tag)?
        };
        let Some(current) = current else {
            continue; // deleted out from under us
        };
        if current.created_at != row.created_at || current.content != row.content {
            continue; // superseded by a newer edit; that row publishes itself
        }

        let event = nostr::Event::from_json(&current.raw_event)
            .map_err(|e| format!("failed to parse retained event '{}': {e}", current.d_tag))?;

        if crate::relay::submit_signed_event(&event, state)
            .await
            .is_err()
        {
            continue; // relay unreachable — stays pending for the next sweep
        }

        let conn = open_retention_db(db_path)?;
        mark_synced(
            &conn,
            current.kind,
            &current.pubkey,
            &current.d_tag,
            current.created_at,
            &current.content,
        )?;
        flushed += 1;
    }

    Ok(flushed)
}

/// SHA-256 (lowercase hex) of a persona's canonical content JSON.
///
/// The drift indicator compares this digest, not event timestamps, to decide
/// whether an agent's persona snapshot is stale — timestamps are fragile across
/// clock skew and export/import round-trips. `PersonaEventContent` field order
/// is fixed by the struct definition, so `serde_json` produces a stable
/// canonical encoding.
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

/// A persona's spawn-relevant config, pinned onto a `ManagedAgentRecord` at
/// create time. After the snapshot, spawn and deploy read these fields off the
/// record and never the live persona, so an agent stays pinned to the config
/// it was created with — restart reuses the snapshot, delete+respawn rewrites
/// it.
pub struct PersonaSnapshot {
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// Persona env layered under the agent's own overrides (agent wins). This
    /// is the complete env map the agent spawns with — no live persona lookup.
    pub env_vars: BTreeMap<String, String>,
    /// `persona_content_hash` of the persona at snapshot time; the drift basis.
    pub source_version: String,
}

/// Build the pinned snapshot for an agent created from `persona`.
///
/// `agent_env_overrides` are the agent's own env vars (persona-independent);
/// they win over persona env on key collision, matching spawn-time precedence
/// (persona env < agent env). The persona's `system_prompt` is always present,
/// so it is wrapped in `Some`.
pub fn persona_snapshot(
    persona: &PersonaRecord,
    agent_env_overrides: &BTreeMap<String, String>,
) -> PersonaSnapshot {
    let mut env_vars = persona.env_vars.clone();
    for (key, value) in agent_env_overrides {
        env_vars.insert(key.clone(), value.clone());
    }
    PersonaSnapshot {
        system_prompt: Some(persona.system_prompt.clone()),
        model: persona.model.clone(),
        provider: persona.provider.clone(),
        env_vars,
        source_version: persona_content_hash(&persona_event_content(persona)),
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
    fn build_persona_delete_has_single_a_tag_no_e_tag() {
        const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let builder = build_persona_delete("test-slug", OWNER).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();

        assert_eq!(event.kind, Kind::Custom(5));

        let a_tags: Vec<&[String]> = event
            .tags
            .iter()
            .map(|t| t.as_slice())
            .filter(|v| v.first().map(String::as_str) == Some("a"))
            .collect();
        assert_eq!(a_tags.len(), 1);
        assert_eq!(a_tags[0][1], format!("{KIND_PERSONA}:{OWNER}:test-slug"));

        // An e-tag would route to the event-id deletion path and leave the
        // replaceable coordinate live — the tombstone must carry none.
        assert!(event
            .tags
            .iter()
            .all(|t| t.as_slice().first().map(String::as_str) != Some("e")));
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
