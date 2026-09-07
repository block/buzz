//! Signed production receiver → ordinary provider preflight → exact-plan child
//! registration. Only destination-local synthetic identities and bounded fake
//! children; run with --no-default-features (never a real OS credential store).
use super::*;
use buzz_core_pkg::{
    desktop_lifecycle::{Action, Outcome, Request, ResultMessage},
    desktop_stop::{StopOutcome, StopResult, StopTarget},
};
use nostr::{Event, EventBuilder, Timestamp};
use std::{cell::Cell, os::unix::fs::PermissionsExt};
use tauri::Manager;

const COMMUNITY: &str = "wss://remote-credential-fixture.example";

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    record: ManagedAgentRecord,
    owner: nostr::Keys,
    named: RuntimeConfiguration,
    stamp: Cell<u64>,
    env: Vec<(&'static str, Option<std::ffi::OsString>)>,
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let env = ["HOME", "XDG_DATA_HOME", "PATH", "BUZZ_PRIVATE_KEY"]
            .map(|key| (key, std::env::var_os(key)))
            .to_vec();
        let owner = nostr::Keys::generate();
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", temp.path().display()));
        std::env::set_var("BUZZ_PRIVATE_KEY", owner.secret_key().to_secret_hex());
        agents::clear_resolve_cache();
        let mut record = record();
        attest(&mut record, &owner);
        // This key exists only in this isolated fixture. The child checks local
        // credential delivery without writing a secret to output or diagnostics.
        let child = format!(
            "#!/bin/sh\n[ \"$BUZZ_PRIVATE_KEY\" = '{}' ] || exit 12\nprintf '%s|%s\\n' \"$BUZZ_ACP_REQUIRED_MODEL\" \"$BUZZ_ACP_MODEL\" >> '{}'\nexec /bin/sleep 20\n",
            record.private_key_nsec, temp.path().join("launches").display()
        );
        for (name, script) in [
            ("buzz-acp", child.as_str()),
            ("buzz-agent", "#!/bin/sh\nexit 0\n"),
            ("buzz-dev-mcp", "#!/bin/sh\nexit 0\n"),
        ] {
            let path = temp.path().join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        *app.state::<crate::app_state::AppState>()
            .relay_url_override
            .lock()
            .unwrap() = Some(COMMUNITY.into());
        assert_eq!(
            app.state::<crate::app_state::AppState>()
                .signing_keys()
                .unwrap()
                .public_key(),
            owner.public_key()
        );
        record.acp_command = temp.path().join("buzz-acp").display().to_string();
        record.runtime = Some("buzz-agent".into());
        record.agent_command = "buzz-agent".into();
        record
            .env_vars
            .insert("OPENAI_COMPAT_API_KEY".into(), "fixture-only".into());
        let host = local_host(app.handle(), &owner.public_key().to_hex(), COMMUNITY).unwrap();
        let mut named = config(&host);
        named.workspace = Some(temp.path().display().to_string());
        let named = save(&mut record, &owner.public_key().to_hex(), COMMUNITY, named);
        let fixture = Self {
            app,
            record,
            owner,
            named,
            stamp: Cell::new(Timestamp::now().as_secs().saturating_sub(10)),
            env,
            temp,
        };
        fixture.persist();
        fixture
    }
    fn persist(&self) {
        agents::save_managed_agents(self.app.handle(), &[self.record.clone()]).unwrap();
    }
    fn target(&self) -> StopTarget {
        StopTarget {
            v: 1,
            community: COMMUNITY.into(),
            desktop: self.named.host.clone(),
            agent: self.record.pubkey.clone(),
        }
    }
    fn ordered(&self, event: Event) -> Event {
        let stamp = self.stamp.get() + 1;
        self.stamp.set(stamp);
        EventBuilder::new(event.kind, event.content)
            .tags(event.tags.iter().cloned())
            .custom_created_at(Timestamp::from_secs(stamp))
            .sign_with_keys(&self.owner)
            .unwrap()
    }
    fn request(&self, action: Action, observed: Option<String>) -> Event {
        self.ordered(
            Request {
                target: self.target(),
                action,
                observed,
                configuration: (!matches!(action, Action::Catalog | Action::Status))
                    .then(|| self.named.reference()),
                cursor: None,
            }
            .sign(&self.owner)
            .unwrap(),
        )
    }
    async fn receive<F, Fut>(&self, event: Event, preflight: F) -> ResultMessage
    where
        F: Fn(Option<String>, bool) -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let response = crate::commands::receive_desktop_lifecycle_with(
            self.app.handle().clone(),
            self.owner.public_key().to_hex(),
            COMMUNITY.into(),
            event.clone(),
            preflight,
        )
        .await
        .unwrap()
        .unwrap();
        let result = ResultMessage::read(&response, &self.owner, &event, COMMUNITY).unwrap();
        let safe = serde_json::to_string(&result).unwrap();
        assert!(
            !safe.contains(&self.record.private_key_nsec)
                || self.record.private_key_nsec.is_empty()
        );
        assert!(!safe.contains("fixture-only"));
        assert!(!safe.contains(self.temp.path().to_str().unwrap()));
        result
    }
    async fn action(&self, action: Action, observed: Option<String>) -> ResultMessage {
        self.receive(self.request(action, observed), |model, allow| async move {
            assert!(model.is_none());
            assert!(!allow);
            Ok(())
        })
        .await
    }
    fn running(&self) -> Option<(u32, Option<RuntimeConfigurationRef>)> {
        let state = self.app.state::<crate::app_state::AppState>();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        let key = agents::ManagedAgentRuntimeKey::new(&self.record.pubkey, COMMUNITY).unwrap();
        runtimes.get_mut(&key).map(|runtime| {
            assert!(runtime.child.try_wait().unwrap().is_none());
            (
                runtime.child.id(),
                runtime.spawn_config.runtime_configuration.clone(),
            )
        })
    }
    fn launched(&self, expected: &str) {
        for _ in 0..100 {
            if std::fs::read_to_string(self.temp.path().join("launches"))
                .unwrap_or_default()
                .contains(expected)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("fixture child did not confirm exact model and destination-local credential");
    }
    async fn stop(&self) -> StopOutcome {
        let event = self.ordered(self.target().sign(&self.owner).unwrap());
        let response = crate::commands::desktop_stop::receive_desktop_stop_for_app(
            self.app.handle().clone(),
            self.owner.public_key().to_hex(),
            COMMUNITY.into(),
            event.clone(),
        )
        .await
        .unwrap()
        .unwrap();
        StopResult::read(&response, &self.owner, &event, COMMUNITY)
            .unwrap()
            .outcome
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for (_, mut runtime) in self
            .app
            .state::<crate::app_state::AppState>()
            .managed_agent_processes
            .lock()
            .unwrap()
            .drain()
        {
            let _ = agents::terminate_process(runtime.child.id());
            let _ = runtime.child.wait();
        }
        for (key, value) in self.env.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        agents::clear_resolve_cache();
    }
}

#[tokio::test]
async fn provisioned_destination_catalog_start_and_same_host_switch_use_shared_launch() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    let catalog = fixture.action(Action::Catalog, None).await;
    assert_eq!(catalog.outcome, Outcome::Ready);
    assert!(
        catalog
            .observation
            .unwrap()
            .catalog
            .unwrap()
            .entry
            .unwrap()
            .eligible
    );
    assert_eq!(
        fixture.action(Action::Preflight, None).await.outcome,
        Outcome::Ready
    );
    let started = fixture.action(Action::Start, None).await;
    assert_eq!(started.outcome, Outcome::Running);
    assert_eq!(
        started.observation.unwrap().running_configuration,
        Some(fixture.named.reference())
    );
    fixture.launched("fixture-model|fixture-model");
    let old_pid = fixture.running().unwrap().0;
    let status = fixture.action(Action::Status, None).await;
    let mut changed = fixture.named.clone();
    changed.model = "switched-model".into();
    fixture.named = save(
        &mut fixture.record,
        &fixture.owner.public_key().to_hex(),
        COMMUNITY,
        changed,
    );
    fixture.persist();
    let switched = fixture.action(Action::Restart, Some(status.id)).await;
    assert_eq!(switched.outcome, Outcome::Running);
    assert_ne!(fixture.running().unwrap().0, old_pid);
    assert_eq!(
        fixture.running().unwrap().1,
        Some(fixture.named.reference())
    );
    fixture.launched("switched-model|switched-model");
    assert_eq!(fixture.stop().await, StopOutcome::Stopped);
    assert!(fixture.running().is_none());
    // Deliberate same-host Stop → fresh Start consumes the new Stop fence,
    // unlike a stale Restart continuation. Keep the exact switched revision.
    assert_eq!(
        fixture.action(Action::Preflight, None).await.outcome,
        Outcome::Ready
    );
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::Running
    );
    assert_eq!(
        fixture.running().unwrap().1,
        Some(fixture.named.reference())
    );
    assert_eq!(fixture.stop().await, StopOutcome::Stopped);
}

#[tokio::test]
async fn unavailable_identity_or_runtime_excludes_catalog_and_refuses_start() {
    let _guard = agents::lock_path_mutex_async().await;
    for missing in ["absent", "wrong", "runtime"] {
        let mut fixture = Fixture::new();
        match missing {
            "absent" => fixture.record.private_key_nsec.clear(),
            "wrong" => {
                fixture.record.private_key_nsec =
                    nostr::Keys::generate().secret_key().to_secret_hex()
            }
            _ => {
                // Cache was warmed by catalog readiness. Revocation must still stat.
                assert!(agents::resolve_command(&fixture.record.acp_command).is_some());
                std::fs::remove_file(&fixture.record.acp_command).unwrap();
            }
        }
        fixture.persist();
        let catalog = fixture.action(Action::Catalog, None).await;
        assert_eq!(catalog.outcome, Outcome::Ready);
        assert!(
            !catalog
                .observation
                .unwrap()
                .catalog
                .unwrap()
                .entry
                .unwrap()
                .eligible
        );
        assert_eq!(
            fixture.action(Action::Start, None).await.outcome,
            Outcome::Ineligible
        );
        assert!(fixture.running().is_none());
        assert!(!fixture.temp.path().join("launches").exists());
    }
}

#[tokio::test]
async fn revoked_key_keeps_existing_process_visible_and_restart_refuses_without_teardown() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::Running
    );
    fixture.launched("fixture-model|fixture-model");
    let pid = fixture.running().unwrap().0;
    fixture.record.private_key_nsec.clear();
    fixture.persist();
    let status = fixture.action(Action::Status, None).await;
    assert_eq!(status.outcome, Outcome::Running);
    assert_eq!(
        fixture
            .action(Action::Restart, Some(status.id))
            .await
            .outcome,
        Outcome::Ineligible
    );
    assert_eq!(fixture.running().unwrap().0, pid);
    assert_eq!(fixture.stop().await, StopOutcome::Stopped);
    assert!(fixture.running().is_none());
}

#[tokio::test]
async fn key_loss_during_preflight_is_rechecked_for_start_and_catalog() {
    let _guard = agents::lock_path_mutex_async().await;
    for action in [Action::Start, Action::Catalog] {
        let fixture = Fixture::new();
        let result = fixture
            .receive(fixture.request(action, None), |_, _| {
                let mut revoked = fixture.record.clone();
                revoked.private_key_nsec.clear();
                agents::save_managed_agents(fixture.app.handle(), &[revoked]).unwrap();
                async { Ok(()) }
            })
            .await;
        if action == Action::Start {
            assert_eq!(result.outcome, Outcome::Ineligible);
        } else {
            assert!(
                !result
                    .observation
                    .unwrap()
                    .catalog
                    .unwrap()
                    .entry
                    .unwrap()
                    .eligible
            );
        }
        assert!(fixture.running().is_none());
        assert!(!fixture.temp.path().join("launches").exists());
    }
}

#[tokio::test]
async fn failed_destination_preflight_never_tears_down_existing_source() {
    let _guard = agents::lock_path_mutex_async().await;
    let fixture = Fixture::new();
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::Running
    );
    fixture.launched("fixture-model|fixture-model");
    let pid = fixture.running().unwrap().0;
    let result = fixture
        .receive(fixture.request(Action::Preflight, None), |_, _| async {
            Err("isolated provider unavailable".into())
        })
        .await;
    assert_eq!(result.outcome, Outcome::Ineligible);
    assert_eq!(fixture.running().unwrap().0, pid);
}

// The fake child changes only its temporary fixture files when ordinary Stop
// signals it. This binds revocation to the actual teardown boundary, after both
// async and locked admission have already accepted the old key/executable.
fn on_stop(fixture: &Fixture, effect: &str) {
    let script = std::fs::read_to_string(&fixture.record.acp_command).unwrap();
    let script = script
        .replacen(
            "#!/bin/sh\n",
            &format!("#!/bin/sh\ntrap '{}' TERM\n", effect),
            1,
        )
        .replace("exec /bin/sleep 20", "/bin/sleep 20 &\nwait");
    std::fs::write(&fixture.record.acp_command, script).unwrap();
}

fn assert_failed_without_child(fixture: &Fixture) -> ManagedAgentRecord {
    assert!(fixture.running().is_none());
    let saved = agents::storage::load_agent_store(fixture.app.handle())
        .unwrap()
        .into_iter()
        .find(|r| r.pubkey == fixture.record.pubkey)
        .unwrap();
    assert!(saved.runtime_pid.is_none());
    assert!(saved.last_stopped_at.is_some());
    assert!(saved.last_error.is_some());
    assert_eq!(
        std::fs::read_to_string(fixture.temp.path().join("launches"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    saved
}

#[tokio::test]
async fn post_stop_key_loss_or_executable_loss_persists_failed_without_restoring_credentials() {
    let _guard = agents::lock_path_mutex_async().await;
    for revoke_key in [true, false] {
        let fixture = Fixture::new();
        let store_path = agents::storage::managed_agents_store_path(fixture.app.handle()).unwrap();
        let revoked_path = fixture.temp.path().join("revoked.json");
        let mut revoked = fixture.record.clone();
        revoked.private_key_nsec.clear();
        std::fs::write(&revoked_path, serde_json::to_vec(&[revoked]).unwrap()).unwrap();
        let effect = if revoke_key {
            format!(
                "/bin/cp \"{}\" \"{}\"; exit 0",
                revoked_path.display(),
                store_path.display()
            )
        } else {
            format!("/bin/rm \"{}\"; exit 0", fixture.record.acp_command)
        };
        on_stop(&fixture, &effect);
        assert_eq!(
            fixture.action(Action::Start, None).await.outcome,
            Outcome::Running
        );
        fixture.launched("fixture-model|fixture-model");
        let status = fixture.action(Action::Status, None).await;
        let result = fixture.action(Action::Restart, Some(status.id)).await;
        assert_eq!(result.outcome, Outcome::Failed);
        let saved = assert_failed_without_child(&fixture);
        if revoke_key {
            assert!(saved.private_key_nsec.is_empty());
            assert!(!std::fs::read_to_string(store_path)
                .unwrap()
                .contains(&fixture.record.private_key_nsec));
            assert_eq!(
                fixture.action(Action::Start, None).await.outcome,
                Outcome::Ineligible
            );
        } else {
            assert_eq!(saved.private_key_nsec, fixture.record.private_key_nsec);
        }
    }
}

#[tokio::test]
async fn post_stop_expiry_persists_truthful_failed_without_second_spawn() {
    let _guard = agents::lock_path_mutex_async().await;
    let fixture = Fixture::new();
    // TERM is ignored only by this bounded synthetic process; ordinary Stop
    // takes its one-second grace then SIGKILL, crossing the captured deadline.
    on_stop(&fixture, "");
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::Running
    );
    fixture.launched("fixture-model|fixture-model");
    let mut plan = prepare_for_app(
        fixture.app.handle(),
        &fixture.record,
        Some(&fixture.named.reference()),
        &fixture.owner.public_key().to_hex(),
        COMMUNITY,
    )
    .unwrap();
    preflight_with(
        &mut plan,
        &fixture.owner.public_key().to_hex(),
        COMMUNITY,
        false,
        |_, _| async { Ok(()) },
    )
    .await
    .unwrap();
    let state = fixture.app.state::<crate::app_state::AppState>();
    let _transition = state.managed_agent_runtime_transition.lock().unwrap();
    plan.expire_at(Timestamp::now().as_secs() + 1);
    let status = agents::start_pair_captured_locked(
        fixture.record.pubkey.clone(),
        COMMUNITY.into(),
        true,
        None,
        None,
        &plan,
        false,
        true,
        None,
        fixture.app.handle().clone(),
    )
    .unwrap();
    assert_eq!(
        status.lifecycle,
        agents::ManagedAgentRuntimeLifecycle::Failed
    );
    assert!(status.error.unwrap().contains("expired"));
    let saved = assert_failed_without_child(&fixture);
    assert_eq!(saved.private_key_nsec, fixture.record.private_key_nsec);
}
