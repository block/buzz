//! Suspended-signer coverage of real command prepare/finish and deletion seams.
use super::*;
use crate::{
    active_user_signer::{tests::ControlledSigner, ActiveUserSigner},
    managed_agents::{
        retention::{
            active_retention_scope, get_pending_sync, get_retained_event, open_retention_db,
            retain_event, RetainedEvent, RetentionScope,
        },
        save_personas, AgentDefinition,
    },
};
use nostr::{EventBuilder, JsonUtil, Kind, Tag, Timestamp};
use std::{ffi::OsString, sync::Arc, time::Duration};
use tauri::Manager;

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    signer: Arc<ControlledSigner>,
    scope: RetentionScope,
    team: TeamRecord,
    member: AgentDefinition,
    old_home: Option<OsString>,
    old_xdg: Option<OsString>,
    _temp: tempfile::TempDir,
    _environment: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    async fn new(fail: bool) -> Self {
        let signer = ControlledSigner::new(fail);
        let capability = ActiveUserSigner::new(signer.clone()).await.unwrap();
        let environment = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        let state = crate::app_state::build_app_state();
        state
            .replace_local_identity_keys(signer.keys.clone())
            .unwrap();
        *state.test_signer.lock().unwrap() = Some(capability);
        *state.relay_url_override.lock().unwrap() = Some("wss://original.example".into());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        load_teams(app.handle()).unwrap();
        load_personas(app.handle()).unwrap();
        let member: AgentDefinition = serde_json::from_value(serde_json::json!({
            "id":"custom:member", "display_name":"Member", "system_prompt":"Exact prompt",
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let team: TeamRecord = serde_json::from_value(serde_json::json!({
            "id":"signer-team", "name":"Team", "persona_ids":[member.id],
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
        }))
        .unwrap();
        save_personas(app.handle(), std::slice::from_ref(&member)).unwrap();
        save_teams(app.handle(), std::slice::from_ref(&team)).unwrap();
        let scope = active_retention_scope(app.handle(), &app.state::<AppState>()).unwrap();
        Self {
            app,
            signer,
            scope,
            team,
            member,
            old_home,
            old_xdg,
            _temp: temp,
            _environment: environment,
        }
    }
    fn seed(&self, kind: u16, version: &str) -> RetainedEvent {
        let event = EventBuilder::new(Kind::Custom(kind), "old content")
            .tags([
                Tag::parse(["d", &self.team.id]).unwrap(),
                Tag::parse(["shared", "true"]).unwrap(),
                Tag::parse(["version", version]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 86400))
            .sign_with_keys(&self.signer.keys)
            .unwrap();
        let row = RetainedEvent {
            kind: kind as u32,
            pubkey: event.pubkey.to_hex(),
            d_tag: self.team.id.clone(),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        };
        retain_event(&open_retention_db(&self.scope.db_path).unwrap(), &row).unwrap();
        row
    }
    fn head(&self, kind: u32) -> Option<RetainedEvent> {
        get_retained_event(
            &open_retention_db(&self.scope.db_path).unwrap(),
            kind,
            &self.signer.keys.public_key().to_hex(),
            &self.team.id,
        )
        .unwrap()
    }
    fn unlocked(&self) {
        let state = self.app.state::<AppState>();
        let _guard = state.managed_agents_store_lock.try_lock().unwrap();
        open_retention_db(&self.scope.db_path)
            .unwrap()
            .execute_batch("BEGIN IMMEDIATE; ROLLBACK")
            .unwrap();
    }
    async fn done<T>(&self, task: tokio::task::JoinHandle<T>) -> T {
        self.signer.release.notify_one();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for (key, value) in [("HOME", &self.old_home), ("XDG_DATA_HOME", &self.old_xdg)] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[tokio::test]
async fn team_definition_finishes_in_captured_scope_with_future_floor() {
    let f = Fixture::new(false).await;
    let old = f.seed(30176, "old");
    let work = {
        let state = f.app.state::<AppState>();
        let _guard = state.managed_agents_store_lock.lock().unwrap();
        retain_team_pending_at(&f.scope, &f.team)
    };
    let app = f.app.handle().clone();
    let task = tokio::spawn(async move { finish_team_pending(&app, work).await });
    f.signer.wait_entered().await;
    f.unlocked();
    f.app
        .state::<AppState>()
        .replace_local_identity_keys(nostr::Keys::generate())
        .unwrap();
    *f.app.state::<AppState>().relay_url_override.lock().unwrap() =
        Some("wss://other.example".into());
    f.done(task).await;
    let head = f.head(30176).unwrap();
    assert!(head.created_at > old.created_at);
    assert!(head.pending_sync);
    assert_eq!(head.pubkey, f.signer.keys.public_key().to_hex());
    let event = nostr::Event::from_json(&head.raw_event).unwrap();
    event.verify().unwrap();
    assert_eq!(
        event.content,
        crate::managed_agents::team_events::build_team_event(&f.team)
            .unwrap()
            .build(event.pubkey)
            .content
    );
}

#[tokio::test]
async fn team_definition_rejects_same_timestamp_head_and_disk_changes() {
    for disk in [false, true] {
        let f = Fixture::new(false).await;
        f.seed(30176, "old");
        let work = retain_team_pending_at(&f.scope, &f.team);
        let app = f.app.handle().clone();
        let task = tokio::spawn(async move { finish_team_pending(&app, work).await });
        f.signer.wait_entered().await;
        f.unlocked();
        let expected = if disk {
            let mut team = f.team.clone();
            team.name = "Concurrent edit".into();
            save_teams(f.app.handle(), &[team]).unwrap();
            f.head(30176).unwrap()
        } else {
            f.seed(30176, "concurrent")
        };
        f.done(task).await;
        assert_eq!(f.head(30176).unwrap().raw_event, expected.raw_event);
    }
}

#[tokio::test]
async fn catalog_share_revalidates_members_and_raw_head() {
    for disk in [false, true] {
        let f = Fixture::new(false).await;
        f.seed(30178, "old");
        let work = {
            let state = f.app.state::<AppState>();
            let _guard = state.managed_agents_store_lock.lock().unwrap();
            pending::prepare_team_publication(
                f.app.handle(),
                &state,
                &f.team,
                std::slice::from_ref(&f.member),
                Some(true),
            )
            .unwrap()
        };
        let app = f.app.handle().clone();
        let task = tokio::spawn(async move {
            pending::finish_team_publication(&app, work)
                .await
                .map(|_| ())
        });
        f.signer.wait_entered().await;
        f.unlocked();
        let expected = if disk {
            let mut member = f.member.clone();
            member.system_prompt = "Concurrent prompt".into();
            save_personas(f.app.handle(), &[member]).unwrap();
            f.head(30178).unwrap()
        } else {
            f.seed(30178, "concurrent")
        };
        assert!(f.done(task).await.is_err());
        assert_eq!(f.head(30178).unwrap().raw_event, expected.raw_event);
    }
}

#[tokio::test]
async fn catalog_refresh_uses_real_job_and_rejects_member_change() {
    let f = Fixture::new(false).await;
    let expected = f.seed(30178, "old");
    let work = {
        let state = f.app.state::<AppState>();
        let _guard = state.managed_agents_store_lock.lock().unwrap();
        pending::prepare_catalog_refresh_in_scope(
            &f.scope,
            &f.team,
            std::slice::from_ref(&f.member),
        )
        .unwrap()
        .unwrap()
    };
    let app = f.app.handle().clone();
    let task = tokio::spawn(async move {
        crate::managed_agents::team_catalog::tombstone_team_catalog_coordinate(work, &app).await
    });
    f.signer.wait_entered().await;
    f.unlocked();
    let mut member = f.member.clone();
    member.system_prompt = "Changed".into();
    save_personas(f.app.handle(), &[member]).unwrap();
    assert!(f
        .done(task)
        .await
        .unwrap_err()
        .contains("disk inputs changed"));
    assert_eq!(f.head(30178).unwrap().raw_event, expected.raw_event);
}

#[tokio::test]
async fn team_delete_cancellation_and_head_race_leave_disk_intact() {
    for cancel in [true, false] {
        let f = Fixture::new(false).await;
        f.seed(30176, "old");
        f.seed(30178, "old");
        let app = f.app.handle().clone();
        let id = f.team.id.clone();
        let task = tokio::spawn(async move { deletion::delete_team_with(id, app).await });
        f.signer.wait_entered().await;
        f.unlocked();
        assert!(load_teams(f.app.handle())
            .unwrap()
            .iter()
            .any(|t| t.id == f.team.id));
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            f.seed(30176, "concurrent");
            f.signer.release.notify_one();
            f.signer.wait_entered().await;
            assert!(f
                .done(task)
                .await
                .unwrap_err()
                .contains("retained head changed"));
        }
        assert!(load_teams(f.app.handle())
            .unwrap()
            .iter()
            .any(|t| t.id == f.team.id));
        assert!(
            get_pending_sync(&open_retention_db(&f.scope.db_path).unwrap())
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn team_delete_signature_failure_preserves_base_policy() {
    let f = Fixture::new(true).await;
    f.seed(30176, "old");
    f.seed(30178, "old");
    let app = f.app.handle().clone();
    let id = f.team.id.clone();
    let task = tokio::spawn(async move { deletion::delete_team_with(id, app).await });
    f.signer.wait_entered().await;
    f.unlocked();
    f.signer.release.notify_one();
    f.signer.wait_entered().await;
    f.done(task).await.unwrap();
    assert!(!load_teams(f.app.handle())
        .unwrap()
        .iter()
        .any(|t| t.id == f.team.id));
    assert!(f.head(30176).is_some());
    assert!(f.head(30178).is_some());
}

#[tokio::test]
async fn team_delete_commits_both_future_dated_coordinate_witnesses() {
    let f = Fixture::new(false).await;
    let floor = f.seed(30176, "old").created_at;
    f.seed(30178, "old");
    let app = f.app.handle().clone();
    let id = f.team.id.clone();
    let task = tokio::spawn(async move { deletion::delete_team_with(id, app).await });
    f.signer.wait_entered().await;
    f.unlocked();
    f.signer.release.notify_one();
    f.signer.wait_entered().await;
    assert!(load_teams(f.app.handle())
        .unwrap()
        .iter()
        .any(|t| t.id == f.team.id));
    f.done(task).await.unwrap();
    assert!(f.head(30176).is_none());
    assert!(f.head(30178).is_none());
    let pending = get_pending_sync(&open_retention_db(&f.scope.db_path).unwrap()).unwrap();
    for kind in [30176, 30178] {
        let row = pending
            .iter()
            .find(|r| r.d_tag == format!("{kind}:{}", f.team.id))
            .unwrap();
        assert_eq!(row.kind, 5);
        assert!(row.created_at > floor);
        let event = nostr::Event::from_json(&row.raw_event).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, f.signer.keys.public_key());
    }
}

#[tokio::test]
async fn team_delete_revalidates_only_its_decision_inputs() {
    for change in ["unrelated", "target", "reference"] {
        let f = Fixture::new(false).await;
        let mut other = f.team.clone();
        other.id = "other-team".into();
        save_teams(f.app.handle(), &[f.team.clone(), other.clone()]).unwrap();
        let app = f.app.handle().clone();
        let id = f.team.id.clone();
        let task = tokio::spawn(async move { deletion::delete_team_with(id, app).await });
        f.signer.wait_entered().await;
        f.unlocked();
        match change {
            "unrelated" => {
                other.name = "Edited elsewhere".into();
                save_teams(f.app.handle(), &[f.team.clone(), other]).unwrap();
                let mut persona = f.member.clone();
                persona.system_prompt = "Unrelated edit".into();
                save_personas(f.app.handle(), &[persona]).unwrap();
                let agent: crate::managed_agents::ManagedAgentRecord = serde_json::from_value(serde_json::json!({
                    "pubkey":nostr::Keys::generate().public_key().to_hex(), "name":"Unrelated runtime",
                    "relay_url":"", "acp_command":"", "agent_command":"", "agent_args":[],
                    "mcp_command":"", "turn_timeout_seconds":0,
                    "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
                })).unwrap();
                save_managed_agents(f.app.handle(), &[agent]).unwrap();
            }
            "target" => {
                let mut target = f.team.clone();
                target.persona_ids.clear();
                save_teams(f.app.handle(), &[target, other]).unwrap();
            }
            _ => {
                let agent: crate::managed_agents::ManagedAgentRecord = serde_json::from_value(serde_json::json!({
                    "pubkey":nostr::Keys::generate().public_key().to_hex(), "name":"New reference", "team_id": f.team.id,
                    "relay_url":"", "acp_command":"", "agent_command":"", "agent_args":[],
                    "mcp_command":"", "turn_timeout_seconds":0,
                    "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
                })).unwrap();
                save_managed_agents(f.app.handle(), &[agent]).unwrap();
            }
        }
        f.signer.release.notify_one();
        f.signer.wait_entered().await;
        let result = f.done(task).await;
        assert_eq!(
            result.is_ok(),
            change == "unrelated",
            "{change}: {result:?}"
        );
        assert_eq!(
            load_teams(f.app.handle())
                .unwrap()
                .iter()
                .any(|t| t.id == f.team.id),
            change != "unrelated"
        );
    }
}

#[tokio::test]
async fn catalog_team_delete_fences_cascade_members_and_reference_edges() {
    for change in ["metadata", "member", "reference"] {
        let mut f = Fixture::new(false).await;
        let owner = f.signer.keys.public_key().to_hex();
        f.team.catalog_source = Some(
            serde_json::from_value(serde_json::json!({
                "owner_pubkey":owner, "team_d_tag":"source-team"
            }))
            .unwrap(),
        );
        f.member.team_catalog_source = Some(serde_json::from_value(serde_json::json!({
            "owner_pubkey":owner, "team_d_tag":"source-team", "member_key":"member", "projection_hash":"hash"
        })).unwrap());
        let mut other = f.team.clone();
        other.id = "other".into();
        other.persona_ids.clear();
        save_teams(f.app.handle(), &[f.team.clone(), other.clone()]).unwrap();
        save_personas(f.app.handle(), std::slice::from_ref(&f.member)).unwrap();
        let app = f.app.handle().clone();
        let id = f.team.id.clone();
        let task = tokio::spawn(async move { deletion::delete_team_with(id, app).await });
        f.signer.wait_entered().await;
        match change {
            "metadata" => {
                other.name = "Unrelated metadata".into();
                save_teams(f.app.handle(), &[f.team.clone(), other]).unwrap();
            }
            "member" => {
                let mut member = f.member.clone();
                member.system_prompt = "Concurrent edit".into();
                save_personas(f.app.handle(), &[member]).unwrap();
            }
            _ => {
                other.persona_ids = vec![f.member.id.clone()];
                save_teams(f.app.handle(), &[f.team.clone(), other]).unwrap();
            }
        }
        f.signer.release.notify_one();
        f.signer.wait_entered().await;
        let result = f.done(task).await;
        assert_eq!(result.is_ok(), change == "metadata", "{change}: {result:?}");
        let members = load_personas(f.app.handle()).unwrap();
        assert_eq!(
            members
                .iter()
                .find(|p| p.id == f.member.id)
                .unwrap()
                .is_active,
            change != "metadata"
        );
    }
}
