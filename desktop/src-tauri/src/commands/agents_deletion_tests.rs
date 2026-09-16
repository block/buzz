//! Actual deletion command cores, with real disk records and retention DBs.
//! Removing a pre-commit record/head fence makes these suspended-signer tests fail.
use super::deletion::delete_managed_agent_with;
use crate::{
    active_user_signer::{tests::ControlledSigner, ActiveUserSigner},
    app_state::{build_app_state, AppState},
    managed_agents::{
        load_managed_agents, load_personas, load_teams, managed_agents_base_dir,
        retention::{
            active_retention_scope, get_pending_sync, get_retained_event, open_retention_db,
            retain_event, RetainedEvent, RetentionScope,
        },
        save_managed_agents, save_personas, save_teams, AgentDefinition, ManagedAgentRecord,
    },
};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use std::{
    ffi::OsString,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tauri::Manager;

const PERSONA: &str = "custom:delete-signer";

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    scope: RetentionScope,
    signer: Arc<ControlledSigner>,
    agents: Vec<String>,
    old_home: Option<OsString>,
    old_xdg: Option<OsString>,
    _temp: tempfile::TempDir,
    _environment: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    async fn new(fail: bool, count: usize) -> Self {
        let signer = ControlledSigner::new(fail);
        let capability = ActiveUserSigner::new(signer.clone())
            .await
            .unwrap()
            .with_test_authorization(signer.keys.clone());
        let environment = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        let state = build_app_state();
        state
            .replace_local_identity_keys(signer.keys.clone())
            .unwrap();
        *state.test_signer.lock().unwrap() = Some(capability);
        *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:1".into());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let scope = active_retention_scope(app.handle(), &app.state::<AppState>()).unwrap();
        load_teams(app.handle()).unwrap();
        load_personas(app.handle()).unwrap();
        let persona: AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": PERSONA, "display_name": "Delete Test", "system_prompt": "Test",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        save_personas(app.handle(), &[persona]).unwrap();
        let agents: Vec<_> = (0..count)
            .map(|_| Keys::generate().public_key().to_hex())
            .collect();
        let records: Vec<_> = agents.iter().map(|pk| record(pk)).collect();
        save_managed_agents(app.handle(), &records).unwrap();
        Self {
            app,
            scope,
            signer,
            agents,
            old_home,
            old_xdg,
            _temp: temp,
            _environment: environment,
        }
    }

    fn seed(&self, agent: &str, time: u64, version: &str) -> RetainedEvent {
        let event = EventBuilder::new(
            Kind::Custom(30177),
            r#"{"name":"Test","persona_id":"custom:delete-signer"}"#,
        )
        .tags([
            Tag::parse(["d", agent]).unwrap(),
            Tag::parse(["version", version]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(&self.signer.keys)
        .unwrap();
        let row = RetainedEvent {
            kind: 30177,
            pubkey: event.pubkey.to_hex(),
            d_tag: agent.into(),
            content: event.content.clone(),
            created_at: time as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        };
        retain_event(&open_retention_db(&self.scope.db_path).unwrap(), &row).unwrap();
        row
    }

    fn head(&self, agent: &str) -> Option<RetainedEvent> {
        get_retained_event(
            &open_retention_db(&self.scope.db_path).unwrap(),
            30177,
            &self.signer.keys.public_key().to_hex(),
            agent,
        )
        .unwrap()
    }

    fn pending(&self) -> Vec<RetainedEvent> {
        get_pending_sync(&open_retention_db(&self.scope.db_path).unwrap()).unwrap()
    }

    fn records(&self) -> Vec<ManagedAgentRecord> {
        load_managed_agents(self.app.handle()).unwrap()
    }

    fn spawn(&self, cascade: bool) -> tokio::task::JoinHandle<Result<(), String>> {
        let app = self.app.handle().clone();
        let agent = self.agents.first().cloned().unwrap_or_default();
        tokio::spawn(async move {
            if cascade {
                crate::commands::personas::delete_persona_with(PERSONA.into(), app).await
            } else {
                delete_managed_agent_with(agent, None, app).await
            }
        })
    }

    fn unlocked(&self) {
        assert!(self
            .app
            .state::<AppState>()
            .managed_agents_store_lock
            .try_lock()
            .is_ok());
        open_retention_db(&self.scope.db_path)
            .unwrap()
            .execute_batch("BEGIN IMMEDIATE; ROLLBACK")
            .unwrap();
    }

    async fn finish(
        &self,
        task: tokio::task::JoinHandle<Result<(), String>>,
        remaining: usize,
    ) -> Result<(), String> {
        self.signer.release.notify_one();
        for _ in 0..remaining {
            self.signer.wait_entered().await;
            self.signer.release.notify_one();
        }
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (name, prior) in [("HOME", &self.old_home), ("XDG_DATA_HOME", &self.old_xdg)] {
            match prior {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn record(pk: &str) -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": pk, "name": "Test Agent", "persona_id": PERSONA,
        "relay_url": "", "acp_command": "", "agent_command": "", "agent_args": [],
        "mcp_command": "", "turn_timeout_seconds": 0,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

#[tokio::test]
async fn captured_owner_future_head_and_pair_persist_after_real_delete() {
    let f = Fixture::new(false, 1).await;
    let floor = Timestamp::now().as_secs() + 86_400;
    f.seed(&f.agents[0], floor, "old");
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    f.unlocked();
    assert_eq!(f.records().len(), 1);
    let state = f.app.state::<AppState>();
    state.replace_local_identity_keys(Keys::generate()).unwrap();
    *state.test_signer.lock().unwrap() = None;
    *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:2".into());
    f.finish(task, 1).await.unwrap();
    assert!(f.records().is_empty());
    assert!(f.head(&f.agents[0]).is_none());
    let pending = f.pending();
    assert_eq!(pending.len(), 2);
    for row in &pending {
        let event = Event::from_json(&row.raw_event).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, f.signer.keys.public_key());
        assert_ne!(
            event.pubkey.to_hex(),
            f.agents[0],
            "owner, not independent agent, authors witnesses"
        );
        if row.kind == 5 {
            assert!(row.created_at > floor as i64);
            assert!(
                event
                    .tags
                    .iter()
                    .any(|t| t.as_slice()
                        == ["a", &format!("30177:{}:{}", event.pubkey, f.agents[0])])
            );
        } else {
            assert_eq!(row.kind, 9035);
            assert_eq!(row.content, format!(r#"{{"persona_id":"{PERSONA}"}}"#));
        }
    }
}

#[tokio::test]
async fn failing_first_signature_is_best_effort_not_delete_denial() {
    let f = Fixture::new(true, 1).await;
    let head = f.seed(&f.agents[0], Timestamp::now().as_secs(), "old");
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    f.unlocked();
    f.finish(task, 0).await.unwrap();
    assert!(f.records().is_empty());
    assert_eq!(f.head(&f.agents[0]).unwrap().raw_event, head.raw_event);
    assert!(f.pending().is_empty());
}

#[tokio::test]
async fn failed_archive_signature_keeps_pair_atomic_but_does_not_deny_delete() {
    let f = Fixture::new(false, 1).await;
    f.signer.fail_kind.store(9035, Ordering::SeqCst);
    f.seed(&f.agents[0], Timestamp::now().as_secs(), "old");
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    f.finish(task, 1).await.unwrap();
    assert!(f.records().is_empty());
    assert!(f.head(&f.agents[0]).is_some());
    assert!(
        f.pending().is_empty(),
        "no partial kind:5 when archive signing fails"
    );
}

#[tokio::test]
async fn cancellation_before_destructive_commit_keeps_disk_and_head() {
    let f = Fixture::new(false, 1).await;
    let head = f.seed(&f.agents[0], Timestamp::now().as_secs(), "old");
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    // Also exercise cancellation at the SECOND signature, after kind:5 exists in memory.
    f.signer.release.notify_one();
    f.signer.wait_entered().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.unlocked();
    assert_eq!(f.records().len(), 1);
    assert_eq!(f.head(&f.agents[0]).unwrap().raw_event, head.raw_event);
    assert!(f.pending().is_empty());
}

#[tokio::test]
async fn same_second_raw_head_replacement_conflicts_before_disk_delete() {
    let f = Fixture::new(false, 1).await;
    let time = Timestamp::now().as_secs();
    f.seed(&f.agents[0], time, "old");
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    f.unlocked();
    let replacement = f.seed(&f.agents[0], time, "replacement");
    let error = f.finish(task, 1).await.unwrap_err();
    assert!(error.contains("retained head changed"));
    assert_eq!(f.records().len(), 1);
    assert_eq!(
        f.head(&f.agents[0]).unwrap().raw_event,
        replacement.raw_event
    );
}

#[tokio::test]
async fn absent_head_becoming_present_conflicts_even_after_signing_failure() {
    let f = Fixture::new(true, 1).await;
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    f.seed(&f.agents[0], Timestamp::now().as_secs(), "new");
    assert!(f
        .finish(task, 0)
        .await
        .unwrap_err()
        .contains("retained head changed"));
    assert_eq!(f.records().len(), 1);
}

#[tokio::test]
async fn changed_record_is_not_deleted_after_suspended_signing() {
    let f = Fixture::new(false, 1).await;
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    let mut records = f.records();
    records[0].name = "Replacement".into();
    save_managed_agents(f.app.handle(), &records).unwrap();
    assert!(f
        .finish(task, 1)
        .await
        .unwrap_err()
        .contains("record changed"));
    assert_eq!(f.records()[0].name, "Replacement");
    assert!(f.pending().is_empty());
}

#[tokio::test]
async fn cascade_membership_race_is_non_destructive() {
    let f = Fixture::new(false, 1).await;
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    f.unlocked();
    let mut records = f.records();
    records.push(record(&Keys::generate().public_key().to_hex()));
    save_managed_agents(f.app.handle(), &records).unwrap();
    assert!(f
        .finish(task, 2)
        .await
        .unwrap_err()
        .contains("disk inputs changed"));
    assert_eq!(f.records().len(), 2);
    assert!(load_personas(f.app.handle())
        .unwrap()
        .iter()
        .any(|p| p.id == PERSONA));
}

#[tokio::test]
async fn new_team_reference_blocks_cascade_after_signing() {
    let f = Fixture::new(false, 1).await;
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    let mut teams = load_teams(f.app.handle()).unwrap();
    let team = serde_json::from_value(serde_json::json!({"id":"new-team", "name":"New",
        "persona_ids":[PERSONA], "created_at":"now", "updated_at":"now"}))
    .unwrap();
    teams.push(team);
    save_teams(f.app.handle(), &teams).unwrap();
    assert!(f
        .finish(task, 2)
        .await
        .unwrap_err()
        .contains("disk inputs changed"));
    assert_eq!(f.records().len(), 1);
}

#[tokio::test]
async fn cascade_keeps_independent_witness_success_when_one_pair_fails() {
    let f = Fixture::new(false, 2).await;
    for agent in &f.agents {
        f.seed(agent, Timestamp::now().as_secs(), "old");
    }
    open_retention_db(&f.scope.db_path).unwrap().execute_batch(&format!(
        "CREATE TRIGGER fail_one_archive BEFORE INSERT ON persona_events WHEN NEW.kind=9035 AND NEW.d_tag='{}' BEGIN SELECT RAISE(ABORT, 'one archive failed'); END;", f.agents[0])).unwrap();
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    f.finish(task, 4).await.unwrap();
    assert!(f.records().is_empty());
    assert!(!load_personas(f.app.handle())
        .unwrap()
        .iter()
        .any(|p| p.id == PERSONA));
    assert!(f.head(&f.agents[0]).is_some());
    assert!(f.head(&f.agents[1]).is_none());
    let pending = f.pending();
    assert_eq!(pending.iter().filter(|p| p.kind == 9035).count(), 1);
    assert_eq!(
        pending.iter().filter(|p| p.kind == 5).count(),
        2,
        "one agent and the persona tombstone"
    );
}

#[tokio::test]
async fn cascade_all_signatures_fail_but_base_delete_policy_is_preserved() {
    let f = Fixture::new(true, 2).await;
    for agent in &f.agents {
        f.seed(agent, Timestamp::now().as_secs(), "old");
    }
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    f.finish(task, 2).await.unwrap();
    assert!(f.records().is_empty());
    for agent in &f.agents {
        assert!(f.head(agent).is_some());
    }
    assert!(f.pending().iter().all(|row| row.kind != 9035));
    // The base policy treats the persona witness independently from agent pairs.
    assert!(f.pending().is_empty());
    assert!(managed_agents_base_dir(f.app.handle()).unwrap().exists());
}

#[tokio::test]
async fn head_conflict_restores_existing_bestie_assignment() {
    use crate::managed_agents::bestie_assignment::{assignment_matches, replace_assignment};
    let f = Fixture::new(false, 1).await;
    let time = Timestamp::now().as_secs();
    f.seed(&f.agents[0], time, "old");
    replace_assignment(
        &mut open_retention_db(&f.scope.db_path).unwrap(),
        &f.agents[0],
    )
    .unwrap();
    let task = f.spawn(false);
    f.signer.wait_entered().await;
    f.seed(&f.agents[0], time, "replacement");
    assert!(f
        .finish(task, 1)
        .await
        .unwrap_err()
        .contains("retained head changed"));
    assert_eq!(f.records().len(), 1);
    assert!(
        assignment_matches(&open_retention_db(&f.scope.db_path).unwrap(), &f.agents[0]).unwrap()
    );
    assert!(!managed_agents_base_dir(f.app.handle())
        .unwrap()
        .join("bestie-assignment-recovery.json")
        .exists());
}

#[tokio::test]
async fn cascade_cancellation_leaves_persona_agents_and_all_heads() {
    let f = Fixture::new(false, 2).await;
    for agent in &f.agents {
        f.seed(agent, Timestamp::now().as_secs(), "old");
    }
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    // One whole pair is signed, then cancellation occurs in the next pair.
    for _ in 0..2 {
        f.signer.release.notify_one();
        f.signer.wait_entered().await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.unlocked();
    assert_eq!(f.records().len(), 2);
    assert!(load_personas(f.app.handle())
        .unwrap()
        .iter()
        .any(|p| p.id == PERSONA));
    for agent in &f.agents {
        assert!(f.head(agent).is_some());
    }
    assert!(f.pending().is_empty());
}

#[tokio::test]
async fn cascade_partial_signing_failure_preserves_independent_pair_success() {
    let f = Fixture::new(false, 2).await;
    for agent in &f.agents {
        f.seed(agent, Timestamp::now().as_secs(), "old");
    }
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    for _ in 0..2 {
        f.signer.release.notify_one();
        f.signer.wait_entered().await;
    }
    f.signer.fail_kind.store(9035, Ordering::SeqCst);
    f.finish(task, 2).await.unwrap();
    assert!(f.records().is_empty());
    assert_eq!(f.agents.iter().filter(|pk| f.head(pk).is_some()).count(), 1);
    assert_eq!(f.pending().iter().filter(|row| row.kind == 9035).count(), 1);
    assert_eq!(f.pending().iter().filter(|row| row.kind == 5).count(), 2);
}

#[tokio::test]
async fn persona_only_delete_fences_its_own_head_before_disk_removal() {
    let f = Fixture::new(false, 0).await;
    let persona = load_personas(f.app.handle())
        .unwrap()
        .into_iter()
        .find(|p| p.id == PERSONA)
        .unwrap();
    let d_tag = crate::managed_agents::persona_events::persona_d_tag(&persona);
    let event = EventBuilder::new(Kind::Custom(30175), "persona")
        .tag(Tag::parse(["d", &d_tag]).unwrap())
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 86400))
        .sign_with_keys(&f.signer.keys)
        .unwrap();
    let mut row = RetainedEvent {
        kind: 30175,
        pubkey: event.pubkey.to_hex(),
        d_tag: d_tag.clone(),
        content: event.content.clone(),
        raw_event: event.as_json(),
        created_at: event.created_at.as_secs() as i64,
        pending_sync: false,
    };
    retain_event(&open_retention_db(&f.scope.db_path).unwrap(), &row).unwrap();
    let task = f.spawn(true);
    f.signer.wait_entered().await;
    f.unlocked();
    let newer = EventBuilder::new(Kind::Custom(30175), "same-second replacement")
        .tag(Tag::parse(["d", &d_tag]).unwrap())
        .custom_created_at(event.created_at)
        .sign_with_keys(&f.signer.keys)
        .unwrap();
    row.raw_event = newer.as_json();
    row.content = newer.content.clone();
    retain_event(&open_retention_db(&f.scope.db_path).unwrap(), &row).unwrap();
    assert!(f
        .finish(task, 0)
        .await
        .unwrap_err()
        .contains("retained head changed"));
    assert!(load_personas(f.app.handle())
        .unwrap()
        .iter()
        .any(|p| p.id == PERSONA));
    assert!(f.pending().is_empty());
}

#[tokio::test]
async fn repeated_delete_replays_committed_assignment_cleanup_before_not_found() {
    use crate::managed_agents::bestie_assignment::{assignment_matches, replace_assignment};
    let f = Fixture::new(false, 1).await;
    let agent = &f.agents[0];
    replace_assignment(&mut open_retention_db(&f.scope.db_path).unwrap(), agent).unwrap();
    let journal = managed_agents_base_dir(f.app.handle())
        .unwrap()
        .join("bestie-assignment-recovery.json");
    std::fs::write(
        &journal,
        serde_json::to_vec(&serde_json::json!({
            "version": 1, "assignments": [{"agent_pubkey": agent, "path": f.scope.db_path}]
        }))
        .unwrap(),
    )
    .unwrap();
    save_managed_agents(f.app.handle(), &[]).unwrap();
    let error = delete_managed_agent_with(agent.clone(), None, f.app.handle().clone())
        .await
        .unwrap_err();
    assert!(error.contains("not found"));
    assert!(!journal.exists());
    assert!(!assignment_matches(&open_retention_db(&f.scope.db_path).unwrap(), agent).unwrap());
}
