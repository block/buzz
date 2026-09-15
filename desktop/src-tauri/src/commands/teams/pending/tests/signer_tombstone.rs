//! Suspended-signer tests of the production PREPARE + tombstone_team_catalog_coordinate
//! path, using real disk stores, AppState store lock, and SQLite transactions.
use super::*;
use crate::active_user_signer::tests::ControlledSigner;
use crate::managed_agents::{load_personas, load_teams, save_personas, save_teams};
use nostr::{Event, Keys, Tag, Timestamp};
use std::{ffi::OsString, sync::Arc};
use tauri::Manager;

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    scope: RetentionScope,
    keys: Keys,
    base: PathBuf,
    old_home: Option<OsString>,
    old_xdg: Option<OsString>,
    _temp: tempfile::TempDir,
    // Serializes only test process environment, NOT a production store lock.
    _environment: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(keys: &Keys) -> Self {
        let environment = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        let state = crate::app_state::build_app_state();
        state.replace_local_identity_keys(keys.clone()).unwrap();
        *state.relay_url_override.lock().unwrap() = Some("wss://tombstone-original.example".into());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let scope = crate::managed_agents::retention::active_retention_scope(
            app.handle(),
            &app.state::<AppState>(),
        )
        .unwrap();
        let base = crate::managed_agents::managed_agents_base_dir(app.handle()).unwrap();
        // Materialize builtins before capturing any decision inputs.
        load_teams(app.handle()).unwrap();
        load_personas(app.handle()).unwrap();
        Self {
            app,
            scope,
            keys: keys.clone(),
            base,
            old_home,
            old_xdg,
            _temp: temp,
            _environment: environment,
        }
    }

    fn seed(&self, time: u64, tag: &str) -> RetainedEvent {
        let event = crate::managed_agents::team_catalog::build_team_catalog_event(
            &team(),
            &members(),
            true,
        )
        .unwrap()
        .tag(Tag::parse(["race", tag]).unwrap())
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(&self.keys)
        .unwrap();
        let row = RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey: event.pubkey.to_hex(),
            d_tag: team().id,
            content: event.content.to_string(),
            created_at: time as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        };
        retain_event(&open_retention_db(&self.scope.db_path).unwrap(), &row).unwrap();
        row
    }

    fn head(&self) -> Option<RetainedEvent> {
        retained_head(
            &self.scope.db_path,
            &self.scope.owner_signer().public_key().to_hex(),
        )
    }

    fn pending(&self) -> Vec<RetainedEvent> {
        get_pending_sync(&open_retention_db(&self.scope.db_path).unwrap()).unwrap()
    }

    async fn delete_job(&self, controlled: &Arc<ControlledSigner>) -> CatalogRetraction {
        let mut job = {
            let state = self.app.state::<AppState>();
            let _guard = state.managed_agents_store_lock.lock().unwrap();
            prepare_catalog_delete_in_scope(self.app.handle(), &self.scope, &team().id).unwrap()
        };
        job.signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
        job
    }

    fn spawn(&self, job: CatalogRetraction) -> tokio::task::JoinHandle<Result<(), String>> {
        let app = self.app.handle().clone();
        tokio::spawn(async move {
            crate::managed_agents::team_catalog::tombstone_team_catalog_coordinate(job, &app).await
        })
    }

    fn assert_unlocked(&self) {
        let state = self.app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .try_lock()
            .expect("signing must release store lock");
        let conn = open_retention_db(&self.scope.db_path).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK")
            .expect("signing must release SQLite writer lock");
    }

    fn assert_atomic_witness(&self, floor: u64) {
        assert!(self.head().is_none());
        let pending = self.pending();
        let row = pending
            .iter()
            .find(|row| row.kind == 5 && row.d_tag == "30178:team-abc")
            .unwrap();
        let event = Event::from_json(&row.raw_event).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, self.scope.owner_signer().public_key());
        assert!(event.created_at.as_secs() > floor);
        assert_eq!(event.kind.as_u16(), 5);
        assert_eq!(event.tags.len(), 1);
        assert_eq!(
            event.tags.iter().next().unwrap().as_slice(),
            &["a", &format!("30178:{}:team-abc", event.pubkey.to_hex())]
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (name, value) in [("HOME", &self.old_home), ("XDG_DATA_HOME", &self.old_xdg)] {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[tokio::test]
async fn future_head_atomic_purge_uses_captured_scope_without_locks() {
    let backend = ControlledSigner::new(false);
    let f = Fixture::new(&backend.keys);
    let floor = Timestamp::now().as_secs() + 3600;
    f.seed(floor, "original");
    let task = f.spawn(f.delete_job(&backend).await);
    backend.wait_entered().await;
    f.assert_unlocked();
    assert!(f.head().is_some());
    // A workspace/identity switch cannot redirect either author or database.
    f.app
        .state::<AppState>()
        .replace_local_identity_keys(Keys::generate())
        .unwrap();
    *f.app.state::<AppState>().relay_url_override.lock().unwrap() =
        Some("wss://new.example".into());
    backend.release.notify_one();
    task.await.unwrap().unwrap();
    f.assert_atomic_witness(floor);
    let new_scope = crate::managed_agents::retention::active_retention_scope(
        f.app.handle(),
        &f.app.state::<AppState>(),
    )
    .unwrap();
    assert_ne!(new_scope.db_path, f.scope.db_path);
    assert!(
        get_pending_sync(&open_retention_db(&new_scope.db_path).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn same_second_raw_head_replacement_and_absent_head_insertion_conflict() {
    for initially_present in [true, false] {
        let backend = ControlledSigner::new(false);
        let f = Fixture::new(&backend.keys);
        let time = Timestamp::now().as_secs() + 100;
        if initially_present {
            f.seed(time, "old");
        }
        let task = f.spawn(f.delete_job(&backend).await);
        backend.wait_entered().await;
        f.assert_unlocked();
        let replacement = f.seed(time, "tag-only replacement");
        backend.release.notify_one();
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .contains("retained head changed"));
        assert_eq!(f.head().unwrap().raw_event, replacement.raw_event);
        assert_eq!(f.pending().len(), 1);
        assert!(f.head().unwrap().pending_sync);
    }
}

#[tokio::test]
async fn failed_and_cancelled_signer_leave_head_and_prior_retry_unchanged() {
    for cancel in [false, true] {
        let backend = ControlledSigner::new(!cancel);
        let f = Fixture::new(&backend.keys);
        let old = f.seed(Timestamp::now().as_secs() + 100, "old");
        // Existing retry evidence must survive too, not merely the head.
        let retry_event = crate::managed_agents::team_catalog::build_team_catalog_delete(
            &team().id,
            &backend.keys.public_key().to_hex(),
        )
        .unwrap()
        .sign_with_keys(&backend.keys)
        .unwrap();
        let retry = RetainedEvent {
            kind: 5,
            pubkey: backend.keys.public_key().to_hex(),
            d_tag: "30178:team-abc".into(),
            content: String::new(),
            created_at: retry_event.created_at.as_secs() as i64,
            raw_event: retry_event.as_json(),
            pending_sync: true,
        };
        retain_event(&open_retention_db(&f.scope.db_path).unwrap(), &retry).unwrap();
        let task = f.spawn(f.delete_job(&backend).await);
        backend.wait_entered().await;
        f.assert_unlocked();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            backend.release.notify_one();
            assert!(task
                .await
                .unwrap()
                .unwrap_err()
                .contains("deliberate signer failure"));
        }
        assert_eq!(f.head().unwrap().raw_event, old.raw_event);
        let rows = f.pending();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().find(|row| row.kind == 5).unwrap().raw_event,
            retry.raw_event
        );
    }
}

#[tokio::test]
async fn enqueue_failure_rolls_back_purge_and_duplicate_delete_preserves_witness() {
    let backend = ControlledSigner::new(false);
    let f = Fixture::new(&backend.keys);
    let floor = Timestamp::now().as_secs() + 100;
    let old = f.seed(floor, "old");
    let first = f.delete_job(&backend).await;
    let duplicate = f.delete_job(&backend).await;
    let conn = open_retention_db(&f.scope.db_path).unwrap();
    conn.execute_batch("CREATE TRIGGER block_delete BEFORE INSERT ON persona_events WHEN NEW.kind = 5 BEGIN SELECT RAISE(ABORT, 'blocked tombstone'); END").unwrap();
    drop(conn);
    let task = f.spawn(first);
    backend.wait_entered().await;
    backend.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("blocked tombstone"));
    assert_eq!(f.head().unwrap().raw_event, old.raw_event);
    open_retention_db(&f.scope.db_path)
        .unwrap()
        .execute_batch("DROP TRIGGER block_delete")
        .unwrap();
    let winner = f.spawn(f.delete_job(&backend).await);
    backend.wait_entered().await;
    backend.release.notify_one();
    winner.await.unwrap().unwrap();
    let retry = f.pending().into_iter().find(|row| row.kind == 5).unwrap();
    let loser = f.spawn(duplicate);
    backend.wait_entered().await;
    backend.release.notify_one();
    assert!(loser
        .await
        .unwrap()
        .unwrap_err()
        .contains("retained head changed"));
    assert_eq!(
        f.pending()
            .iter()
            .find(|row| row.kind == 5)
            .unwrap()
            .raw_event,
        retry.raw_event
    );
    f.assert_atomic_witness(floor);
}

#[tokio::test]
async fn changed_team_or_missing_member_repaired_during_sign_is_not_purged() {
    for repair_member in [false, true] {
        let backend = ControlledSigner::new(false);
        let f = Fixture::new(&backend.keys);
        let old = f.seed(Timestamp::now().as_secs() + 100, "old");
        let mut job = {
            let state = f.app.state::<AppState>();
            let _guard = state.managed_agents_store_lock.lock().unwrap();
            let mut teams = load_teams(f.app.handle()).unwrap();
            teams.push(team());
            save_teams(f.app.handle(), &teams).unwrap();
            let personas = load_personas(f.app.handle()).unwrap();
            // Both required members absent: the real resolution-failure arm.
            prepare_catalog_refresh_in_scope(&f.scope, &team(), &personas)
                .unwrap()
                .unwrap()
        };
        job.signer = ActiveUserSigner::new(backend.clone()).await.unwrap();
        let task = f.spawn(job);
        backend.wait_entered().await;
        f.assert_unlocked();
        {
            let state = f.app.state::<AppState>();
            let _guard = state.managed_agents_store_lock.lock().unwrap();
            if repair_member {
                let mut personas = load_personas(f.app.handle()).unwrap();
                personas.extend(members());
                save_personas(f.app.handle(), &personas).unwrap();
            } else {
                let mut teams = load_teams(f.app.handle()).unwrap();
                teams.iter_mut().find(|t| t.id == team().id).unwrap().name =
                    "replacement team".into();
                save_teams(f.app.handle(), &teams).unwrap();
            }
        }
        backend.release.notify_one();
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .contains("disk inputs changed"));
        assert_eq!(f.head().unwrap().raw_event, old.raw_event);
        assert_eq!(f.pending().len(), 1);
    }
}

#[tokio::test]
async fn absent_disk_team_recreated_during_delete_sign_is_preserved() {
    let backend = ControlledSigner::new(false);
    let f = Fixture::new(&backend.keys);
    let old = f.seed(Timestamp::now().as_secs() + 100, "old");
    let task = f.spawn(f.delete_job(&backend).await);
    backend.wait_entered().await;
    {
        let state = f.app.state::<AppState>();
        let _guard = state.managed_agents_store_lock.lock().unwrap();
        let mut teams = load_teams(f.app.handle()).unwrap();
        teams.push(team());
        save_teams(f.app.handle(), &teams).unwrap();
    }
    backend.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("disk inputs changed"));
    assert_eq!(f.head().unwrap().raw_event, old.raw_event);
}

#[tokio::test]
async fn boot_waits_for_tombstone_before_last_legs_and_fatal_team_error_stops_it() {
    for fail_teams in [false, true] {
        let backend = ControlledSigner::new(false);
        let f = Fixture::new(&backend.keys);
        let floor = Timestamp::now().as_secs() + 100;
        f.seed(floor, "orphan catalog");
        let mut live_team = team();
        live_team.id = "live-team".into();
        let mut teams = load_teams(f.app.handle()).unwrap();
        teams.push(live_team);
        save_teams(f.app.handle(), &teams).unwrap();
        let mut agent = member("agent-definition", "Agent after catalog").into_agent_record();
        agent.pubkey = Keys::generate().public_key().to_hex();
        let agent_pubkey = agent.pubkey.clone();
        // Boot reads the raw unified store; this record deliberately has no
        // local secret requirement because 30177 retention is owner-authored.
        let agent_path = f.base.join("managed-agents.json");
        let mut agents: Vec<crate::managed_agents::ManagedAgentRecord> =
            serde_json::from_str(&std::fs::read_to_string(&agent_path).unwrap()).unwrap();
        agents.push(agent);
        std::fs::write(agent_path, serde_json::to_vec(&agents).unwrap()).unwrap();
        // A separate orphan 30176 is the observable LAST deletion-sweep leg.
        let event = crate::managed_agents::team_events::build_team_event(&team())
            .unwrap()
            .sign_with_keys(&backend.keys)
            .unwrap();
        retain_event(
            &open_retention_db(&f.scope.db_path).unwrap(),
            &RetainedEvent {
                kind: KIND_TEAM,
                pubkey: backend.keys.public_key().to_hex(),
                d_tag: team().id,
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
        .unwrap();
        // Positive persona writers now use the same async capability. Seed
        // that earlier leg so this test suspends specifically at team/catalog.
        crate::event_sync::migrate_personas_to_events(
            f.app.handle(),
            &ActiveUserSigner::local(backend.keys.clone()),
            &f.scope.db_path,
        )
        .await;
        if fail_teams {
            std::fs::write(f.base.join("teams.json"), "{broken").unwrap();
        }
        let signer = ActiveUserSigner::new(backend.clone()).await.unwrap();
        let task = tokio::spawn(crate::event_sync::run_event_sync_blocking(
            f.app.handle().clone(),
            signer,
            f.scope.db_path.clone(),
        ));
        if fail_teams {
            assert!(task
                .await
                .unwrap()
                .unwrap_err()
                .contains("team-event-migration"));
            assert!(f.head().is_some());
        } else {
            // The team prerequisite signs first; only after it commits may
            // catalog signing start.
            backend.wait_entered().await;
            f.assert_unlocked();
            backend.release.notify_one();
            backend.wait_entered().await;
            f.assert_unlocked();
            assert!(
                !task.is_finished(),
                "community exposure must still be awaiting reconcile"
            );
            assert!(get_retained_event(
                &open_retention_db(&f.scope.db_path).unwrap(),
                KIND_TEAM,
                &backend.keys.public_key().to_hex(),
                &team().id
            )
            .unwrap()
            .is_some());
            let conn = open_retention_db(&f.scope.db_path).unwrap();
            assert!(
                get_retained_event(
                    &conn,
                    KIND_TEAM,
                    &backend.keys.public_key().to_hex(),
                    "live-team"
                )
                .unwrap()
                .is_some(),
                "fatal team prerequisite must precede catalog signing"
            );
            assert!(
                get_retained_event(
                    &conn,
                    buzz_core_pkg::kind::KIND_MANAGED_AGENT,
                    &backend.keys.public_key().to_hex(),
                    &agent_pubkey
                )
                .unwrap()
                .is_none(),
                "managed-agent leg must wait for catalog"
            );
            drop(conn);
            backend.release.notify_one();
            // Subsequent agent and negative-side sweep writers also await the
            // captured signer. Keep driving those later legs, never detach.
            let mut task = task;
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        result = &mut task => { result.unwrap().unwrap(); break; }
                        _ = backend.entered.notified() => backend.release.notify_one(),
                    }
                }
            })
            .await
            .unwrap();
            f.assert_atomic_witness(floor);
        }
        let agent_head = get_retained_event(
            &open_retention_db(&f.scope.db_path).unwrap(),
            buzz_core_pkg::kind::KIND_MANAGED_AGENT,
            &backend.keys.public_key().to_hex(),
            &agent_pubkey,
        )
        .unwrap();
        assert_eq!(agent_head.is_none(), fail_teams);
        let team_head = get_retained_event(
            &open_retention_db(&f.scope.db_path).unwrap(),
            KIND_TEAM,
            &backend.keys.public_key().to_hex(),
            &team().id,
        )
        .unwrap();
        assert_eq!(team_head.is_some(), fail_teams);
    }
}

#[tokio::test]
async fn unchanged_unresolvable_disk_inputs_commit_but_unrelated_personas_do_not_conflict() {
    let backend = ControlledSigner::new(false);
    let f = Fixture::new(&backend.keys);
    let floor = Timestamp::now().as_secs() + 100;
    f.seed(floor, "shared head");
    let mut job = {
        let state = f.app.state::<AppState>();
        let _guard = state.managed_agents_store_lock.lock().unwrap();
        let mut teams = load_teams(f.app.handle()).unwrap();
        teams.push(team());
        save_teams(f.app.handle(), &teams).unwrap();
        let personas = load_personas(f.app.handle()).unwrap();
        prepare_catalog_refresh_in_scope(&f.scope, &team(), &personas)
            .unwrap()
            .unwrap()
    };
    job.signer = ActiveUserSigner::new(backend.clone()).await.unwrap();
    let task = f.spawn(job);
    backend.wait_entered().await;
    {
        let state = f.app.state::<AppState>();
        let _guard = state.managed_agents_store_lock.lock().unwrap();
        let mut personas = load_personas(f.app.handle()).unwrap();
        personas.push(member(
            "unrelated-private-persona",
            "Never part of this team",
        ));
        save_personas(f.app.handle(), &personas).unwrap();
    }
    backend.release.notify_one();
    task.await.unwrap().unwrap();
    f.assert_atomic_witness(floor);
}
