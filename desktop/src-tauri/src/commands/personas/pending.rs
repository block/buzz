//! Retention-store enqueue helpers for owner-authored persona writes: retain
//! a pending 30175 on create/update, purge + tombstone on delete. The flush
//! loop (`flush_pending_events`) is the sole publisher.

use tauri::AppHandle;

use crate::app_state::AppState;
use crate::managed_agents::{
    retention::{RetainedEvent, RetentionScope},
    AgentDefinition,
};

pub(super) struct PreparedPersonaPublication {
    pub scope: RetentionScope,
    pub event: nostr::Event,
    pub retained: RetainedEvent,
    pub persona: AgentDefinition,
}

/// Retain a freshly authored persona event in the local store, flagged for
/// relay sync. Called inside a command's `managed_agents_store_lock`-held body
/// after `save_personas`; the background flush loop publishes it out-of-band.
///
/// The event is signed with the owner keys at call time, so its `created_at`
/// is `now` — newer than any prior retained row, clearing the upsert's
/// newer-or-equal guard. `pending_sync = 1` enqueues it for the flush loop,
/// which is the sole publisher. Best-effort: a failure here is logged and
/// swallowed so a retention hiccup never blocks the disk-authoritative write.
/// The explicit catalog toggle uses [`prepare_persona_publication`] directly
/// so its durable enqueue failure reaches the UI.
///
/// Unlike `retain_managed_agent_pending`, this has no projection-equality
/// short-circuit: personas have no start/stop runtime churn, so a republish
/// only happens on a genuine create/update/delete/share user edit
/// (`set_persona_active` does not retain, so the local-only `is_active` toggle
/// never republishes, while `set_persona_shared` must retain because the tag is
/// relay-authoritative). A byte-identical user-save republish is harmlessly
/// NIP-33-replaced. The guard is intentionally omitted.
pub(in crate::commands) fn retain_persona_pending(
    app: &AppHandle,
    state: &AppState,
    persona: &AgentDefinition,
) {
    if let Err(e) = prepare_persona_publication(app, state, persona, None) {
        eprintln!("buzz-desktop: persona-retain: {e}");
    }
}

/// Build, sign, and durably retain a persona event in the active relay+owner
/// scope.
///
/// Ordinary definition writes pass `None` and preserve the scoped head's
/// exact share tag. The explicit share toggle passes `Some(shared)`. Returning
/// the retained event lets that command immediately await relay acceptance
/// without rebuilding or re-signing a different NIP-33 head.
pub(super) fn prepare_persona_publication(
    app: &AppHandle,
    state: &AppState,
    persona: &AgentDefinition,
    shared_override: Option<bool>,
) -> Result<PreparedPersonaPublication, String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    let (event, retained, persona) = prepare_persona_publication_at(
        &scope.db_path,
        &scope.owner_keys,
        persona,
        shared_override,
    )?;
    Ok(PreparedPersonaPublication {
        scope,
        event,
        retained,
        persona,
    })
}

fn retained_persona_is_shared(row: Option<&RetainedEvent>) -> bool {
    use buzz_core_pkg::kind::persona_event_is_shared;
    use nostr::JsonUtil;

    row.and_then(|retained| nostr::Event::from_json(&retained.raw_event).ok())
        .is_some_and(|event| persona_event_is_shared(&event))
}

/// Project each persona's catalog visibility from the active relay+owner
/// scope's retained head.
///
/// Infallible by design. The scope needs `signing_keys()`, which fails for the
/// whole process whenever the identity is lost or the keyring is locked, and a
/// propagated error there would break listing, creating, and updating EVERY
/// agent. Share state is a view projection, so an unresolvable scope degrades
/// to "not shared" — the safe direction: it can under-report visibility but can
/// never present an unshared persona as published. The durable share state
/// lives in the retention head, so nothing is lost: the true value reappears
/// once the identity is signable again.
pub(super) fn project_active_persona_sharing(
    app: &AppHandle,
    state: &AppState,
    personas: &mut [AgentDefinition],
) {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state);
    project_scoped_persona_sharing(scope, personas);
}

fn project_scoped_persona_sharing(
    scope: Result<RetentionScope, String>,
    personas: &mut [AgentDefinition],
) {
    let projected = scope.and_then(|scope| {
        project_persona_sharing_at(
            &scope.db_path,
            &scope.owner_keys.public_key().to_hex(),
            personas,
        )
    });
    if let Err(error) = projected {
        eprintln!("buzz-desktop: persona-share-projection unavailable, reporting every agent as unshared: {error}");
        for persona in personas {
            persona.shared = false;
        }
    }
}

fn project_persona_sharing_at(
    db_path: &std::path::Path,
    owner_pubkey: &str,
    personas: &mut [AgentDefinition],
) -> Result<(), String> {
    use crate::managed_agents::{
        persona_events::persona_d_tag,
        retention::{get_retained_event, open_retention_db},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;

    let conn = open_retention_db(db_path)?;
    for persona in personas {
        if persona.is_builtin {
            persona.shared = false;
            continue;
        }
        let retained =
            get_retained_event(&conn, KIND_PERSONA, owner_pubkey, &persona_d_tag(persona))?;
        persona.shared = retained_persona_is_shared(retained.as_ref());
    }
    Ok(())
}

pub(super) fn prepare_persona_publication_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    persona: &AgentDefinition,
    shared_override: Option<bool>,
) -> Result<(nostr::Event, RetainedEvent, AgentDefinition), String> {
    use crate::managed_agents::{
        persona_events::{build_persona_event, monotonic_created_at, persona_d_tag},
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let d_tag = persona_d_tag(persona);
    let pubkey = keys.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    let existing = get_retained_event(&conn, KIND_PERSONA, &pubkey, &d_tag)?;
    let mut scoped_persona = persona.clone();
    scoped_persona.shared =
        shared_override.unwrap_or_else(|| retained_persona_is_shared(existing.as_ref()));
    let event = build_persona_event(&scoped_persona)?
        .custom_created_at(monotonic_created_at(
            existing.as_ref().map(|row| row.created_at),
        ))
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign persona event: {e}"))?;
    let retained = RetainedEvent {
        kind: KIND_PERSONA,
        pubkey,
        d_tag,
        content: event.content.to_string(),
        created_at: event.created_at.as_secs() as i64,
        raw_event: event.as_json(),
        pending_sync: true,
    };
    retain_event(&conn, &retained)?;
    Ok((event, retained, scoped_persona))
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
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let pubkey = scope.owner_keys.public_key().to_hex();
        let event = build_persona_delete(d_tag, &pubkey)?
            .sign_with_keys(&scope.owner_keys)
            .map_err(|e| format!("failed to sign persona tombstone: {e}"))?;
        let conn = open_retention_db(&scope.db_path)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, scoped_retention_db_path,
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use std::collections::BTreeMap;

    fn persona() -> AgentDefinition {
        AgentDefinition {
            id: "catalog-reviewer".to_string(),
            display_name: "Catalog Reviewer".to_string(),
            avatar_url: None,
            system_prompt: "Review the catalog.".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-07-27T00:00:00Z".to_string(),
            updated_at: "2026-07-27T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn share_state_and_pending_heads_are_scoped_by_relay_and_owner() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let community_a = scoped_retention_db_path(dir.path(), "wss://a.example", &owner);
        let community_b = scoped_retention_db_path(dir.path(), "wss://b.example", &owner);
        std::fs::create_dir_all(community_a.parent().unwrap()).unwrap();

        let (_, _, shared_in_a) =
            prepare_persona_publication_at(&community_a, &keys, &persona(), Some(true)).unwrap();
        assert!(shared_in_a.shared);

        let (_, _, unshared_in_b) =
            prepare_persona_publication_at(&community_b, &keys, &persona(), None).unwrap();
        assert!(!unshared_in_b.shared);

        let mut edited = persona();
        edited.system_prompt = "Review the latest catalog.".to_string();
        let (_, _, edited_in_a) =
            prepare_persona_publication_at(&community_a, &keys, &edited, None).unwrap();
        assert!(
            edited_in_a.shared,
            "ordinary edits preserve only the active scope's share choice"
        );

        let conn_a = open_retention_db(&community_a).unwrap();
        let conn_b = open_retention_db(&community_b).unwrap();
        assert!(retained_persona_is_shared(
            get_retained_event(&conn_a, KIND_PERSONA, &owner, "catalog-reviewer")
                .unwrap()
                .as_ref()
        ));
        assert!(!retained_persona_is_shared(
            get_retained_event(&conn_b, KIND_PERSONA, &owner, "catalog-reviewer")
                .unwrap()
                .as_ref()
        ));
    }

    /// A `shared = true` persona plus the scope that says so.
    fn shared_persona_scope(dir: &std::path::Path) -> (RetentionScope, Vec<AgentDefinition>) {
        let keys = nostr::Keys::generate();
        let db_path = scoped_retention_db_path(dir, "wss://a.example", &keys.public_key().to_hex());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true)).unwrap();
        (
            RetentionScope {
                db_path,
                relay_url: "wss://a.example".to_string(),
                owner_keys: keys,
            },
            vec![persona()],
        )
    }

    #[test]
    fn test_resolvable_scope_projects_the_retained_share_state() {
        let dir = tempfile::tempdir().unwrap();
        let (scope, mut personas) = shared_persona_scope(dir.path());

        project_scoped_persona_sharing(Ok(scope), &mut personas);

        assert!(personas[0].shared);
    }

    #[test]
    fn test_recovery_mode_identity_projects_unshared_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let (_scope, mut personas) = shared_persona_scope(dir.path());
        personas[0].shared = true;

        // The real recovery-mode failure: `active_retention_scope` cannot
        // resolve a scope without signing keys, which is exactly what
        // `identity_lost` / `keyring_locked` withhold.
        let state = crate::app_state::build_app_state();
        state
            .identity_lost
            .store(true, std::sync::atomic::Ordering::Release);
        let error = state
            .signing_keys()
            .expect_err("recovery mode must withhold signing keys");

        project_scoped_persona_sharing(Err(error), &mut personas);

        assert!(
            !personas[0].shared,
            "an unresolvable scope degrades to unshared so list/create/update keep working"
        );
    }

    #[test]
    fn test_unopenable_retention_db_projects_unshared_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let mut personas = vec![persona()];
        personas[0].shared = true;

        project_scoped_persona_sharing(
            Ok(RetentionScope {
                // A directory cannot be opened as the retention database.
                db_path: dir.path().to_path_buf(),
                relay_url: "wss://a.example".to_string(),
                owner_keys: keys,
            }),
            &mut personas,
        );

        assert!(!personas[0].shared);
    }

    #[test]
    fn explicit_share_enqueue_failure_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let error = prepare_persona_publication_at(dir.path(), &keys, &persona(), Some(true))
            .expect_err("a directory cannot be opened as the retention database");
        assert!(error.contains("failed to open retention db"));
    }
}
