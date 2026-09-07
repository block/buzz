//! Production orchestration with only provider I/O, process sweeps and relay
//! publication replaced. No external providers or system agent enumeration.
use super::*;
use std::{cell::RefCell, os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
use tauri::Manager;

const ONE: &str = "wss://launch-one.example";
const TWO: &str = "wss://launch-two.example";

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    record: ManagedAgentRecord,
    owner: String,
    env: Vec<(&'static str, Option<std::ffi::OsString>)>,
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let env = ["HOME", "XDG_DATA_HOME", "PATH"]
            .map(|key| (key, std::env::var_os(key)))
            .to_vec();
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", temp.path().display()));
        agents::clear_resolve_cache();
        for name in ["buzz-agent", "buzz-dev-mcp", "buzz-acp"] {
            let path = temp.path().join(name);
            let script = if name == "buzz-acp" {
                format!("#!/bin/sh\nprintf '%s|%s|%s\\n' \"$BUZZ_ACP_MODEL\" \"$BUZZ_ACP_REQUIRED_MODEL\" \"$BUZZ_ACP_LAZY_POOL\" >> '{}'\n", temp.path().join("launches").display())
            } else {
                "#!/bin/sh\nexit 0\n".into()
            };
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<crate::app_state::AppState>();
        *state.relay_url_override.lock().unwrap() = Some(ONE.into());
        let mut record = record();
        let owner = attest(&mut record, &state.signing_keys().unwrap());
        record.acp_command = temp.path().join("buzz-acp").display().to_string();
        record.runtime = Some("buzz-agent".into());
        record.agent_command = "buzz-agent".into();
        record.model = Some("default-model".into());
        record.provider = Some("openai".into());
        record.start_on_app_launch = true;
        record
            .env_vars
            .insert("OPENAI_COMPAT_API_KEY".into(), "fixture-only".into());
        Self {
            app,
            record,
            owner,
            env,
            temp,
        }
    }
    fn named(&mut self, relay: &str, provider: &str, model: &str) -> RuntimeConfiguration {
        let host = local_host(self.app.handle(), &self.owner, relay).unwrap();
        let mut entry = config(&host);
        entry.provider = Some(provider.into());
        entry.model = model.into();
        save(&mut self.record, &self.owner, relay, entry)
    }
    fn persist(&self) {
        agents::save_managed_agents(self.app.handle(), std::slice::from_ref(&self.record)).unwrap();
    }
    fn finish_children(&self) -> Vec<(String, Option<RuntimeConfigurationRef>)> {
        let state = self.app.state::<crate::app_state::AppState>();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        runtimes
            .iter_mut()
            .map(|(key, runtime)| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                loop {
                    if let Some(status) = runtime.child.try_wait().unwrap() {
                        assert!(status.success());
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "fixture child timed out"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                (
                    key.relay_url.clone(),
                    runtime.spawn_config.runtime_configuration.clone(),
                )
            })
            .collect()
    }
    fn no_launch(&self) {
        assert!(self
            .app
            .state::<crate::app_state::AppState>()
            .managed_agent_processes
            .lock()
            .unwrap()
            .is_empty());
        assert!(!self.temp.path().join("launches").exists());
        assert!(agents::read_all_agent_runtime_receipts(self.app.handle()).is_empty());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only owned fixture children; never call system-wide cleanup helpers.
        for (_, mut runtime) in self
            .app
            .state::<crate::app_state::AppState>()
            .managed_agent_processes
            .lock()
            .unwrap()
            .drain()
        {
            let _ = runtime.child.kill();
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
async fn ordinary_default_start_cannot_launch_new_selection_after_preflight() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    fixture.persist();
    let app = fixture.app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();
    let pubkey = fixture.record.pubkey.clone();
    let (resume, wait) = tokio::sync::oneshot::channel();
    let future = crate::commands::start_local_agent_with_preflight_using(
        &app,
        &state,
        &pubkey,
        crate::commands::LocalStartIntent::Explicit,
        None,
        None,
        None,
        None,
        |model, _| async move {
            assert_eq!(model, None); // Default is openai, not the later mesh pick.
            wait.await.unwrap();
            Ok(())
        },
    );
    tokio::pin!(future);
    assert!(futures_util::poll!(&mut future).is_pending());
    fixture.named(ONE, "relay-mesh", "never-preflighted");
    fixture.persist();
    resume.send(()).unwrap();
    assert!(future
        .await
        .unwrap_err()
        .contains("Selected configuration changed"));
    fixture.no_launch();
}

#[tokio::test]
async fn restore_preflights_selected_provider_in_both_directions() {
    let _guard = agents::lock_path_mutex_async().await;
    for (default_provider, named_provider, expected) in [
        ("relay-mesh", "openai", None),
        ("openai", "relay-mesh", Some("selected-model")),
    ] {
        let mut fixture = Fixture::new();
        fixture.record.provider = Some(default_provider.into());
        let named = fixture.named(ONE, named_provider, "selected-model");
        fixture.persist();
        let calls = RefCell::new(Vec::new());
        let sweeps = RefCell::new(0);
        let published = RefCell::new(Vec::new());
        agents::restore_with(
            fixture.app.handle(),
            &AtomicBool::new(false),
            |model, _| {
                calls.borrow_mut().push(model.clone());
                async move {
                    assert_eq!(model.as_deref(), expected);
                    Ok(())
                }
            },
            |pids| {
                assert!(pids.is_empty());
                *sweeps.borrow_mut() += 1;
            },
            |key, _| published.borrow_mut().push(key),
        )
        .await
        .unwrap();
        assert_eq!(*calls.borrow(), vec![expected.map(str::to_owned)]);
        assert_eq!(*sweeps.borrow(), 1);
        assert_eq!(*published.borrow(), vec![fixture.record.pubkey.clone()]);
        assert_eq!(
            fixture.finish_children(),
            vec![(ONE.into(), Some(named.reference()))]
        );
        assert!(
            std::fs::read_to_string(fixture.temp.path().join("launches"))
                .unwrap()
                .starts_with("selected-model|selected-model|")
        );
    }
}

#[tokio::test]
async fn restore_refuses_provider_preflight_failure_before_spawn() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    fixture.named(ONE, "relay-mesh", "offline-model");
    fixture.persist();
    agents::restore_with(
        fixture.app.handle(),
        &AtomicBool::new(false),
        |model, _| async move {
            assert_eq!(model.as_deref(), Some("offline-model"));
            Err("fixture peer offline".into())
        },
        |_| {},
        |_, _| panic!("failed restore must not publish"),
    )
    .await
    .unwrap();
    fixture.no_launch();
    assert_eq!(
        agents::load_managed_agents(fixture.app.handle()).unwrap()[0]
            .last_error
            .as_deref(),
        Some("fixture peer offline")
    );
}

#[tokio::test]
async fn bulk_restart_preflights_every_captured_community_and_revalidates_before_spawn() {
    let _guard = agents::lock_path_mutex_async().await;
    // The unchanged case proves a real lazy pair spawn and receipts. The other
    // cases edit while the first provider awaits, after BOTH plans were captured.
    for mutation in ["unchanged", "revision", "workspace", "selection"] {
        let mut fixture = Fixture::new();
        let first = fixture.named(ONE, "relay-mesh", "mesh-one");
        let second = fixture.named(TWO, "relay-mesh", "mesh-two");
        let workspace = fixture.temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut entry = second.clone();
        entry.workspace = Some(workspace.display().to_string());
        let second = save(&mut fixture.record, &fixture.owner, TWO, entry);
        fixture.record.last_stopped_at = Some("prior security restart Stop".into());
        fixture.persist();
        let calls = RefCell::new(Vec::new());
        let (resume, wait) = tokio::sync::oneshot::channel();
        let wait = RefCell::new(Some(wait));
        let app = fixture.app.handle().clone();
        let state = app.state::<crate::app_state::AppState>();
        let pubkey = fixture.record.pubkey.clone();
        let relays = vec![ONE.into(), TWO.into()];
        let future = crate::commands::start_local_agent_pairs_with_preflight_using(
            &app,
            &state,
            &pubkey,
            &relays,
            |model, _| {
                calls.borrow_mut().push(model);
                let wait = wait.borrow_mut().take();
                async move {
                    if let Some(wait) = wait {
                        wait.await.unwrap();
                    }
                    Ok(())
                }
            },
        );
        tokio::pin!(future);
        assert!(futures_util::poll!(&mut future).is_pending());
        match mutation {
            "revision" => {
                let mut edit = second.clone();
                edit.model = "unpreflighted-edit".into();
                save(&mut fixture.record, &fixture.owner, TWO, edit);
                fixture.persist();
            }
            "workspace" => std::fs::remove_dir(&workspace).unwrap(),
            "selection" => {
                fixture
                    .record
                    .runtime_configurations
                    .replace(
                        &fixture.owner,
                        TWO,
                        &second.host,
                        RuntimeConfigurations {
                            selected: None,
                            entries: vec![second.clone()],
                        },
                    )
                    .unwrap();
                fixture.persist();
            }
            _ => {}
        }
        resume.send(()).unwrap();
        let result = future.await;
        assert_eq!(
            *calls.borrow(),
            vec![Some("mesh-one".into()), Some("mesh-two".into())]
        );
        let mut launches = fixture.finish_children();
        launches.sort_by(|a, b| a.0.cmp(&b.0));
        if mutation == "unchanged" {
            result.unwrap();
            assert_eq!(
                launches,
                vec![
                    (ONE.into(), Some(first.reference())),
                    (TWO.into(), Some(second.reference()))
                ]
            );
        } else {
            assert!(result.is_err());
            assert_eq!(launches, vec![(ONE.into(), Some(first.reference()))]);
        }
        let output = std::fs::read_to_string(fixture.temp.path().join("launches")).unwrap();
        assert!(output.lines().all(|line| line.ends_with("|true")));
        assert!(!output.contains("unpreflighted-edit"));
        assert_eq!(
            agents::read_all_agent_runtime_receipts(fixture.app.handle()).len(),
            launches.len()
        );
    }
}

#[tokio::test]
async fn restore_selection_fence_survives_suspension_in_both_directions() {
    let _guard = agents::lock_path_mutex_async().await;
    for starts_named in [false, true] {
        let mut fixture = Fixture::new();
        let named = fixture.named(ONE, "relay-mesh", "named-model");
        if !starts_named {
            fixture
                .record
                .runtime_configurations
                .replace(
                    &fixture.owner,
                    ONE,
                    &named.host,
                    RuntimeConfigurations {
                        selected: None,
                        entries: vec![named.clone()],
                    },
                )
                .unwrap();
        }
        fixture.persist();
        let app = fixture.app.handle().clone();
        let shutdown = AtomicBool::new(false);
        let (resume, wait) = tokio::sync::oneshot::channel();
        let wait = RefCell::new(Some(wait));
        let future = agents::restore_with(
            &app,
            &shutdown,
            |model, _| {
                assert_eq!(model.as_deref(), starts_named.then_some("named-model"));
                let wait = wait.borrow_mut().take().unwrap();
                async move {
                    wait.await.unwrap();
                    Ok(())
                }
            },
            |_| {},
            |_, _| panic!("stale restore must not publish"),
        );
        tokio::pin!(future);
        assert!(futures_util::poll!(&mut future).is_pending());
        fixture
            .record
            .runtime_configurations
            .replace(
                &fixture.owner,
                ONE,
                &named.host,
                RuntimeConfigurations {
                    selected: (!starts_named).then(|| named.id.clone()),
                    entries: vec![named.clone()],
                },
            )
            .unwrap();
        fixture.persist();
        resume.send(()).unwrap();
        future.await.unwrap();
        fixture.no_launch();
        assert!(agents::load_managed_agents(&app).unwrap()[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains("Selected configuration changed"));
    }
}

#[tokio::test]
async fn ordinary_default_revalidates_effective_inputs_not_only_selection() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    fixture.persist();
    let app = fixture.app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();
    let pubkey = fixture.record.pubkey.clone();
    let (resume, wait) = tokio::sync::oneshot::channel();
    let future = crate::commands::start_local_agent_with_preflight_using(
        &app,
        &state,
        &pubkey,
        crate::commands::LocalStartIntent::Automatic,
        None,
        None,
        None,
        None,
        |model, _| async move {
            assert_eq!(model, None);
            wait.await.unwrap();
            Ok(())
        },
    );
    tokio::pin!(future);
    assert!(futures_util::poll!(&mut future).is_pending());
    fixture.record.provider = Some("relay-mesh".into());
    fixture.record.model = Some("unpreflighted-default-edit".into());
    fixture.persist();
    resume.send(()).unwrap();
    assert!(future
        .await
        .unwrap_err()
        .contains("Agent changed during runtime preflight"));
    fixture.no_launch();
}

impl Fixture {
    fn hold_children(&self) {
        let path = self.temp.path().join("buzz-acp");
        let mut script = std::fs::read_to_string(&path).unwrap();
        // exec means fixture cleanup owns the only remaining PID; no orphan sleep.
        script.push_str("exec /bin/sleep 30\n");
        std::fs::write(path, script).unwrap();
    }
    fn running(&self, relay: &str) -> (u32, String, Option<RuntimeConfigurationRef>) {
        let state = self.app.state::<crate::app_state::AppState>();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        let key = agents::ManagedAgentRuntimeKey::new(&self.record.pubkey, relay).unwrap();
        let runtime = runtimes.get_mut(&key).unwrap();
        assert!(runtime.child.try_wait().unwrap().is_none());
        (
            runtime.child.id(),
            runtime.start_nonce.clone(),
            runtime.spawn_config.runtime_configuration.clone(),
        )
    }
}

#[tokio::test]
async fn direct_restart_preflights_target_and_preserves_old_child_on_refusal() {
    let _guard = agents::lock_path_mutex_async().await;
    for relay in [ONE, TWO] {
        for mutation in [
            "unchanged",
            "revision",
            "workspace",
            "offline",
            "generation",
        ] {
            let mut fixture = Fixture::new();
            fixture.hold_children();
            let old = fixture.named(relay, "openai", "old-model");
            fixture.persist();
            agents::start_pair_with_preflight(
                fixture.record.pubkey.clone(),
                relay.into(),
                None,
                true,
                false,
                fixture.app.handle().clone(),
                |model, _| async move {
                    assert_eq!(model, None);
                    Ok(())
                },
            )
            .await
            .unwrap();
            let old_runtime = fixture.running(relay);
            assert_eq!(old_runtime.2, Some(old.reference()));
            let workspace = fixture.temp.path().join("target-workspace");
            std::fs::create_dir(&workspace).unwrap();
            let mut target = fixture.named(relay, "relay-mesh", "target-model");
            target.workspace = Some(workspace.display().to_string());
            let target = save(&mut fixture.record, &fixture.owner, relay, target);
            fixture.persist();
            let (resume, wait) = tokio::sync::oneshot::channel();
            let future = agents::start_pair_with_preflight(
                fixture.record.pubkey.clone(),
                relay.into(),
                None,
                false,
                true,
                fixture.app.handle().clone(),
                |model, _| async move {
                    assert_eq!(model.as_deref(), Some("target-model"));
                    wait.await.unwrap();
                    if mutation == "offline" {
                        Err("fixture provider unavailable".into())
                    } else {
                        Ok(())
                    }
                },
            );
            tokio::pin!(future);
            assert!(futures_util::poll!(&mut future).is_pending());
            assert_eq!(fixture.running(relay), old_runtime);
            let mut replacement = None;
            match mutation {
                "revision" => {
                    let mut edited = target.clone();
                    edited.model = "not-preflighted".into();
                    save(&mut fixture.record, &fixture.owner, relay, edited);
                    fixture.persist();
                }
                "workspace" => std::fs::remove_dir(&workspace).unwrap(),
                "generation" => {
                    agents::start_pair_with_preflight(
                        fixture.record.pubkey.clone(),
                        relay.into(),
                        None,
                        false,
                        true,
                        fixture.app.handle().clone(),
                        |_, _| async { Ok(()) },
                    )
                    .await
                    .unwrap();
                    replacement = Some(fixture.running(relay));
                }

                _ => {}
            }
            resume.send(()).unwrap();
            let result = future.await;
            if mutation == "unchanged" {
                result.unwrap();
                let new_runtime = fixture.running(relay);
                assert_ne!(new_runtime.1, old_runtime.1);
                assert_eq!(new_runtime.2, Some(target.reference()));
            } else {
                assert!(result.is_err());
                assert_eq!(fixture.running(relay), replacement.unwrap_or(old_runtime));
            }
        }
    }
}

#[tokio::test]
async fn direct_start_and_reconcile_continuation_refuse_selection_change_and_stop() {
    let _guard = agents::lock_path_mutex_async().await;
    for explicit in [true, false] {
        for mutation in ["selection", "stop"] {
            let mut fixture = Fixture::new();
            fixture.persist();
            let (resume, wait) = tokio::sync::oneshot::channel();
            let future = agents::start_pair_with_preflight(
                fixture.record.pubkey.clone(),
                ONE.into(),
                None,
                explicit,
                false,
                fixture.app.handle().clone(),
                |model, _| async move {
                    assert_eq!(model, None);
                    wait.await.unwrap();
                    Ok(())
                },
            );
            tokio::pin!(future);
            assert!(futures_util::poll!(&mut future).is_pending());
            if mutation == "selection" {
                fixture.named(ONE, "relay-mesh", "not-preflighted");
                fixture.persist();
            } else {
                let state = fixture.app.state::<crate::app_state::AppState>();
                let _transition = state.managed_agent_runtime_transition.lock().unwrap();
                agents::stop_pair_locked(
                    fixture.record.pubkey.clone(),
                    ONE.into(),
                    fixture.app.handle().clone(),
                )
                .unwrap();
            }
            resume.send(()).unwrap();
            assert!(future.await.is_err());
            fixture.no_launch();
        }
    }
}

#[cfg(feature = "mesh-llm")]
#[tokio::test]
async fn recovery_uses_running_pair_snapshots_not_default_or_next_selection() {
    let _guard = agents::lock_path_mutex_async().await;
    for default_mesh in [false, true] {
        let mut fixture = Fixture::new();
        fixture.hold_children();
        fixture.record.provider = Some(if default_mesh { "relay-mesh" } else { "openai" }.into());
        fixture.named(ONE, "relay-mesh", "running-one");
        fixture.named(
            TWO,
            if default_mesh { "openai" } else { "relay-mesh" },
            "running-two",
        );
        fixture.persist();
        for relay in [ONE, TWO] {
            agents::start_pair_with_preflight(
                fixture.record.pubkey.clone(),
                relay.into(),
                None,
                false,
                false,
                fixture.app.handle().clone(),
                |_, _| async { Ok(()) },
            )
            .await
            .unwrap();
        }
        let app = fixture.app.handle().clone();
        let state = app.state::<crate::app_state::AppState>();
        let mut before = crate::mesh_llm::running_mesh_consumers(&state);
        before.sort_by(|a, b| a.0.relay_url.cmp(&b.0.relay_url));
        let expected = if default_mesh {
            vec![(ONE, "running-one")]
        } else {
            vec![(ONE, "running-one"), (TWO, "running-two")]
        };
        assert_eq!(
            before
                .iter()
                .map(|(key, _, model)| (key.relay_url.as_str(), model.as_str()))
                .collect::<Vec<_>>(),
            expected
        );
        // Both record Default and both next-launch selections change. Neither
        // can rewrite the provider/model requirement of a running generation.
        fixture.record.provider = Some("openai".into());
        fixture.named(ONE, "openai", "next-one");
        fixture.named(TWO, "relay-mesh", "next-two");
        fixture.persist();
        let mut after = crate::mesh_llm::running_mesh_consumers(&state);
        after.sort_by(|a, b| a.0.relay_url.cmp(&b.0.relay_url));
        assert_eq!(before, after);
    }
}

#[tokio::test]
async fn reconcile_keeps_authorization_inputs_without_cross_pair_timestamp_failure() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    fixture.hold_children();
    fixture.named(ONE, "relay-mesh", "mesh-one");
    fixture.named(TWO, "relay-mesh", "mesh-two");
    fixture.record.last_stopped_at = Some("prior-stop".into());
    fixture.persist();
    let probed_record = fixture.record.clone();
    for (relay, model) in [(ONE, "mesh-one"), (TWO, "mesh-two")] {
        agents::start_pair_with_preflight(
            fixture.record.pubkey.clone(),
            relay.into(),
            Some(&probed_record),
            false,
            false,
            fixture.app.handle().clone(),
            |actual, _| async move {
                assert_eq!(actual.as_deref(), Some(model));
                Ok(())
            },
        )
        .await
        .unwrap();
    }
    assert_ne!(fixture.running(ONE).1, fixture.running(TWO).1);
}

#[tokio::test]
async fn bulk_stop_during_preflight_cannot_be_resumed_by_automatic_start() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    fixture.named(ONE, "relay-mesh", "mesh-one");
    fixture.named(TWO, "relay-mesh", "mesh-two");
    fixture.persist();
    let app = fixture.app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();
    let relays = vec![ONE.into(), TWO.into()];
    let (resume, wait) = tokio::sync::oneshot::channel();
    let wait = RefCell::new(Some(wait));
    let future = crate::commands::start_local_agent_pairs_with_preflight_using(
        &app,
        &state,
        &fixture.record.pubkey,
        &relays,
        |_, _| {
            let wait = wait.borrow_mut().take();
            async move {
                if let Some(wait) = wait {
                    wait.await.unwrap();
                }
                Ok(())
            }
        },
    );
    tokio::pin!(future);
    assert!(futures_util::poll!(&mut future).is_pending());
    {
        let _transition = state.managed_agent_runtime_transition.lock().unwrap();
        agents::stop_pair_locked(fixture.record.pubkey.clone(), TWO.into(), app.clone()).unwrap();
    }
    resume.send(()).unwrap();
    assert!(future.await.unwrap_err().contains("Stop interrupted"));
    fixture.no_launch();
}
