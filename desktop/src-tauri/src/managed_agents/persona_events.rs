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
///
/// Field order MUST match the NIP-AP reference vectors (`docs/nips/NIP-AP.md`
/// content body: `display_name, system_prompt, avatar_url, runtime, model,
/// provider, name_pool`). serde emits fields in declaration order, so this
/// order pins the exact content bytes and therefore the NIP-01 event id — a
/// reorder here breaks cross-implementation interop. Guarded by
/// `content_matches_nip_ap_vector`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaEventContent {
    pub display_name: String,
    /// Optional since the unified agent model (NIP-AP revision): a definition
    /// can be pure configuration. Writers emit `Some` whenever the record has
    /// a prompt (including the empty string) so pre-revision content bytes —
    /// and therefore `persona_content_hash` — are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    /// Definition-level defaults copied onto instances at creation
    /// (NIP-AP behavioral fields). Absent = defer to client defaults;
    /// `skip_serializing_if` keeps pre-revision hashes stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_toolsets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
}

/// Derive the d-tag (persona slug) from a `PersonaRecord`.
///
/// Uses `source_team_persona_slug` if available, otherwise falls back to `id`,
/// then normalizes to the NIP-AP slug grammar (`^[a-z0-9][a-z0-9_-]{0,63}$`,
/// `docs/nips/NIP-AP.md:27`) via [`normalize_d_tag`]. Team pack slugs are
/// `[a-zA-Z0-9_-]+` (mixed case, may lead with `_`/`-`), so an un-normalized
/// slug like `CodeReviewer` or `_ops` is signed locally but REJECTED by the
/// relay's identical grammar — pending forever. In-app personas use a
/// lowercase-hex UUID `id` that is already valid, so they are unaffected.
///
/// Both the outbound publish and the inbound match key route through this fn,
/// so the normalized value is consistent in both directions and cannot drift.
pub fn persona_d_tag(record: &PersonaRecord) -> String {
    let raw = record
        .source_team_persona_slug
        .as_deref()
        .unwrap_or(&record.id);
    normalize_d_tag(raw)
}

/// Normalize a raw slug to the NIP-AP grammar `^[a-z0-9][a-z0-9_-]{0,63}$`.
///
/// - ASCII-lowercase every char (pack slugs are `[a-zA-Z0-9_-]+`, so this is
///   the only transform uppercase slugs need).
/// - Map any char outside `[a-z0-9_-]` to `-` (defensive; pack slugs never
///   contain such chars, but `id` fallbacks and future inputs might).
/// - If the first char is not `[a-z0-9]` (i.e. a leading `_`/`-`), prepend `a`
///   rather than trimming — trimming `_ops`→`ops` would collide with a real
///   `ops` pack, whereas the prefix keeps distinct inputs distinct.
/// - Truncate to 64 bytes (the grammar's max).
///
/// The transform is deterministic. It is NOT globally injective (`A-b` and
/// `a_b` both contain only safe chars and stay distinct, but two slugs
/// differing only in case — e.g. `Ops` and `ops` — collapse to the same
/// d-tag). That case-fold collision is inherent to the lowercase relay grammar
/// and is the correct NIP-33 behavior: same logical persona, one coordinate.
fn normalize_d_tag(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if !out
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        out.insert(0, 'a');
    }
    out.truncate(64);
    out
}

/// Compute the NIP-AP monotonic `created_at` for a write (`docs/nips/NIP-AP.md:117`
/// step 3): `max(now, T + 1)` where `T` is the retained head's `created_at`
/// (or 0 when no head exists).
///
/// NIP-33 keeps the greatest `created_at` per coordinate, breaking ties by
/// lowest event id. The local retention upsert (`retain_event`) replaces on
/// `>=`, so without this bump a same-second second edit is kept LOCALLY while
/// the relay's lowest-id tiebreak may keep the OLDER event — divergence, and
/// the flush can mark the local row synced against a head the relay rejected.
/// Bumping past the head guarantees a fresh write always supersedes regardless
/// of clock skew.
pub fn monotonic_created_at(prior_head_created_at: Option<i64>) -> nostr::Timestamp {
    let now = nostr::Timestamp::now().as_secs() as i64;
    let floor = prior_head_created_at.map_or(0, |t| t + 1);
    nostr::Timestamp::from(now.max(floor) as u64)
}

/// Build a kind:30175 event from a `PersonaRecord`.
///
/// Returns an unsigned `EventBuilder` — the caller signs and submits.
pub fn build_persona_event(record: &PersonaRecord) -> Result<EventBuilder, String> {
    // Single projection point — persona_event_content owns the field mapping
    // (and the hash-stability rules that come with it).
    let content = persona_event_content(record);

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
        system_prompt: content.system_prompt.unwrap_or_default(),
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
        // Always Some — including for an empty prompt — so pre-revision
        // records serialize byte-identically and persona_content_hash is
        // stable across the upgrade (drift badges must not flip).
        system_prompt: Some(record.system_prompt.clone()),
        runtime: record.runtime.clone(),
        model: record.model.clone(),
        provider: record.provider.clone(),
        name_pool: record.name_pool.clone(),
        // NIP-AP behavioral defaults: RESERVED this release — parsed at the
        // wire layer but not yet carried on PersonaRecord or applied at
        // instance creation (that lands with the create-path unification).
        // Guarded by `behavioral_defaults_are_staged_not_applied`.
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        mcp_toolsets: None,
        parallelism: None,
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
    /// `persona_content_hash` of the persona at snapshot time; the drift basis.
    pub source_version: String,
}

/// Apply persona-wins-when-set precedence for a single optional string field.
///
/// Returns the persona's value when it is non-`None` and non-whitespace-only;
/// otherwise falls back to the record's value with the same blank filter applied.
/// Returns `None` only when both are blank — a genuinely unconfigured field stays
/// unconfigured.
///
/// This is the single source of truth for the precedence rule used by
/// `persona_snapshot_with_agent_config_fallback`, `build_deploy_payload`, and
/// `resolve_effective_prompt_model_provider` so the three paths cannot drift.
pub fn persona_field_with_record_fallback(
    persona_value: Option<&str>,
    record_value: Option<&str>,
) -> Option<String> {
    let non_blank = |v: Option<&str>| v.filter(|s| !s.trim().is_empty()).map(str::to_owned);
    non_blank(persona_value).or_else(|| non_blank(record_value))
}

/// Build the pinned snapshot for an agent created from `persona`.
///
/// The persona's `system_prompt` is always present, so it is wrapped in
/// `Some`. Env vars are deliberately absent: `record.env_vars` holds agent
/// overrides only, and the live persona env is merged underneath at read
/// time (spawn / readiness / deploy) — never snapshotted.
pub fn persona_snapshot(persona: &PersonaRecord) -> PersonaSnapshot {
    PersonaSnapshot {
        system_prompt: Some(persona.system_prompt.clone()),
        model: persona.model.clone(),
        provider: persona.provider.clone(),
        source_version: persona_content_hash(&persona_event_content(persona)),
    }
}

/// Build the pinned snapshot for an **existing** agent record being re-snapshotted
/// from its linked persona (on spawn or app-launch restore).
///
/// Precedence rule: when the persona sets `model` or `provider` (non-`None`, non-empty),
/// the persona wins — this is the expected inheritance. When the persona leaves
/// these fields blank (`None` or empty string), the agent record's own values are
/// preserved instead. This prevents a persona with no configured model/provider from
/// clobbering a value the user already set on the agent, which would trap the agent
/// in a permanent "needs configuration" loop that users cannot escape.
///
/// `source_version` is always updated to the current persona content hash so the
/// drift badge clears correctly even when model/provider are not touched.
///
/// Env vars are not part of the snapshot: `record.env_vars` (agent overrides)
/// is left untouched and the live persona env is merged underneath at read time.
///
/// The two fields (`model`, `provider`) are independent: a persona that sets only
/// `model` wins on `model` while the agent's `provider` is preserved, and vice versa.
pub fn persona_snapshot_with_agent_config_fallback(
    persona: &PersonaRecord,
    current_agent_model: Option<&str>,
    current_agent_provider: Option<&str>,
) -> PersonaSnapshot {
    // Delegate system_prompt and source_version to persona_snapshot so future
    // PersonaSnapshot field additions stay automatically consistent.
    let base = persona_snapshot(persona);

    // Apply the shared precedence rule: persona wins when non-blank, else
    // the agent record's value is preserved so a configured agent stays configured.
    let model = persona_field_with_record_fallback(base.model.as_deref(), current_agent_model);
    let provider =
        persona_field_with_record_fallback(base.provider.as_deref(), current_agent_provider);

    PersonaSnapshot {
        model,
        provider,
        ..base
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
    fn monotonic_created_at_bumps_past_head() {
        // No head: uses now (floor 0).
        let now = nostr::Timestamp::now().as_secs() as i64;
        let none = monotonic_created_at(None).as_secs() as i64;
        assert!(none >= now, "no-head write must be >= now");

        // Head in the FUTURE (same-second or clock-skewed): must bump to head+1,
        // never reuse now (which would be <= head and lose the NIP-33 tiebreak).
        let future_head = now + 1000;
        let bumped = monotonic_created_at(Some(future_head)).as_secs() as i64;
        assert_eq!(
            bumped,
            future_head + 1,
            "must supersede a future head by +1"
        );

        // Head in the PAST: now already exceeds it, so now wins.
        let past = monotonic_created_at(Some(now - 1000)).as_secs() as i64;
        assert!(past >= now, "past head must not drag created_at backward");
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

    /// Mirror of the relay slug grammar (`ingest.rs:923` `^[a-z0-9][a-z0-9_-]{0,63}$`)
    /// so the normalization tests assert what the relay actually enforces.
    fn passes_relay_slug_grammar(d: &str) -> bool {
        let bytes = d.as_bytes();
        !d.is_empty()
            && d.len() <= 64
            && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
            && bytes[1..]
                .iter()
                .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    }

    #[test]
    fn d_tag_normalizes_pack_slug_to_relay_grammar() {
        // The cited failing cases: mixed-case and leading-underscore pack slugs
        // that the relay rejects un-normalized → pending forever.
        for (raw, expected) in [
            ("CodeReviewer", "codereviewer"),
            ("_ops", "a_ops"),
            ("Code-Reviewer", "code-reviewer"),
            ("UPPER_snake", "upper_snake"),
            ("-leading-dash", "a-leading-dash"),
        ] {
            let mut record = sample_persona();
            record.source_team_persona_slug = Some(raw.to_string());
            let d = persona_d_tag(&record);
            assert_eq!(d, expected, "normalization of {raw:?}");
            assert!(
                passes_relay_slug_grammar(&d),
                "normalized {raw:?} -> {d:?} still fails the relay grammar"
            );
        }
    }

    #[test]
    fn d_tag_already_valid_slug_is_unchanged() {
        // In-app personas use a lowercase-hex UUID id — already valid, must pass
        // through untouched (no spurious coordinate change on existing data).
        let mut record = sample_persona();
        record.source_team_persona_slug = None;
        record.id = "11111111-2222-3333-4444-555555555555".to_string();
        let d = persona_d_tag(&record);
        assert_eq!(d, "11111111-2222-3333-4444-555555555555");
        assert!(passes_relay_slug_grammar(&d));
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

    /// NIP-AP reference vector (Event 1, `docs/nips/NIP-AP.md:195-207`): the
    /// serialized content bytes MUST match the spec exactly, byte-for-byte.
    /// serde emits fields in declaration order, so this pins the content
    /// encoding — and therefore the NIP-01 event id — for cross-implementation
    /// interop. The field order is `display_name, system_prompt, avatar_url,
    /// runtime, model, provider, name_pool`.
    #[test]
    fn content_matches_nip_ap_vector() {
        // Exact body from NIP-AP.md Event 1 (no trailing whitespace, no BOM).
        const VECTOR: &str = r#"{"display_name":"Test Agent","system_prompt":"You are a test assistant.","avatar_url":"https://example.com/avatar.png","runtime":"goose","model":"claude-opus-4","provider":"anthropic","name_pool":["Alpha","Beta"]}"#;

        let content = PersonaEventContent {
            display_name: "Test Agent".to_string(),
            system_prompt: Some("You are a test assistant.".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            runtime: Some("goose".to_string()),
            model: Some("claude-opus-4".to_string()),
            provider: Some("anthropic".to_string()),
            name_pool: vec!["Alpha".to_string(), "Beta".to_string()],
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            mcp_toolsets: None,
            parallelism: None,
        };
        assert_eq!(
            serde_json::to_string(&content).unwrap(),
            VECTOR,
            "serialized content drifted from the NIP-AP Event 1 vector"
        );

        // Hash invariance across the unified-model widening: REAL pre-revision
        // content bytes (fixture string, not a round-trip through the new
        // struct) must parse and re-serialize byte-identically, so
        // persona_content_hash — the drift-badge basis — is unchanged on
        // upgrade. A bare Option serializing "system_prompt":null would flip
        // every persona's hash fleet-wide.
        let parsed: PersonaEventContent = serde_json::from_str(VECTOR).unwrap();
        assert_eq!(
            serde_json::to_string(&parsed).unwrap(),
            VECTOR,
            "pre-revision content bytes must survive a parse/serialize round-trip unchanged"
        );
        assert_eq!(
            persona_content_hash(&parsed),
            {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(VECTOR.as_bytes()))
            },
            "persona_content_hash of pre-revision bytes must equal the direct digest"
        );

        // Hash stability, adversarial shapes: the empty prompt and the
        // minimal old-writer body (display_name + system_prompt only) are the
        // two easiest regressions if the projection or skip attributes ever
        // change.
        const EMPTY_PROMPT: &str = r#"{"display_name":"X","system_prompt":""}"#;
        let parsed: PersonaEventContent = serde_json::from_str(EMPTY_PROMPT).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), EMPTY_PROMPT);
        const MINIMAL: &str = r#"{"display_name":"Minimal","system_prompt":"Hello."}"#;
        let parsed: PersonaEventContent = serde_json::from_str(MINIMAL).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), MINIMAL);

        // An event built from this content carries the byte-exact vector as its
        // signed content, so a second implementer following the spec computes
        // the same NIP-01 id.
        let record = PersonaRecord {
            id: "test-agent".to_string(),
            display_name: "Test Agent".to_string(),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            system_prompt: "You are a test assistant.".to_string(),
            runtime: Some("goose".to_string()),
            model: Some("claude-opus-4".to_string()),
            provider: Some("anthropic".to_string()),
            name_pool: vec!["Alpha".to_string(), "Beta".to_string()],
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: BTreeMap::new(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let event = build_persona_event(&record)
            .unwrap()
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        assert_eq!(event.content, VECTOR);
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

    /// NIP-AP behavioral defaults are RESERVED this release: the wire type
    /// parses them (foreign data survives deserialization) but the local
    /// projection does not emit them and PersonaRecord does not carry them.
    /// This test locks the staged behavior so activating the fields later is
    /// a deliberate act — and documents that a foreign definition's
    /// behavioral values do NOT survive a local edit-and-republish cycle yet.
    #[test]
    fn behavioral_defaults_are_staged_not_applied() {
        const FOREIGN: &str = r#"{"display_name":"F","system_prompt":"p","respond_to":"anyone","respond_to_allowlist":["deadbeef"],"mcp_toolsets":"default","parallelism":4}"#;
        let parsed: PersonaEventContent = serde_json::from_str(FOREIGN).unwrap();
        // Wire layer preserves the fields...
        assert_eq!(parsed.respond_to.as_deref(), Some("anyone"));
        assert_eq!(parsed.parallelism, Some(4));
        // ...but the record round-trip drops them (staged, not applied).
        let record = persona_from_event_content_for_test(parsed);
        let reprojected = persona_event_content(&record);
        assert_eq!(reprojected.respond_to, None);
        assert_eq!(reprojected.respond_to_allowlist, Vec::<String>::new());
        assert_eq!(reprojected.mcp_toolsets, None);
        assert_eq!(reprojected.parallelism, None);
    }

    /// Test-only bridge: build a PersonaRecord from parsed content the same
    /// way `persona_from_event` maps fields, without needing a signed event.
    fn persona_from_event_content_for_test(content: PersonaEventContent) -> PersonaRecord {
        PersonaRecord {
            id: "staged".to_string(),
            display_name: content.display_name,
            avatar_url: content.avatar_url,
            system_prompt: content.system_prompt.unwrap_or_default(),
            runtime: content.runtime,
            model: content.model,
            provider: content.provider,
            name_pool: content.name_pool,
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: BTreeMap::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn persona_content_hash_is_deterministic() {
        let content = PersonaEventContent {
            display_name: "Test".to_string(),
            avatar_url: None,
            system_prompt: Some("Hello".to_string()),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            mcp_toolsets: None,
            parallelism: None,
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
            system_prompt: Some("Hello".to_string()),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            mcp_toolsets: None,
            parallelism: None,
        };
        let mut content2 = content1.clone();
        content2.system_prompt = Some("Goodbye".to_string());
        assert_ne!(
            persona_content_hash(&content1),
            persona_content_hash(&content2)
        );
    }

    // ── persona_field_with_record_fallback ────────────────────────────────────

    #[test]
    fn field_fallback_persona_present_wins() {
        assert_eq!(
            persona_field_with_record_fallback(Some("persona-model"), Some("record-model")),
            Some("persona-model".to_owned()),
        );
    }

    #[test]
    fn field_fallback_persona_blank_uses_record() {
        assert_eq!(
            persona_field_with_record_fallback(None, Some("record-model")),
            Some("record-model".to_owned()),
        );
        assert_eq!(
            persona_field_with_record_fallback(Some("  "), Some("record-model")),
            Some("record-model".to_owned()),
        );
    }

    #[test]
    fn field_fallback_both_blank_is_none() {
        assert_eq!(persona_field_with_record_fallback(None, None), None);
        assert_eq!(persona_field_with_record_fallback(Some(""), Some("")), None);
    }

    #[test]
    fn field_fallback_record_blank_is_none() {
        assert_eq!(
            persona_field_with_record_fallback(None, Some("  ")),
            None,
            "whitespace-only record value must also be treated as blank"
        );
    }

    // ── persona_snapshot_with_agent_config_fallback ────────────────────────────

    /// Helper: a persona with no model/provider configured.
    fn blank_model_persona() -> PersonaRecord {
        PersonaRecord {
            model: None,
            provider: None,
            ..sample_persona()
        }
    }

    /// (a) Persona leaves model/provider blank, agent record has values →
    /// record values preserved AND source_version still updated to current hash.
    #[test]
    fn fallback_preserves_record_values_when_persona_blank() {
        let persona = blank_model_persona();
        let expected_version = persona_content_hash(&persona_event_content(&persona));

        let snapshot =
            persona_snapshot_with_agent_config_fallback(&persona, Some("gpt-4o"), Some("openai"));

        assert_eq!(
            snapshot.model.as_deref(),
            Some("gpt-4o"),
            "blank persona model must fall back to agent record value"
        );
        assert_eq!(
            snapshot.provider.as_deref(),
            Some("openai"),
            "blank persona provider must fall back to agent record value"
        );
        assert_eq!(
            snapshot.source_version, expected_version,
            "source_version must still reflect current persona hash"
        );
    }

    /// (b) Persona has model/provider set → persona wins over agent record.
    #[test]
    fn fallback_persona_wins_when_set() {
        let persona = sample_persona(); // has model=Some("claude-opus-4"), provider=Some("anthropic")

        let snapshot = persona_snapshot_with_agent_config_fallback(
            &persona,
            Some("gpt-4o"), // agent had a different model
            Some("openai"), // agent had a different provider
        );

        assert_eq!(
            snapshot.model.as_deref(),
            Some("claude-opus-4"),
            "persona model must win when persona has a value"
        );
        assert_eq!(
            snapshot.provider.as_deref(),
            Some("anthropic"),
            "persona provider must win when persona has a value"
        );
    }

    /// (c) Both blank → snapshot keeps None; a genuinely unconfigured agent
    /// stays unconfigured (no fabricated values).
    #[test]
    fn fallback_both_blank_stays_none() {
        let persona = blank_model_persona();

        let snapshot = persona_snapshot_with_agent_config_fallback(
            &persona, None, // agent also has no model
            None, // agent also has no provider
        );

        assert!(
            snapshot.model.is_none(),
            "neither persona nor agent has model — snapshot must be None"
        );
        assert!(
            snapshot.provider.is_none(),
            "neither persona nor agent has provider — snapshot must be None"
        );
    }

    /// Whitespace-only values on the persona are treated as blank; agent
    /// fallback applies.
    #[test]
    fn fallback_treats_whitespace_only_persona_value_as_blank() {
        let mut persona = sample_persona();
        persona.model = Some("  ".to_string());
        persona.provider = Some("\t".to_string());

        let snapshot = persona_snapshot_with_agent_config_fallback(
            &persona,
            Some("claude-opus-4"),
            Some("anthropic"),
        );

        assert_eq!(
            snapshot.model.as_deref(),
            Some("claude-opus-4"),
            "whitespace-only persona model must be treated as blank"
        );
        assert_eq!(
            snapshot.provider.as_deref(),
            Some("anthropic"),
            "whitespace-only persona provider must be treated as blank"
        );
    }

    /// Cross-field independence: persona sets model but not provider → model
    /// comes from persona, provider falls back to the record.  This is the
    /// practically common case (model-only personas).
    #[test]
    fn fallback_persona_model_set_provider_blank_uses_record_provider() {
        let mut persona = sample_persona(); // model=Some("claude-opus-4"), provider=Some("anthropic")
        persona.provider = None; // blank provider on persona

        let snapshot = persona_snapshot_with_agent_config_fallback(
            &persona,
            Some("gpt-4o"), // record model (should be overridden by persona)
            Some("openai"), // record provider (should be preserved)
        );

        assert_eq!(
            snapshot.model.as_deref(),
            Some("claude-opus-4"),
            "persona model must win when persona has a value"
        );
        assert_eq!(
            snapshot.provider.as_deref(),
            Some("openai"),
            "record provider must be used when persona provider is blank"
        );
    }

    /// Inverse: persona sets provider but not model → provider comes from
    /// persona, model falls back to the record.
    #[test]
    fn fallback_persona_provider_set_model_blank_uses_record_model() {
        let mut persona = sample_persona(); // model=Some("claude-opus-4"), provider=Some("anthropic")
        persona.model = None; // blank model on persona

        let snapshot = persona_snapshot_with_agent_config_fallback(
            &persona,
            Some("gpt-4o"), // record model (should be preserved)
            Some("openai"), // record provider (should be overridden by persona)
        );

        assert_eq!(
            snapshot.model.as_deref(),
            Some("gpt-4o"),
            "record model must be used when persona model is blank"
        );
        assert_eq!(
            snapshot.provider.as_deref(),
            Some("anthropic"),
            "persona provider must win when persona has a value"
        );
    }
}
