//! Serialize `AgentDefinition` ↔ kind:30175 persona events and publish/fetch via relay.
//!
//! Persona events are NIP-33 parameterized replaceable events keyed by
//! `(pubkey, kind, d_tag)` where `d_tag` is the plaintext persona slug.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use buzz_core_pkg::kind::{event_is_shared, KIND_PERSONA};
use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

use super::{AgentDefinition, ManagedAgentRecord};
use crate::app_state::AppState;

/// Serializes the retention-store flush publisher per `(relay, owner)` scope,
/// keyed by the canonical retention database path. The flush re-reads each row
/// then awaits a relay POST; a second concurrent flush of the SAME scope must
/// not publish a deletion tombstone in that gap and strand a purged head after
/// it. Keying by scope (not process-wide) keeps the serialization no broader
/// than the durable invariant — retention is scoped per `(relay, owner)` — so
/// an unresponsive relay in one community cannot block publication in another.
/// A `LazyLock` static (rather than an `AppState` field) keeps the invariant at
/// its acquisition site and out of the size-ratcheted `app_state.rs`; the map
/// only ever grows one small entry per active scope.
static FLUSH_PUBLISHER_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Resolve the per-scope publisher mutex for `db_path`, inserting one on first
/// use. The std-mutex guard is released before the caller awaits the returned
/// async mutex, so it never spans an await point.
fn flush_publisher_lock(db_path: &std::path::Path) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks: MutexGuard<'_, _> = FLUSH_PUBLISHER_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        locks
            .entry(db_path.to_path_buf())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// Bounds how long one retained row may hold the per-scope publisher lock while
/// awaiting the relay. `submit_signed_event_at_with_keys` first waits on the
/// process-wide admission gate (up to 300s on a 429) and then POSTs on the
/// app-wide `http_client`, whose builder configures only pool options —
/// reqwest leaves connect/read/total timeouts unset, so a relay that accepts
/// the connection and never finishes the response would otherwise pin the lock
/// forever. A healthy admission wait + POST + body parse completes far inside
/// this bound; a timeout takes the same `Err` path as a relay rejection, so the
/// row stays pending for the next 30s sweep and a timed-out tombstone keeps its
/// replacement deferred this pass. A live 300s admission gate therefore
/// surfaces as timeout-pending rather than a held lock — the correct durable
/// behavior, since the sweep retries.
const PUBLISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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
    let mut tags =
        vec![Tag::parse(["d", d_tag.as_str()]).map_err(|e| format!("invalid d-tag: {e}"))?];
    if record.shared {
        tags.push(Tag::parse(["shared", "true"]).map_err(|e| format!("invalid shared tag: {e}"))?);
    }

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
        shared: event_is_shared(event),
        source_team: None,
        source_team_persona_slug: Some(d_tag),
        catalog_source: None,
        team_catalog_source: None,
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
#[cfg(test)]
pub async fn flush_pending_events(
    db_path: &std::path::Path,
    state: &AppState,
) -> Result<u32, String> {
    let relay_url = crate::relay::relay_ws_url_with_override(state);
    let owner_keys = state.signing_keys()?;
    flush_pending_events_at(db_path, state, &relay_url, &owner_keys).await
}

/// Resolve and flush only the currently active `(relay, owner)` scope.
///
/// The scope snapshots its relay, owner keys, and database path together
/// before network work starts. Switching communities during the flush cannot
/// redirect rows from the old scope into the new relay.
pub async fn flush_active_pending_events(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<u32, String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    flush_pending_events_at(&scope.db_path, state, &scope.relay_url, &scope.owner_keys).await
}

pub fn active_pending_event(
    app: &tauri::AppHandle,
    state: &AppState,
    kind: u32,
    d_tag: &str,
) -> Result<bool, String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    let owner_pubkey = scope.owner_keys.public_key().to_hex();
    let conn = crate::managed_agents::retention::open_retention_db(&scope.db_path)?;
    Ok(
        crate::managed_agents::retention::get_retained_event(&conn, kind, &owner_pubkey, d_tag)?
            .is_some_and(|event| event.pending_sync),
    )
}

pub(crate) async fn flush_pending_events_at(
    db_path: &std::path::Path,
    state: &AppState,
    relay_url: &str,
    owner_keys: &nostr::Keys,
) -> Result<u32, String> {
    use crate::managed_agents::retention::{
        deferred_behind_failed_tombstone, get_pending_sync, get_retained_event, mark_synced,
        open_retention_db,
    };
    use nostr::JsonUtil;

    // Serialize the whole flush for THIS retention scope into a single
    // publisher. Concurrent callers exist (the 30s sweep, the team-share
    // toggle, managed-policy updates), and the re-read→POST await gap below
    // would otherwise let a second flush of the same scope publish a deletion
    // tombstone between this flush's row re-read and its POST, landing a purged
    // head after its tombstone. Held across snapshot, re-read, POST, and
    // mark_synced so the only interleavings are head-before-tombstone or
    // purged-row-skip. Keyed by the canonical scope db_path — never
    // process-wide — so a stalled relay in one community cannot block
    // publication in another (retention is scoped per `(relay, owner)`).
    let publisher_lock = flush_publisher_lock(db_path);
    let _publisher_guard = publisher_lock.lock().await;

    let owner_pubkey = owner_keys.public_key().to_hex();
    let relay_api_base = crate::relay::relay_http_base_url(relay_url);
    let pending = {
        let conn = open_retention_db(db_path)?;
        get_pending_sync(&conn)?
    }; // connection dropped before any .await

    let mut flushed = 0u32;
    let mut failed_tombstones: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for row in pending {
        if row.pubkey != owner_pubkey {
            continue;
        }
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

        // Relay ingest rejects any event whose `created_at` is more than
        // ±900s from server time (`crates/buzz-relay/src/handlers/ingest.rs`
        // MAX_TIMESTAMP_DRIFT_SECS). A kind:5 tombstone is signed strictly past
        // the head it retracts, so its retained `created_at` is the domination
        // floor `f`: any publish at `t >= f` still soft-deletes the head (NIP-09
        // only clears coordinate versions with `created_at <= t`). Reconcile the
        // two constraints at publish time so a byte-frozen future-dated
        // tombstone can never age out of the acceptance window and strand the
        // head live forever:
        //   f <= now         → re-date to `now` (dominates, in-window)
        //   now < f <= now+900 → publish at `f` (dominates, in-window)
        //   f > now+900       → no acceptable timestamp yet; leave pending and
        //                       block its replacement, converging as the wall
        //                       clock advances toward `f`.
        // A boundary publish the relay still rejects self-heals: the submit
        // error below re-queues it for the next sweep.
        const RELAY_ACCEPT_WINDOW_SECS: i64 = 900;
        let event = if current.kind == 5 {
            let now = nostr::Timestamp::now().as_secs() as i64;
            if current.created_at - now > RELAY_ACCEPT_WINDOW_SECS {
                // Its replacement must keep deferring behind the unpublished
                // tombstone so a re-created head is never wiped out of order.
                failed_tombstones.insert((current.pubkey.clone(), current.d_tag.clone()));
                continue;
            }
            redate_tombstone(&event, now.max(current.created_at), owner_keys)?
        } else if buzz_core_pkg::kind::is_identity_archive_request_kind(current.kind) {
            // NIP-IA requests are freshness-checked by the relay (±120s on
            // `created_at`), so a request retained while the relay was
            // unreachable would be permanently stale. Re-sign with a fresh
            // timestamp at publish time; kind, tags, and content are preserved,
            // and `mark_synced` below still compares against the retained row's
            // original `created_at`/`content`, which are untouched.
            resign_with_fresh_timestamp(&event, state)?
        } else {
            event
        };

        // Bound the relay await: the admission gate can wait up to 300s and the
        // shared http_client sets no request timeout, so a non-responding relay
        // would otherwise hold the per-scope publisher lock indefinitely. A
        // timeout is treated exactly like a relay rejection — the row stays
        // pending for the next sweep and a timed-out tombstone keeps its
        // replacement deferred this pass.
        let submit = tokio::time::timeout(
            PUBLISH_TIMEOUT,
            crate::relay::submit_signed_event_at_with_keys(
                &event,
                state,
                &relay_api_base,
                owner_keys,
            ),
        )
        .await;
        if !matches!(submit, Ok(Ok(_))) {
            if current.kind == 5 {
                failed_tombstones.insert((current.pubkey.clone(), current.d_tag.clone()));
            }
            continue; // relay unreachable, rejected, or timed out — stays pending
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

/// Re-sign a retained kind:5 tombstone at `created_at`, preserving its `a`-tag
/// coordinate and (empty) content.
///
/// The flush loop chooses `created_at` in `[floor, now+900]` so the deletion
/// both dominates the head it retracts (NIP-09 `created_at <=` soft-delete) and
/// clears the relay's ±900s ingest window. Signing at the original owner keys
/// keeps the event authored by the same identity that owns the coordinate; the
/// `mark_synced` compare-and-clear below still keys on the retained row's
/// untouched `created_at`/`content`, so a concurrent edit is never masked.
fn redate_tombstone(
    event: &nostr::Event,
    created_at: i64,
    owner_keys: &nostr::Keys,
) -> Result<nostr::Event, String> {
    nostr::EventBuilder::new(event.kind, event.content.clone())
        .tags(event.tags.iter().cloned())
        .custom_created_at(nostr::Timestamp::from(created_at as u64))
        .sign_with_keys(owner_keys)
        .map_err(|e| format!("failed to re-sign tombstone: {e}"))
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
/// create time for display/backward-compat purposes. For a linked instance,
/// spawn and deploy do NOT read these snapshotted fields — they resolve
/// model/provider/prompt live from the definition on every spawn (see
/// `effective_config::resolve_effective_config`), so a definition edit
/// propagates on the next restart without delete+respawn. The snapshot still
/// matters for `runtime` (materialized per B5, no live-read path yet) and for
/// `persona_source_version`, the drift basis the Agents menu compares against
/// the definition's current content hash.
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

/// Re-pin `record` to `persona`: build a snapshot via [`persona_snapshot`]
/// and mirror it onto the record — the definition quad
/// (`system_prompt`/`model`/`provider`/`runtime`), the env-override
/// self-heal, and the `persona_source_version` drift basis.
///
/// Definition-authoritative: blank definition model/provider produce `None`
/// on the record. The effective-config resolver falls through to the global
/// default at read time; stale materialized record bytes are never preserved.
///
/// This is the single apply used by every snapshot-apply site: the spawn
/// re-pin (`start_local_agent_with_preflight`), the launch backfill and
/// restore re-snapshot (`restore.rs`), and the prospective re-snapshot inside
/// `prospective_spawn_config_snapshot` — so a future `PersonaSnapshot` field
/// addition propagates to all of them at once.
///
/// Deliberately does NOT touch `updated_at`: persistence stamps are the
/// caller's concern, and the prospective snapshot (which applies this to a
/// clone) must stay pure.
pub fn apply_persona_snapshot(record: &mut ManagedAgentRecord, persona: &AgentDefinition) {
    let snapshot = persona_snapshot(persona);
    if let Some(prompt) = snapshot.system_prompt {
        record.system_prompt = Some(prompt);
    }
    record.model = snapshot.model;
    record.provider = snapshot.provider;
    record.runtime = snapshot.runtime;
    // Drop a stale create-time harness pin when the definition switches to a
    // different known runtime (builtin, static preset, or loaded custom). A pin
    // that names an unknown/custom command is always kept.
    //
    // Both sides are resolved through the canonical harness-identity resolver
    // (`canonical_harness_command`) which accepts either a runtime id OR a
    // command string — covering aliases (e.g. "claude-code-acp"), path prefixes
    // ("/usr/local/bin/goose"), and harnesses whose id ≠ command. The persona
    // runtime side is resolved via `command_for_runtime_id` (id-only input is
    // sufficient there since persona.runtime is always an authoritative id).
    //
    // Comparison is on canonical primary commands so "goose", "/usr/local/bin/goose",
    // and runtime id "goose" all represent the same harness; the stale pin is
    // dropped only when the canonical commands differ.
    if let Some(new_cmd) = persona
        .runtime
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .and_then(super::command_for_runtime_id)
    {
        if let Some(pin) = record
            .agent_command_override
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            // Resolve the pin via the canonical resolver (accepts id OR command).
            if let Some(pin_cmd) = super::canonical_harness_command(pin) {
                if pin_cmd != new_cmd {
                    // Known harness switched to a different known harness — drop stale pin.
                    record.agent_command_override = None;
                }
                // Same harness: keep the pin (e.g. explicit path override for same runtime).
            }
            // Custom/unknown pin: always keep.
        }
    }
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

/// Preview what `record` would look like immediately after the start/restore
/// paths re-pin it to its linked persona, without mutating `record` itself.
///
/// Every decision made ahead of the real re-pin — the relay-mesh preflight in
/// `start_local_agent_with_preflight`, the restart-badge snapshot in
/// `prospective_spawn_config_snapshot` — needs to reason about spawn-time
/// state, not
/// pre-snapshot bytes, so a persona edit that flips a field (e.g. `provider`
/// to/from relay-mesh) between saves is reflected in the decision instead of
/// the stale value the real [`apply_persona_snapshot`] is about to overwrite
/// anyway. Idempotent: applying it to an already-current record is a no-op,
/// so the spawn-time stamp and later recomputes agree when nothing changed.
///
/// Orphaned records (persona deleted) pass through unchanged: the caller's
/// own orphan handling — refusing to spawn, snapshotting as `(None, None, None)`
/// — runs on the real record downstream, not on this preview.
pub fn preview_prospective_persona_snapshot(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
) -> ManagedAgentRecord {
    let mut preview = record.clone();
    if let Some(persona_id) = preview.persona_id.clone() {
        if let Some(persona) = personas.iter().find(|p| p.id == persona_id) {
            apply_persona_snapshot(&mut preview, persona);
        }
    }
    preview
}
#[cfg(test)]
mod stale_pin_tests;
#[cfg(test)]
mod tests;
