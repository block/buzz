//! Retention-store enqueue helpers for owner-authored persona writes: retain
//! a pending 30175 on create/update, purge + tombstone on delete. The flush
//! loop (`flush_pending_events`) is the sole publisher.

use tauri::AppHandle;

use crate::app_state::AppState;
use crate::managed_agents::AgentDefinition;

/// Retain a freshly authored persona event in the local store, flagged for
/// relay sync. Called inside a command's `managed_agents_store_lock`-held body
/// after `save_personas`; the background flush loop publishes it out-of-band.
///
/// The event is signed with the owner keys at call time, so its `created_at`
/// is `now` — newer than any prior retained row, clearing the upsert's
/// newer-or-equal guard. `pending_sync = 1` enqueues it for the flush loop,
/// which is the sole publisher. Best-effort: a failure here is logged and
/// swallowed so a retention hiccup never blocks the disk-authoritative write.
///
/// Unlike `retain_managed_agent_pending`, this has no projection-equality
/// short-circuit: personas have no start/stop runtime churn, so a republish
/// only happens on a genuine create/update/delete user edit (`set_persona_active`
/// does not retain, so the local-only `is_active` toggle never republishes, and
/// a byte-identical user-save republish is harmlessly NIP-33-replaced). The
/// guard is intentionally omitted.
pub(in crate::commands) fn retain_persona_pending(
    app: &AppHandle,
    state: &AppState,
    persona: &AgentDefinition,
) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        persona_events::{build_persona_event, monotonic_created_at, persona_d_tag},
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let d_tag = persona_d_tag(persona);
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            // Monotonic created_at: read the retained head for this coordinate
            // and bump past it (NIP-AP step 3) so a same-second edit supersedes.
            let prior =
                get_retained_event(&conn, KIND_PERSONA, &keys.public_key().to_hex(), &d_tag)?
                    .map(|row| row.created_at);
            let event = build_persona_event(persona)?
                .custom_created_at(monotonic_created_at(prior))
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign persona event: {e}"))?;
            (keys.public_key().to_hex(), event)
        };
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_PERSONA,
                pubkey,
                d_tag,
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: persona-retain: {e}");
    }
}

/// Purge a deleted persona's pending row and enqueue a NIP-09 tombstone, both
/// inside the `managed_agents_store_lock`-held delete body.
///
/// PURGE IN: `delete_retained_event` removes the persona's `(30175, pubkey,
/// d_tag)` row. Running it under the same lock that serializes `retain_event`
/// closes the same-second resurrect race — a concurrent edit can't re-insert a
/// pending persona row after the tombstone is queued.
///
/// PUBLISH OUT: the kind:5 tombstone is retained at its own coordinate `(5,
/// pubkey, d_tag)` (distinct from the purged persona row) with `pending_sync =
/// 1`; the flush loop publishes it. Best-effort: a failure is logged and
/// swallowed so a retention hiccup never blocks the disk-authoritative delete.
pub(in crate::commands) fn tombstone_persona_pending(
    app: &AppHandle,
    state: &AppState,
    d_tag: &str,
) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        persona_events::build_persona_delete,
        retention::{
            delete_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
            RetainedEvent,
        },
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let result = (|| -> Result<(), String> {
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            let pubkey = keys.public_key().to_hex();
            let event = build_persona_delete(d_tag, &pubkey)?
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign persona tombstone: {e}"))?;
            (pubkey, event)
        };
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        // Purge the persona row first so an unpublished edit can never resurrect
        // it after the tombstone publishes.
        delete_retained_event(&conn, KIND_PERSONA, &pubkey, d_tag)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey,
                // Key by the target coordinate so cross-kind d-tag tombstones
                // occupy distinct rows (F2c).
                d_tag: tombstone_retention_d_tag(KIND_PERSONA, d_tag),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: persona-tombstone: {e}");
    }
}
