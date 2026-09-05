//! Boot-time disk↔relay reconcile for managed-agent (kind:30177) events.
//!
//! `run_event_sync` already reconciles personas (30175) and teams (30176)
//! into the retention store at boot; managed agents were the missing leg —
//! their events were enqueued only on the interactive save path
//! (`retain_managed_agent_pending`), so a record edited on disk between
//! launches, or a save whose publish was missed, silently diverged from the
//! relay. This module mirrors `migrate_personas_in_dir`: per-coordinate
//! content diff, monotonic `created_at` bump, retain with `pending_sync = 1`
//! for the existing flush loop.
//!
//! Best-effort contract (decided in #centralize-personas-and-agents):
//! - No file watcher — hand edits are picked up at next boot only.
//! - No deletion reconcile — a record absent from `managed-agents.json` is
//!   left untouched in retention; a truncated or partial file must never
//!   trigger tombstones.
//! - A malformed store fails loudly: the broken file is preserved as
//!   `managed-agents.json.invalid` (see [`super::storage::backup_invalid_store`])
//!   and an error is returned, never silently skipped.

use std::path::Path;

use super::{
    agent_events::build_agent_event,
    persona_events::monotonic_created_at,
    retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    ManagedAgentRecord,
};
use buzz_core_pkg::{
    kind::{KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT},
    private_managed_agent::{self, Payload, PrivateConfig, PrivateIdentity},
};
use nostr::JsonUtil;
use std::collections::BTreeMap;

/// Reconcile `managed-agents.json` into kind:30177 events in the retention
/// store. Boot-time entry point, called from `event_sync::run_event_sync`
/// after the persona and team legs.
pub(crate) fn reconcile_agents_to_events(
    app: &tauri::AppHandle,
    keys: &nostr::Keys,
    db_path: &Path,
) {
    let Ok(base_dir) = super::managed_agents_base_dir(app) else {
        return;
    };

    match reconcile_agents_in_dir_at(&base_dir, keys, db_path) {
        Ok(0) => {}
        Ok(reconciled) => {
            eprintln!(
                "buzz-desktop: agent-event-reconcile: {reconciled} agents reconciled to retention"
            );
        }
        Err(e) => {
            eprintln!("buzz-desktop: agent-event-reconcile: {e}");
        }
    }
}

/// Core reconcile logic, decoupled from the Tauri `AppHandle` for testing.
///
/// Reads `managed-agents.json` and hydrates keys from the keyring: the 30177
/// projection ([`super::agent_events::agent_event_content`]) is the opt-IN
/// no-secrets allowlist and needs no keys, but the 30179 private-config
/// projection carries the agent nsec, which is keyring-resident on a default
/// build. For each record it compares the freshly built event's content against
/// the retained row at `(30177, owner, agent_pubkey)` and re-retains (marking
/// `pending_sync = 1`) only when the row is absent or its content differs — an
/// unchanged agent never churns `pending_sync`.
///
/// Returns the number of agents (re)written to the retention store.
#[cfg(test)]
pub(crate) fn reconcile_agents_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    // No key store: unit tests must never touch the live OS keyring (a macOS
    // Keychain ACL prompt blocks a headless test binary forever). Tests that
    // exercise key hydration inject a fake via
    // [`reconcile_agents_in_dir_with`].
    reconcile_agents_in_dir_with(
        base_dir,
        keys,
        &base_dir.join("retention.db"),
        None::<&crate::secret_store::SecretStore>,
    )
}

/// Production entry: hydrates keys from the real agent secret store (`None`
/// on builds without a keyring backend, where keys stay inline in the JSON).
fn reconcile_agents_in_dir_at(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    reconcile_agents_in_dir_with(
        base_dir,
        keys,
        db_path,
        super::storage::agent_secret_store(),
    )
}

/// Testable core, generic over the [`super::storage::KeyStore`] seam so unit
/// tests never reach the live OS keyring.
fn reconcile_agents_in_dir_with(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
    store: Option<&impl super::storage::KeyStore>,
) -> Result<u32, String> {
    let store_path = base_dir.join("managed-agents.json");
    if !store_path.exists() {
        return Ok(0);
    }

    let content = std::fs::read_to_string(&store_path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;

    let mut records: Vec<ManagedAgentRecord> = serde_json::from_str(&content).map_err(|e| {
        super::storage::backup_invalid_store(&store_path);
        format!("failed to parse managed-agents.json (preserved as .invalid): {e}")
    })?;

    if records.is_empty() {
        return Ok(0);
    }

    // The 30179 private-config projection carries the agent nsec, which on a
    // default `system-keyring` build lives in the keyring and NOT in the JSON.
    // Without this, `retain_private_agent_record`'s empty-nsec skip fires for
    // every untouched agent and boot reconcile publishes zero 30179s.
    if let Some(store) = store {
        super::storage::hydrate_keys_with(store, &mut records);
    }

    let conn =
        open_retention_db(db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

    let mut reconciled = 0u32;

    for record in &records {
        // A record without a pubkey has no event coordinate yet (key-less
        // agents mint keys on first start) — nothing to reconcile.
        if record.pubkey.is_empty() {
            continue;
        }

        // One unpublishable record must not strand every record after it in
        // file order: log it and keep walking.
        match retain_agent_record_at_boot(&conn, keys, record) {
            Ok(true) => reconciled += 1,
            Ok(false) => {}
            Err(e) => eprintln!("buzz-desktop: agent-event-reconcile: {e}"),
        }
    }

    Ok(reconciled)
}

/// Boot-only variant of [`retain_agent_record`]: reconciles the kind:30177
/// identity record exactly as the interactive paths do, but publishes the
/// kind:30179 private config **only when no retained head exists**.
///
/// Boot reads `managed-agents.json` raw — there is no overlay to resolve
/// against, because `hydrate_private_config_overlay` runs after this leg and
/// depends on the very rows written here. On a device that FOLLOWS another
/// device's config, disk is stale by construction (inbound 30179 updates the
/// overlay and retention, never the JSON), so rebuilding the 30179 projection
/// from disk republishes every stale field over a newer head as an audit-clean
/// gen+1 successor, and `monotonic_created_at` makes it win LWW. That fires at
/// launch, unprompted, and re-arms on every new head the follower receives.
///
/// Restricting boot to the head-absent case keeps the requirement boot exists
/// to serve — an agent whose nsec lives in the keyring must get its FIRST
/// 30179 published — while leaving an existing head to the interactive edit
/// paths, which resolve the overlay before retaining and so author from
/// relay-fresh state. Every 30177 (no-secrets projection) behaves exactly as
/// before: the upgrade republish waves run on that kind, not this one.
fn retain_agent_record_at_boot(
    conn: &rusqlite::Connection,
    keys: &nostr::Keys,
    record: &ManagedAgentRecord,
) -> Result<bool, String> {
    let transaction = conn
        .unchecked_transaction()
        .map_err(|error| format!("failed to begin agent retention transaction: {error}"))?;
    let public_changed = retain_public_agent_record(&transaction, keys, record)?;
    let private_head = get_retained_event(
        &transaction,
        KIND_PRIVATE_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &record.pubkey,
    )?;
    // A public-only recreation preserves its key/lifecycle, not deleted private
    // settings. Absence after deletion is not permission to mint stale disk config.
    let owner = keys.public_key().to_hex();
    let deleted_before = [KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT]
        .into_iter()
        .map(|kind| {
            get_retained_event(
                &transaction,
                buzz_core_pkg::kind::KIND_DELETION,
                &owner,
                &super::retention::tombstone_retention_d_tag(kind, &record.pubkey),
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(Option::is_some);
    let private_changed = if private_head.is_some() || deleted_before {
        false
    } else {
        retain_private_agent_record(&transaction, keys, record)?
    };
    transaction
        .commit()
        .map_err(|error| format!("failed to commit agent retention transaction: {error}"))?;
    Ok(public_changed || private_changed)
}

/// Retain `record`'s kind:30177 identity record, marking it `pending_sync`
/// for the flush loop, when its projection differs from the retained head.
/// Returns `Ok(true)` when a row was (re)written and `Ok(false)` when the
/// retained content already matches (a true no-op — no `pending_sync` churn).
///
/// This is the single content-diff + monotonic-bump engine shared by the
/// boot-time reconcile above and the interactive edit paths
/// (`retain_managed_agent_pending`, persona-rename propagation). Every
/// mutation of an agent's published identity must go through it so the
/// retained record can never silently drift from `managed-agents.json`.
pub(crate) fn retain_agent_record(
    conn: &rusqlite::Connection,
    keys: &nostr::Keys,
    record: &ManagedAgentRecord,
) -> Result<bool, String> {
    let transaction = conn
        .unchecked_transaction()
        .map_err(|error| format!("failed to begin agent retention transaction: {error}"))?;
    let public_changed = retain_public_agent_record(&transaction, keys, record)?;
    let private_changed = retain_private_agent_record(&transaction, keys, record)?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit agent retention transaction: {error}"))?;
    Ok(public_changed || private_changed)
}

fn retain_public_agent_record(
    conn: &rusqlite::Connection,
    keys: &nostr::Keys,
    record: &ManagedAgentRecord,
) -> Result<bool, String> {
    let owner_pubkey = keys.public_key().to_hex();
    let existing = get_retained_event(conn, KIND_MANAGED_AGENT, &owner_pubkey, &record.pubkey)?;

    // Build the event first and compare ITS content, so the comparison and
    // the retained row share one serialization of the projection (mirrors
    // `migrate_personas_in_dir`). Serializing the projection independently
    // here would silently diverge if `build_agent_event` ever changed how
    // it serializes — republishing every agent every boot. Content is
    // timestamp-independent, so the monotonic bump below never forces a
    // spurious republish; an unchanged agent is still a true no-op.
    let event = build_agent_event(record)?
        .custom_created_at(monotonic_created_at(
            existing.as_ref().map(|row| row.created_at),
        ))
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event for '{}': {e}", record.name))?;

    let content = event.content.clone();
    if existing.as_ref().is_some_and(|row| row.content == content) {
        return Ok(false);
    }

    retain_event(
        conn,
        &RetainedEvent {
            kind: KIND_MANAGED_AGENT,
            pubkey: owner_pubkey,
            d_tag: record.pubkey.clone(),
            content,
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        },
    )
    .map_err(|e| format!("failed to retain '{}': {e}", record.name))?;
    Ok(true)
}

fn retain_private_agent_record(
    conn: &rusqlite::Connection,
    keys: &nostr::Keys,
    record: &ManagedAgentRecord,
) -> Result<bool, String> {
    if record.private_key_nsec.is_empty() {
        return Ok(false);
    }

    let owner_pubkey = keys.public_key().to_hex();
    let existing = get_retained_event(
        conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_pubkey,
        &record.pubkey,
    )?;
    let previous_event = existing
        .as_ref()
        .and_then(|row| nostr::Event::from_json(&row.raw_event).ok());
    let generation = previous_event
        .as_ref()
        .and_then(event_generation)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| format!("private config generation overflow for '{}'", record.name))?;
    let previous_event_id = previous_event.as_ref().map(|event| event.id.to_hex());
    let created_at = monotonic_created_at(existing.as_ref().map(|row| row.created_at));
    let mut payload =
        private_payload_from_record(record, &owner_pubkey, generation, previous_event_id)?;

    // Preserve fields authored by a newer client. This writer owns the known
    // typed fields only; flatten/extension data must survive an older Desktop
    // editing one known value.
    let existing_payload = previous_event.as_ref().and_then(|existing_event| {
        private_managed_agent::validate_and_decrypt(existing_event, keys)
            .ok()
            .map(|(_, payload)| payload)
    });
    if let Some(existing_payload) = &existing_payload {
        payload.extensions.clone_from(&existing_payload.extensions);
        payload.extra.clone_from(&existing_payload.extra);
        payload
            .config
            .extra
            .clone_from(&existing_payload.config.extra);
    }

    // NIP-44 encryption is randomized, so compare the validated plaintext
    // payload rather than ciphertext. Metadata derived from the retained head
    // changes only after a meaningful config mutation.
    if existing_payload
        .as_ref()
        .is_some_and(|existing| private_payload_body_eq(existing, &payload))
    {
        return Ok(false);
    }

    let event = private_managed_agent::build_event(keys, &payload, created_at.as_secs())
        .map_err(|e| format!("failed to build private config for '{}': {e}", record.name))?;

    retain_event(
        conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_pubkey,
            d_tag: record.pubkey.clone(),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        },
    )
    .map_err(|e| format!("failed to retain private config for '{}': {e}", record.name))?;
    Ok(true)
}

fn event_generation(event: &nostr::Event) -> Option<u64> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some("g"))
            .then(|| values.get(1)?.parse().ok())
            .flatten()
    })
}

fn private_payload_from_record(
    record: &ManagedAgentRecord,
    owner_pubkey: &str,
    generation: u64,
    previous_event_id: Option<String>,
) -> Result<Payload, String> {
    let backend = serde_json::to_value(&record.backend)
        .map_err(|e| format!("failed to serialize backend for '{}': {e}", record.name))?;
    let relay_mesh = record
        .relay_mesh
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| format!("failed to serialize relay mesh for '{}': {e}", record.name))?;

    Ok(Payload {
        format: private_managed_agent::FORMAT.to_string(),
        version: private_managed_agent::VERSION,
        agent_pubkey: record.pubkey.clone(),
        owner_pubkey: owner_pubkey.to_string(),
        generation,
        previous_event_id,
        updated_at: record.updated_at.clone(),
        identity: PrivateIdentity {
            private_key_nsec: record.private_key_nsec.clone(),
            auth_tag: record.auth_tag.clone(),
        },
        config: PrivateConfig {
            relay_url: record.relay_url.clone(),
            name: record.name.clone(),
            persona_id: record.persona_id.clone(),
            runtime: record.runtime.clone(),
            model: record.model.clone(),
            provider: record.provider.clone(),
            system_prompt: record.system_prompt.clone(),
            parallelism: Some(record.parallelism),
            respond_to: Some(record.respond_to.as_str().to_string()),
            respond_to_allowlist: record.respond_to_allowlist.clone(),
            agent_command_override: record.agent_command_override.clone(),
            agent_args: record.agent_args.clone(),
            idle_timeout_seconds: record.idle_timeout_seconds,
            max_turn_duration_seconds: record.max_turn_duration_seconds,
            env_vars: record.env_vars.clone(),
            backend,
            backend_agent_id: record.backend_agent_id.clone(),
            team_id: record.team_id.clone(),
            persona_name_in_team: record.persona_name_in_team.clone(),
            relay_mesh,
            effort_level: record.effort_level.clone(),
            extra: serde_json::Map::new(),
        },
        extensions: BTreeMap::new(),
        extra: serde_json::Map::new(),
    })
}

fn private_payload_body_eq(left: &Payload, right: &Payload) -> bool {
    left.agent_pubkey == right.agent_pubkey
        && left.owner_pubkey == right.owner_pubkey
        && left.identity == right.identity
        && left.config == right.config
        && left.extensions == right.extensions
        && left.extra == right.extra
}

#[cfg(test)]
mod tests;
