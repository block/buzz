//! Retention-store enqueue helpers for owner-authored persona writes: retain
//! a pending 30175 on create/update, purge + tombstone on delete. The flush
//! loop (`flush_pending_events`) is the sole publisher.

use tauri::AppHandle;

use crate::app_state::AppState;
use crate::managed_agents::{
    retention::{RetainedEvent, RetentionScope},
    AgentDefinition,
};

pub(in crate::commands) struct PreparedPersonaPublication {
    pub scope: RetentionScope,
    pub event: nostr::Event,
    pub retained: RetainedEvent,
    pub persona: AgentDefinition,
}

pub(in crate::commands) struct UnsignedPersonaPublication {
    pub scope: RetentionScope,
    pub head: crate::managed_agents::persona_events::definition::PreparedPersonaHead,
}

pub(in crate::commands) type PersonaRetentionWork = Result<UnsignedPersonaPublication, String>;

/// Prepare under the store lock after the disk-authoritative save. The caller
/// must finish this work outside the lock; boot reconcile recovers failures.
pub(in crate::commands) fn retain_persona_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    persona: &AgentDefinition,
) -> PersonaRetentionWork {
    prepare_persona_publication(app, state, persona, None)
}

pub(in crate::commands) fn retain_persona_pending_at(
    scope: &RetentionScope,
    persona: &AgentDefinition,
) -> PersonaRetentionWork {
    prepare_persona_in_scope(scope.clone(), persona, None)
}

pub(super) fn prepare_persona_publication<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    persona: &AgentDefinition,
    shared_override: Option<bool>,
) -> PersonaRetentionWork {
    prepare_persona_in_scope(
        crate::managed_agents::retention::active_retention_scope(app, state)?,
        persona,
        shared_override,
    )
}

fn prepare_persona_in_scope(
    scope: RetentionScope,
    persona: &AgentDefinition,
    shared_override: Option<bool>,
) -> PersonaRetentionWork {
    let head = crate::managed_agents::persona_events::definition::prepare_persona_head(
        &scope.db_path,
        &scope.owner_signer(),
        persona,
        shared_override,
        true,
    )?;
    Ok(UnsignedPersonaPublication { scope, head })
}

pub(super) struct SignedPersonaPublication {
    scope: RetentionScope,
    head: crate::managed_agents::persona_events::definition::SignedPersonaHead,
}

pub(super) async fn sign_persona_publication(
    work: PersonaRetentionWork,
) -> Result<SignedPersonaPublication, String> {
    let work = work?;
    Ok(SignedPersonaPublication {
        scope: work.scope,
        head: work.head.sign().await?,
    })
}

impl SignedPersonaPublication {
    /// Caller holds the store lock. Commit before applying linked-record side
    /// effects so strict failure cannot leave those effects without their work.
    pub(super) fn commit(
        self,
        personas: &[AgentDefinition],
    ) -> Result<PreparedPersonaPublication, String> {
        let (event, retained, persona) = self.head.commit(personas)?;
        Ok(PreparedPersonaPublication {
            scope: self.scope,
            event,
            retained,
            persona,
        })
    }
}

pub(in crate::commands) async fn finish_persona_publication<R: tauri::Runtime>(
    app: &AppHandle<R>,
    work: PersonaRetentionWork,
) -> Result<PreparedPersonaPublication, String> {
    use tauri::Manager;
    let signed = sign_persona_publication(work).await?;
    let state = app.state::<AppState>();
    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    signed.commit(&crate::managed_agents::load_personas(app)?)
}

pub(in crate::commands) async fn finish_persona_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    work: PersonaRetentionWork,
) {
    if let Err(e) = finish_persona_publication(app, work).await {
        eprintln!("buzz-desktop: persona-retain: {e}");
    }
}

fn retained_persona_is_shared(row: Option<&RetainedEvent>) -> bool {
    use buzz_core_pkg::kind::event_is_shared;
    use nostr::JsonUtil;

    row.and_then(|retained| nostr::Event::from_json(&retained.raw_event).ok())
        .is_some_and(|event| event_is_shared(&event))
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
pub(super) fn project_active_persona_sharing<R: tauri::Runtime>(
    app: &AppHandle<R>,
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
            &scope.owner_signer().public_key().to_hex(),
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

#[cfg(test)]
pub(super) async fn prepare_persona_publication_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    persona: &AgentDefinition,
    shared_override: Option<bool>,
) -> Result<(nostr::Event, RetainedEvent, AgentDefinition), String> {
    crate::managed_agents::persona_events::definition::prepare_persona_head(
        db_path,
        &crate::active_user_signer::ActiveUserSigner::local(keys.clone()),
        persona,
        shared_override,
        true,
    )?
    .sign()
    .await?
    .commit(std::slice::from_ref(persona))
}

/// Prepare a persona tombstone while the caller holds the store lock. Signing
/// and the destructive commit are owned by the deletion caller.
pub(crate) fn tombstone_persona_at(
    db_path: &std::path::Path,
    signer: &crate::active_user_signer::ActiveUserSigner,
    d_tag: &str,
) -> Result<super::super::agents::deletion_witness::AgentTombstone, String> {
    super::super::agents::deletion_witness::prepare_definition_tombstone(
        db_path,
        signer,
        buzz_core_pkg::kind::KIND_PERSONA,
        d_tag,
    )
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
            session_policy: Default::default(),
            description: None,
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
            team_catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-07-27T00:00:00Z".to_string(),
            updated_at: "2026-07-27T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn share_state_and_pending_heads_are_scoped_by_relay_and_owner() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let community_a = scoped_retention_db_path(dir.path(), "wss://a.example", &owner);
        let community_b = scoped_retention_db_path(dir.path(), "wss://b.example", &owner);
        std::fs::create_dir_all(community_a.parent().unwrap()).unwrap();

        let (_, _, shared_in_a) =
            prepare_persona_publication_at(&community_a, &keys, &persona(), Some(true))
                .await
                .unwrap();
        assert!(shared_in_a.shared);

        let (_, _, unshared_in_b) =
            prepare_persona_publication_at(&community_b, &keys, &persona(), None)
                .await
                .unwrap();
        assert!(!unshared_in_b.shared);

        let mut edited = persona();
        edited.system_prompt = "Review the latest catalog.".to_string();
        let (_, _, edited_in_a) =
            prepare_persona_publication_at(&community_a, &keys, &edited, None)
                .await
                .unwrap();
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
    async fn shared_persona_scope(dir: &std::path::Path) -> (RetentionScope, Vec<AgentDefinition>) {
        let keys = nostr::Keys::generate();
        let db_path = scoped_retention_db_path(dir, "wss://a.example", &keys.public_key().to_hex());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        prepare_persona_publication_at(&db_path, &keys, &persona(), Some(true))
            .await
            .unwrap();
        (
            RetentionScope {
                db_path,
                relay_url: "wss://a.example".to_string(),
                signer: crate::active_user_signer::ActiveUserSigner::local(keys.clone()),
            },
            vec![persona()],
        )
    }

    #[tokio::test]
    async fn test_resolvable_scope_projects_the_retained_share_state() {
        let dir = tempfile::tempdir().unwrap();
        let (scope, mut personas) = shared_persona_scope(dir.path()).await;

        project_scoped_persona_sharing(Ok(scope), &mut personas);

        assert!(personas[0].shared);
    }

    #[tokio::test]
    async fn test_recovery_mode_identity_projects_unshared_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let (_scope, mut personas) = shared_persona_scope(dir.path()).await;
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

    #[tokio::test]
    async fn test_unopenable_retention_db_projects_unshared_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let mut personas = vec![persona()];
        personas[0].shared = true;

        project_scoped_persona_sharing(
            Ok(RetentionScope {
                // A directory cannot be opened as the retention database.
                db_path: dir.path().to_path_buf(),
                relay_url: "wss://a.example".to_string(),
                signer: crate::active_user_signer::ActiveUserSigner::local(keys.clone()),
            }),
            &mut personas,
        );

        assert!(!personas[0].shared);
    }

    #[tokio::test]
    async fn explicit_share_enqueue_failure_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let error = prepare_persona_publication_at(dir.path(), &keys, &persona(), Some(true))
            .await
            .expect_err("a directory cannot be opened as the retention database");
        assert!(error.contains("failed to open retention db"));
    }

    #[tokio::test]
    async fn shared_publication_rejects_invisible_definition_text() {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let db_path = dir.path().join("retention.sqlite3");
        let mut unsafe_persona = persona();
        unsafe_persona.system_prompt = "Review\u{200B} the catalog.".to_string();

        let error = prepare_persona_publication_at(&db_path, &keys, &unsafe_persona, Some(true))
            .await
            .expect_err("sharing must reject an invisible instruction character");

        assert!(error.contains("U+200B"));
    }

    /// Seed a retained 30175 persona head dated `created_at` seconds since
    /// epoch, then return the enqueued kind:5 tombstone after tombstoning.
    fn seed_persona_head(db_path: &std::path::Path, keys: &nostr::Keys, created_at: i64) {
        use crate::managed_agents::persona_events::build_persona_event;
        use nostr::JsonUtil;
        let mut shared = persona();
        shared.shared = true;
        let event = build_persona_event(&shared)
            .unwrap()
            .custom_created_at(nostr::Timestamp::from(created_at as u64))
            .sign_with_keys(keys)
            .unwrap();
        let conn = open_retention_db(db_path).unwrap();
        crate::managed_agents::retention::retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_PERSONA,
                pubkey: keys.public_key().to_hex(),
                d_tag: "catalog-reviewer".to_string(),
                content: event.content.to_string(),
                created_at,
                raw_event: event.as_json(),
                pending_sync: false,
            },
        )
        .unwrap();
    }

    fn enqueued_persona_tombstone(db_path: &std::path::Path) -> RetainedEvent {
        use crate::managed_agents::retention::get_pending_sync;
        let conn = open_retention_db(db_path).unwrap();
        get_pending_sync(&conn)
            .unwrap()
            .into_iter()
            .find(|row| row.kind == 5)
            .expect("a kind:5 persona tombstone is enqueued")
    }

    #[tokio::test]
    async fn persona_tombstone_created_at_strictly_dominates_a_future_dated_head() {
        // The retained 30175 head may be future-dated (monotonic_created_at
        // bumps a same-second re-publish past the prior head). The relay only
        // soft-deletes coordinate versions with created_at <= the tombstone's,
        // and the flush loop never re-reads the (purged) head — so a kind:5
        // signed at wall-clock `now` would leave the persona live forever once
        // its local retry witness is gone.
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = scoped_retention_db_path(dir.path(), "wss://a.example", &owner);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        let future = nostr::Timestamp::now().as_secs() as i64 + 86_400;
        seed_persona_head(&db_path, &keys, future);

        tombstone_persona_at(
            &db_path,
            &crate::active_user_signer::ActiveUserSigner::local(keys.clone()),
            "catalog-reviewer",
        )
        .unwrap()
        .sign(&crate::active_user_signer::ActiveUserSigner::local(
            keys.clone(),
        ))
        .await
        .commit()
        .unwrap();

        let tombstone = enqueued_persona_tombstone(&db_path);
        assert!(
            tombstone.created_at > future,
            "tombstone created_at ({}) must strictly dominate the future-dated head ({future})",
            tombstone.created_at
        );
        // The head row itself is purged in the same transaction.
        let conn = open_retention_db(&db_path).unwrap();
        assert!(
            get_retained_event(&conn, KIND_PERSONA, &owner, "catalog-reviewer")
                .unwrap()
                .is_none(),
            "the 30175 head is purged so no stale edit can republish it"
        );
    }

    #[tokio::test]
    async fn persona_tombstone_rolls_back_head_purge_when_enqueue_fails() {
        // The head purge and kind:5 enqueue run in one `BEGIN IMMEDIATE`
        // transaction. A `BEFORE INSERT` trigger blocks the enqueue (which
        // follows the head DELETE); the whole transaction must roll back so the
        // 30175 head survives with its local retry witness intact.
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = scoped_retention_db_path(dir.path(), "wss://a.example", &owner);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        let future = nostr::Timestamp::now().as_secs() as i64 + 86_400;
        seed_persona_head(&db_path, &keys, future);

        let conn = open_retention_db(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER block_all_inserts BEFORE INSERT ON persona_events
             BEGIN
                 SELECT RAISE(ABORT, 'insert blocked by test trigger');
             END;",
        )
        .unwrap();
        drop(conn);

        let err = tombstone_persona_at(
            &db_path,
            &crate::active_user_signer::ActiveUserSigner::local(keys.clone()),
            "catalog-reviewer",
        )
        .unwrap()
        .sign(&crate::active_user_signer::ActiveUserSigner::local(
            keys.clone(),
        ))
        .await
        .commit()
        .expect_err("tombstone with INSERT trigger must fail");
        assert!(
            err.contains("insert blocked by test trigger") || err.contains("blocked"),
            "error must name the trigger cause; got: {err}"
        );

        let conn = open_retention_db(&db_path).unwrap();
        assert!(
            get_retained_event(&conn, KIND_PERSONA, &owner, "catalog-reviewer")
                .unwrap()
                .is_some(),
            "the 30175 head must survive when the tombstone enqueue fails"
        );
    }
}
