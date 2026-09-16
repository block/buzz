//! The same prepare/finish writer seams used by commands, with real disk and
//! SQLite state. Signing must not hold either lock or retarget a captured scope.
use super::pending::{finish_persona_publication, retain_persona_pending};
use crate::{
    active_user_signer::{tests::ControlledSigner, ActiveUserSigner},
    app_state::{build_app_state, AppState},
    commands::agents::{finish_managed_agent_pending, prepare_managed_agent_pending},
    managed_agents::{
        agent_events::build_agent_event,
        load_managed_agents, load_personas,
        persona_events::build_persona_event,
        retention::{
            active_retention_scope, get_retained_event, open_retention_db, retain_event,
            RetainedEvent, RetentionScope,
        },
        save_managed_agents, save_personas, AgentDefinition, ManagedAgentRecord,
    },
};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use std::{ffi::OsString, sync::Arc};
use tauri::Manager;

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    signer: Arc<ControlledSigner>,
    scope: RetentionScope,
    persona: AgentDefinition,
    agent: ManagedAgentRecord,
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
        let persona: AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "custom:definition-signer", "display_name": "Definition", "system_prompt": "Exact\nPrompt",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        })).unwrap();
        let agent: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": Keys::generate().public_key().to_hex(), "name": "Instance", "persona_id": persona.id,
            "relay_url": "", "acp_command": "", "agent_command": "", "agent_args": [],
            "mcp_command": "", "turn_timeout_seconds": 0,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        })).unwrap();
        load_personas(app.handle()).unwrap();
        save_personas(app.handle(), std::slice::from_ref(&persona)).unwrap();
        save_managed_agents(app.handle(), std::slice::from_ref(&agent)).unwrap();
        Self {
            app,
            signer,
            scope,
            persona,
            agent,
            old_home,
            old_xdg,
            _temp: temp,
            _environment: environment,
        }
    }

    fn seed(&self, kind: u16, version: &str) -> RetainedEvent {
        let d = if kind == 30177 {
            &self.agent.pubkey
        } else {
            &crate::managed_agents::persona_events::persona_d_tag(&self.persona)
        };
        let event = EventBuilder::new(Kind::Custom(kind), "old content")
            .tags([
                Tag::parse(["d", d]).unwrap(),
                Tag::parse(["version", version]).unwrap(),
                Tag::parse(["shared", "true"]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 86400))
            .sign_with_keys(&self.signer.keys)
            .unwrap();
        let row = RetainedEvent {
            kind: kind.into(),
            pubkey: event.pubkey.to_hex(),
            d_tag: d.clone(),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        };
        retain_event(&open_retention_db(&self.scope.db_path).unwrap(), &row).unwrap();
        row
    }

    fn row(&self, prior: &RetainedEvent) -> Option<RetainedEvent> {
        get_retained_event(
            &open_retention_db(&self.scope.db_path).unwrap(),
            prior.kind,
            &prior.pubkey,
            &prior.d_tag,
        )
        .unwrap()
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

    fn start_agent(&self) -> tokio::task::JoinHandle<()> {
        let work = {
            let state = self.app.state::<AppState>();
            let _guard = state.managed_agents_store_lock.lock().unwrap();
            prepare_managed_agent_pending(self.app.handle(), &state, &self.agent)
        };
        let app = self.app.handle().clone();
        tokio::spawn(async move {
            finish_managed_agent_pending(&app, &app.state::<AppState>(), work).await
        })
    }

    fn start_persona(&self) -> tokio::task::JoinHandle<Result<(), String>> {
        let work = {
            let state = self.app.state::<AppState>();
            let _guard = state.managed_agents_store_lock.lock().unwrap();
            retain_persona_pending(self.app.handle(), &state, &self.persona)
        };
        let app = self.app.handle().clone();
        tokio::spawn(async move { finish_persona_publication(&app, work).await.map(|_| ()) })
    }

    fn switch_scope(&self) {
        let state = self.app.state::<AppState>();
        state.replace_local_identity_keys(Keys::generate()).unwrap();
        *state.test_signer.lock().unwrap() = None;
        *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:2".into());
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
async fn agent_command_writer_captures_exact_owner_projection_and_future_floor() {
    let f = Fixture::new(false).await;
    let prior = f.seed(30177, "first");
    let task = f.start_agent();
    f.signer.wait_entered().await;
    f.unlocked();
    assert_eq!(f.row(&prior).unwrap().raw_event, prior.raw_event);
    f.switch_scope();
    f.signer.release.notify_one();
    task.await.unwrap();
    let row = f.row(&prior).unwrap();
    assert!(row.pending_sync);
    let event = Event::from_json(&row.raw_event).unwrap();
    event.verify().unwrap();
    let expected = build_agent_event(&f.agent)
        .unwrap()
        .custom_created_at(Timestamp::from((prior.created_at + 1) as u64))
        .sign_with_keys(&f.signer.keys)
        .unwrap();
    assert_eq!(event.id, expected.id);
    assert_ne!(event.pubkey.to_hex(), f.agent.pubkey);
}

#[tokio::test]
async fn agent_writer_rejects_store_change_removal_and_same_timestamp_raw_head_change() {
    for change in ["store", "remove", "head"] {
        let f = Fixture::new(false).await;
        let mut prior = f.seed(30177, "first");
        let task = f.start_agent();
        f.signer.wait_entered().await;
        f.unlocked();
        match change {
            "store" => {
                let mut records = load_managed_agents(f.app.handle()).unwrap();
                records
                    .iter_mut()
                    .find(|r| r.pubkey == f.agent.pubkey)
                    .unwrap()
                    .name = "Newer".into();
                save_managed_agents(f.app.handle(), &records).unwrap();
            }
            "remove" => {
                save_managed_agents(f.app.handle(), &[]).unwrap();
            }
            _ => {
                prior.raw_event = EventBuilder::new(Kind::Custom(30177), &prior.content)
                    .tag(Tag::parse(["d", &f.agent.pubkey]).unwrap())
                    .custom_created_at(Timestamp::from(prior.created_at as u64))
                    .sign_with_keys(&f.signer.keys)
                    .unwrap()
                    .as_json();
                retain_event(&open_retention_db(&f.scope.db_path).unwrap(), &prior).unwrap();
            }
        }
        f.signer.release.notify_one();
        task.await.unwrap();
        assert_eq!(
            f.row(&prior).unwrap().raw_event,
            prior.raw_event,
            "{change}"
        );
        assert!(!f.row(&prior).unwrap().pending_sync);
    }
}

#[tokio::test]
async fn persona_writer_captures_share_tag_and_exact_template_without_locks() {
    let f = Fixture::new(false).await;
    let prior = f.seed(30175, "first");
    let task = f.start_persona();
    f.signer.wait_entered().await;
    f.unlocked();
    f.switch_scope();
    f.signer.release.notify_one();
    task.await.unwrap().unwrap();
    let row = f.row(&prior).unwrap();
    assert!(row.pending_sync);
    let mut persona = f.persona.clone();
    persona.shared = true;
    let expected = build_persona_event(&persona)
        .unwrap()
        .custom_created_at(Timestamp::from((prior.created_at + 1) as u64))
        .sign_with_keys(&f.signer.keys)
        .unwrap();
    let event = Event::from_json(&row.raw_event).unwrap();
    event.verify().unwrap();
    assert_eq!(event.id, expected.id);
}

#[tokio::test]
async fn persona_writer_rejects_changed_definition_or_share_head() {
    for change_head in [false, true] {
        let f = Fixture::new(false).await;
        let mut prior = f.seed(30175, "first");
        let task = f.start_persona();
        f.signer.wait_entered().await;
        f.unlocked();
        if change_head {
            // Same timestamp and content, different signed share tag.
            prior.raw_event = EventBuilder::new(Kind::Custom(30175), &prior.content)
                .tag(Tag::parse(["d", &prior.d_tag]).unwrap())
                .custom_created_at(Timestamp::from(prior.created_at as u64))
                .sign_with_keys(&f.signer.keys)
                .unwrap()
                .as_json();
            retain_event(&open_retention_db(&f.scope.db_path).unwrap(), &prior).unwrap();
        } else {
            let mut personas = load_personas(f.app.handle()).unwrap();
            personas
                .iter_mut()
                .find(|p| p.id == f.persona.id)
                .unwrap()
                .system_prompt = "Newer".into();
            save_personas(f.app.handle(), &personas).unwrap();
        }
        f.signer.release.notify_one();
        let error = task.await.unwrap().unwrap_err();
        assert!(
            error.contains("changed during definition signing"),
            "{error}"
        );
        assert_eq!(f.row(&prior).unwrap().raw_event, prior.raw_event);
    }
}

#[tokio::test]
async fn failed_and_cancelled_signing_preserves_disk_and_prior_pending_semantics() {
    for fail in [false, true] {
        let f = Fixture::new(fail).await;
        let prior = f.seed(30177, "first");
        let task = f.start_agent();
        f.signer.wait_entered().await;
        f.unlocked();
        if fail {
            f.signer.release.notify_one();
            task.await.unwrap();
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert_eq!(f.row(&prior).unwrap().raw_event, prior.raw_event);
        assert!(!f.row(&prior).unwrap().pending_sync);
        assert!(load_managed_agents(f.app.handle())
            .unwrap()
            .iter()
            .any(|r| r.pubkey == f.agent.pubkey));
        // The documented disk-authoritative recovery path publishes on boot.
        crate::managed_agents::reconcile::reconcile_agents_to_events(
            f.app.handle(),
            &ActiveUserSigner::local(f.signer.keys.clone()),
            &f.scope.db_path,
        )
        .await;
        assert!(f.row(&prior).unwrap().pending_sync);
    }
}

#[tokio::test]
async fn persona_signer_failure_reaches_strict_caller_and_keeps_disk_save() {
    let f = Fixture::new(true).await;
    let prior = f.seed(30175, "first");
    let task = f.start_persona();
    f.signer.wait_entered().await;
    f.unlocked();
    f.signer.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("deliberate signer failure"));
    assert_eq!(f.row(&prior).unwrap().raw_event, prior.raw_event);
    assert!(load_personas(f.app.handle())
        .unwrap()
        .iter()
        .any(|p| p.id == f.persona.id));
}

fn edit_request(f: &Fixture) -> crate::managed_agents::UpdatePersonaRequest {
    serde_json::from_value(serde_json::json!({
        "id": f.persona.id, "displayName": "Local edit", "systemPrompt": "Local prompt"
    }))
    .unwrap()
}

#[tokio::test]
async fn strict_command_sign_failure_does_not_mutate_linked_records() {
    let f = Fixture::new(true).await;
    let mut linked = f.agent.clone();
    linked.name = f.persona.display_name.clone();
    save_managed_agents(f.app.handle(), std::slice::from_ref(&linked)).unwrap();
    let before = serde_json::to_value(load_managed_agents(f.app.handle()).unwrap()).unwrap();
    let app = f.app.handle().clone();
    let input = edit_request(&f);
    let task =
        tokio::spawn(
            async move { super::sharing::update_persona_and_publish_with(input, app).await },
        );
    f.signer.wait_entered().await;
    f.unlocked();
    f.signer.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("deliberate signer failure"));
    assert_eq!(
        serde_json::to_value(load_managed_agents(f.app.handle()).unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
async fn persona_command_local_edit_survives_older_inbound_during_signing() {
    let f = Fixture::new(false).await;
    let now = Timestamp::now().as_secs();
    let seed = |persona: &AgentDefinition, time| {
        build_persona_event(persona)
            .unwrap()
            .custom_created_at(Timestamp::from(time))
            .sign_with_keys(&f.signer.keys)
            .unwrap()
    };
    let old = seed(&f.persona, now - 100);
    super::inbound::reconcile_for_test(
        old.as_json(),
        f.scope.relay_url.clone(),
        f.app.handle().clone(),
    )
    .unwrap();
    let app = f.app.handle().clone();
    let input = edit_request(&f);
    let task =
        tokio::spawn(async move { super::update::update_persona_with(input, app, false).await });
    f.signer.wait_entered().await;
    f.unlocked();
    let mut inbound = f.persona.clone();
    inbound.display_name = "Older inbound".into();
    let delayed = seed(&inbound, now - 50);
    let app = f.app.handle().clone();
    let relay = f.scope.relay_url.clone();
    let mut arrival = tokio::spawn(async move {
        super::inbound::reconcile_async_for_test(delayed.as_json(), relay, app).await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut arrival)
            .await
            .is_err()
    );
    f.unlocked();
    f.signer.release.notify_one();
    assert!(task.await.unwrap().is_ok());
    tokio::time::timeout(std::time::Duration::from_secs(5), arrival)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let disk = load_personas(f.app.handle()).unwrap();
    assert_eq!(
        disk.iter()
            .find(|p| p.id == f.persona.id)
            .unwrap()
            .display_name,
        "Local edit"
    );
}

#[tokio::test]
async fn strict_command_cancellation_before_signing_does_not_mutate_linked_records() {
    let f = Fixture::new(false).await;
    let mut linked = f.agent.clone();
    linked.name = f.persona.display_name.clone();
    save_managed_agents(f.app.handle(), std::slice::from_ref(&linked)).unwrap();
    let before = serde_json::to_value(load_managed_agents(f.app.handle()).unwrap()).unwrap();
    let app = f.app.handle().clone();
    let input = edit_request(&f);
    let task =
        tokio::spawn(
            async move { super::sharing::update_persona_and_publish_with(input, app).await },
        );
    f.signer.wait_entered().await;
    f.unlocked();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        serde_json::to_value(load_managed_agents(f.app.handle()).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        load_personas(f.app.handle())
            .unwrap()
            .iter()
            .find(|p| p.id == f.persona.id)
            .unwrap()
            .display_name,
        "Local edit",
        "BASE saves persona first; no new cancellation rollback policy"
    );
}

#[tokio::test]
async fn ordinary_command_sign_failure_still_applies_linked_edits() {
    let f = Fixture::new(true).await;
    let mut linked = f.agent.clone();
    linked.name = f.persona.display_name.clone();
    save_managed_agents(f.app.handle(), std::slice::from_ref(&linked)).unwrap();
    let app = f.app.handle().clone();
    let input = edit_request(&f);
    let task =
        tokio::spawn(async move { super::update::update_persona_with(input, app, false).await });
    f.signer.wait_entered().await;
    f.unlocked();
    f.signer.release.notify_one();
    f.signer.wait_entered().await; // linked 30177 is still attempted on ordinary failure
    f.signer.release.notify_one();
    assert!(task.await.unwrap().is_ok());
    assert_eq!(
        load_managed_agents(f.app.handle()).unwrap()[0].name,
        "Local edit"
    );
}

#[path = "inbound_order_tests.rs"]
mod inbound_order_tests;
