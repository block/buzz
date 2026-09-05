#[cfg(unix)]
#[test]
fn tracked_local_child_stays_visible_and_stoppable_after_backend_migration() {
    use super::*;
    use crate::managed_agents::{
        private_config_overlay::{test_relay_payload, PrivateConfigOverlay},
        ManagedAgentPairRuntime, ManagedAgentProcess, ManagedAgentRuntimeKey,
    };
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;

    let _env_lock = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    struct EnvGuard(&'static str, Option<std::ffi::OsString>);
    impl EnvGuard {
        fn set(name: &'static str, value: &std::path::Path) -> Self {
            let prior = std::env::var_os(name);
            std::env::set_var(name, value);
            Self(name, prior)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.1.take() {
                Some(value) => std::env::set_var(self.0, value),
                None => std::env::remove_var(self.0),
            }
        }
    }
    let _home = EnvGuard::set("HOME", temp.path());
    let _xdg = EnvGuard::set("XDG_DATA_HOME", temp.path());
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    // Always reap exactly the children this test registered, including on panic.
    struct Children<'a>(&'a AppState);
    impl Drop for Children<'_> {
        fn drop(&mut self) {
            let mut runtimes = self
                .0
                .managed_agent_processes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for (_, mut runtime) in runtimes.drain() {
                let _ = runtime.child.kill();
                let _ = runtime.child.wait();
            }
        }
    }
    let _children = Children(&state);
    let pubkey = "ab".repeat(32);
    let mut overlay = PrivateConfigOverlay::default();
    overlay.insert(test_relay_payload(&pubkey)).unwrap();
    let mut record = overlay.materialize_relay_only_record(&pubkey, &[]).unwrap();
    record.backend = BackendKind::Provider {
        id: "test-provider".into(),
        config: serde_json::json!({}),
    };
    record.backend_agent_id = Some("remote-resource".into());
    save_managed_agents(app.handle(), &[record.clone()]).unwrap();
    let here = crate::managed_agents::workspace_pair_key(app.handle(), &record).unwrap();
    let elsewhere = ManagedAgentRuntimeKey::new(&pubkey, "wss://other.example").unwrap();
    assert_ne!(here, elsewhere);
    let spawn = |key: &ManagedAgentRuntimeKey| {
        let child = Command::new("/bin/sleep")
            .arg("60")
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        ManagedAgentPairRuntime::starting(ManagedAgentProcess {
            child,
            log_path: temp.path().join(format!("{}.log", key.runtime_id())),
            spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
                &record,
                &[],
                &[],
                &key.relay_url,
                &Default::default(),
                false,
                crate::managed_agents::AcpSessionPolicy::Channel,
            ),
            setup_mode: false,
            adapter_availability: None,
            start_nonce: key.runtime_id(),
        })
    };
    {
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        runtimes.insert(here.clone(), spawn(&here));
        runtimes.insert(elsewhere.clone(), spawn(&elsewhere));
        let summary = build_managed_agent_summary(
            app.handle(),
            &record,
            &runtimes,
            &[],
            &[],
            &Default::default(),
        )
        .unwrap();
        assert_eq!(summary.status, "running");
        assert_eq!(summary.pid, Some(runtimes[&here].child.id()));
        assert_eq!(
            summary.log_path,
            runtimes[&here].log_path.display().to_string()
        );
    }
    // Authority is intentionally unavailable. Stop still owns this local child.
    assert!(!state.managed_agent_authority_ready.load(Ordering::Acquire));
    stop_managed_agent_blocking(&pubkey, app.handle()).unwrap();
    {
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        assert!(!runtimes.contains_key(&here));
        assert!(runtimes
            .get_mut(&elsewhere)
            .unwrap()
            .child
            .try_wait()
            .unwrap()
            .is_none());
        let summary = build_managed_agent_summary(
            app.handle(),
            &record,
            &runtimes,
            &[],
            &[],
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            summary.status, "deployed",
            "another community's child is not local status"
        );
        assert_eq!(summary.pid, None);
    }
    let saved = load_managed_agents(app.handle()).unwrap();
    assert!(saved[0].last_stopped_at.is_some());
    assert_eq!(
        saved[0].backend_agent_id.as_deref(),
        Some("remote-resource")
    );
    // No owned child here: normal authority and remote-lifecycle refusals apply.
    assert!(stop_managed_agent_blocking(&pubkey, app.handle())
        .unwrap_err()
        .contains("authority is unavailable"));
    state
        .managed_agent_authority_ready
        .store(true, Ordering::Release);
    assert!(stop_managed_agent_blocking(&pubkey, app.handle())
        .unwrap_err()
        .contains("remote agents are stopped via !shutdown"));
}
