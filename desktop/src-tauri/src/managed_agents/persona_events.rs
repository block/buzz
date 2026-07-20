//! Serialize `AgentDefinition` ↔ kind:30175 persona events and publish/fetch via relay.
//!
//! Persona events are NIP-33 parameterized replaceable events keyed by
//! `(pubkey, kind, d_tag)` where `d_tag` is the plaintext persona slug.

use std::collections::BTreeMap;

use buzz_core_pkg::kind::KIND_PERSONA;
use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use super::{AgentDefinition, ManagedAgentRecord};
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
    pub parallelism: Option<u32>,
}

/// Derive the d-tag (persona slug) from a `AgentDefinition`.
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
pub fn persona_d_tag(record: &AgentDefinition) -> String {
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

/// Build a kind:30175 event from a `AgentDefinition`.
///
/// Returns an unsigned `EventBuilder` — the caller signs and submits.
pub fn build_persona_event(record: &AgentDefinition) -> Result<EventBuilder, String> {
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

/// Parse a kind:30175 event back into a `AgentDefinition`.
///
/// The event's d-tag becomes the persona ID and slug.
pub fn persona_from_event(event: &nostr::Event) -> Result<AgentDefinition, String> {
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

    Ok(AgentDefinition {
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
        respond_to: content.respond_to,
        respond_to_allowlist: content.respond_to_allowlist,
        parallelism: content.parallelism,
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
        deferred_behind_failed_tombstone, get_pending_sync, get_retained_event, mark_synced,
        open_retention_db,
    };
    use nostr::JsonUtil;

    let pending = {
        let conn = open_retention_db(db_path)?;
        get_pending_sync(&conn)?
    }; // connection dropped before any .await

    let mut flushed = 0u32;
    let mut failed_tombstones: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for row in pending {
        if deferred_behind_failed_tombstone(row.kind, &row.pubkey, &row.d_tag, &failed_tombstones) {
            continue; // its tombstone failed this sweep; next sweep re-orders them
        }
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

        // NIP-IA requests are freshness-checked by the relay (±120s on
        // `created_at`), so a request retained while the relay was
        // unreachable would be permanently stale. Re-sign with a fresh
        // timestamp at publish time; kind, tags, and content are preserved,
        // and `mark_synced` below still compares against the retained row's
        // original `created_at`/`content`, which are untouched.
        let is_archive_request =
            buzz_core_pkg::kind::is_identity_archive_request_kind(current.kind);
        let event = if is_archive_request {
            resign_with_fresh_timestamp(&event, state)?
        } else {
            event
        };

        if crate::relay::submit_signed_event(&event, state)
            .await
            .is_err()
        {
            if current.kind == 5 {
                failed_tombstones.insert((current.pubkey.clone(), current.d_tag.clone()));
            }
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

/// Re-sign a retained event with the current owner keys and a fresh
/// `created_at`, preserving kind, tags, and content.
///
/// Used for relay-freshness-checked kinds (NIP-IA 9035/9036) that would
/// otherwise go permanently stale sitting in the retention store while the
/// relay is unreachable. `.allow_self_tagging()` mirrors
/// `events::build_archive_identity_request` — nostr strips `p` tags matching
/// the signer by default, which would corrupt a self-targeted request.
///
/// Synchronous; the `state.keys` guard is dropped on return, so callers may
/// `.await` afterwards.
fn resign_with_fresh_timestamp(
    event: &nostr::Event,
    state: &AppState,
) -> Result<nostr::Event, String> {
    let keys = state.signing_keys()?;
    nostr::EventBuilder::new(event.kind, event.content.clone())
        .tags(event.tags.iter().cloned())
        .allow_self_tagging()
        .sign_with_keys(&keys)
        .map_err(|e| format!("failed to re-sign retained event: {e}"))
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

/// Project a `AgentDefinition` onto the content fields published in persona
/// events and engrams. Centralizes the field mapping so a new persona field is
/// added in exactly one place.
pub fn persona_event_content(record: &AgentDefinition) -> PersonaEventContent {
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
        // NIP-AP behavioral defaults: live since the create-path unification
        // (B5) — carried on AgentDefinition in wire shape and copied verbatim.
        // Quad-absent records serialize identically to the reserved era, so
        // persona_content_hash is stable across the activation (guarded by
        // `quad_absent_definition_hash_stable_across_activation`).
        respond_to: record.respond_to.clone(),
        respond_to_allowlist: record.respond_to_allowlist.clone(),
        parallelism: record.parallelism,
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
    /// Preferred ACP runtime ID, copied verbatim from the persona (including
    /// `None`). Unlike `model`/`provider`, there is no record-fallback: the
    /// materialized instance `runtime` must mirror the definition so that
    /// definition edits propagate on the next spawn rather than being silently
    /// shadowed by the stale materialized value.
    pub runtime: Option<String>,
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
pub fn persona_snapshot(persona: &AgentDefinition) -> PersonaSnapshot {
    PersonaSnapshot {
        system_prompt: Some(persona.system_prompt.clone()),
        model: persona.model.clone(),
        provider: persona.provider.clone(),
        runtime: persona.runtime.clone(),
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
    persona: &AgentDefinition,
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

/// Re-pin `record` to `persona`: build the snapshot (via
/// [`persona_snapshot_with_agent_config_fallback`], so blank persona
/// `model`/`provider` preserve the record's own values) and mirror it onto the
/// record — the definition quad (`system_prompt`/`model`/`provider`/`runtime`),
/// the env-override self-heal, and the `persona_source_version` drift basis.
///
/// This is the single apply used by every snapshot-apply site: the spawn
/// re-pin (`start_local_agent_with_preflight`), the launch backfill and
/// restore re-snapshot (`restore.rs`), and the prospective re-snapshot inside
/// `spawn_config_hash` — so a future `PersonaSnapshot` field addition
/// propagates to all of them at once.
///
/// Deliberately does NOT touch `updated_at`: persistence stamps are the
/// caller's concern, and `spawn_config_hash` (which applies this to a clone)
/// must stay pure.
pub fn apply_persona_snapshot(record: &mut ManagedAgentRecord, persona: &AgentDefinition) {
    let snapshot = persona_snapshot_with_agent_config_fallback(
        persona,
        record.model.as_deref(),    // fallback: record.model
        record.provider.as_deref(), // fallback: record.provider
    );
    if let Some(prompt) = snapshot.system_prompt {
        record.system_prompt = Some(prompt);
    }
    record.model = snapshot.model;
    record.provider = snapshot.provider;
    record.runtime = snapshot.runtime;
    // env_vars stay overrides-only. Self-heal records written before the env
    // refresh: persona env used to be baked into `record.env_vars`, turning
    // inherited values into pseudo-overrides that shadow later persona edits.
    // An override equal to the persona's current value is indistinguishable
    // from inheritance, so drop it and let the live merge supply it.
    record
        .env_vars
        .retain(|k, v| persona.env_vars.get(k) != Some(v));
    record.persona_source_version = Some(snapshot.source_version);
}
#[cfg(test)]
mod tests;
