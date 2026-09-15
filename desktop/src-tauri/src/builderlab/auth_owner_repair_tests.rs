//! Actual registered managed-agent message command, real store + loopback HTTP.
use super::*;
use crate::managed_agents::{load_managed_agents, save_managed_agents, ManagedAgentRecord};
use nostr::ToBech32;
use tauri::Manager;

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    api: MockApi,
    agent: Keys,
    old_home: Option<std::ffi::OsString>,
    old_xdg: Option<std::ffi::OsString>,
    _temp: tempfile::TempDir,
    _environment: std::sync::MutexGuard<'static, ()>,
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
impl Fixture {
    async fn new(stored: bool) -> Self {
        let api = MockApi::new().await;
        let state = build_app_state_for_mode(SignerMode::Remote);
        *state.relay_url_override.lock().unwrap() = Some(api.base.to_string());
        api.login(&state.native_auth, "A").await.unwrap();
        let environment = crate::managed_agents::lock_path_mutex();
        // This production OnceLock is process-wide; do not let our temporary
        // HOME become the cached working directory of concurrent provider tests.
        crate::managed_agents::default_agent_workdir();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let agent = Keys::generate();
        let auth = stored.then(|| {
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&api.state.keys, &agent.public_key(), "")
                .unwrap()
        });
        let record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey":agent.public_key().to_hex(), "private_key_nsec":agent.secret_key().to_bech32().unwrap(),
            "name":"Repair", "auth_tag":auth,
            "relay_url":"", "acp_command":"", "agent_command":"", "agent_args":[],
            "mcp_command":"", "turn_timeout_seconds":0,
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
        })).unwrap();
        save_managed_agents(app.handle(), &[record]).unwrap();
        Self {
            app,
            api,
            agent,
            old_home,
            old_xdg,
            _temp: temp,
            _environment: environment,
        }
    }
    fn spawn(
        &self,
    ) -> tokio::task::JoinHandle<Result<crate::models::SendChannelMessageResponse, String>> {
        let app = self.app.handle().clone();
        let agent = self.agent.public_key().to_hex();
        tokio::spawn(async move {
            crate::commands::send_managed_agent_channel_message(
                agent,
                uuid::Uuid::new_v4().to_string(),
                "repair message".into(),
                None,
                None,
                None,
                None,
                None,
                app.clone(),
                app.state(),
            )
            .await
        })
    }
    fn records(&self) -> serde_json::Value {
        serde_json::to_value(load_managed_agents(self.app.handle()).unwrap()).unwrap()
    }
}

#[tokio::test]
async fn actual_message_repair_publishes_agent_event_with_captured_owner_proof() {
    for stored in [false, true] {
        let f = Fixture::new(stored).await;
        let before = f.records();
        if stored {
            // An independently keyed agent with its existing proof does not
            // need a current human login and must not trigger owner crypto.
            f.app
                .state::<crate::AppState>()
                .native_auth
                .clear()
                .unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(5), f.spawn())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let published = f.api.state.agent_publications.lock().unwrap();
        assert_eq!(published.len(), 1);
        let (event, proof) = &published[0];
        assert_eq!(event.id.to_hex(), result.event_id);
        assert_eq!(event.pubkey, f.agent.public_key());
        assert_ne!(event.pubkey, f.api.state.keys.public_key());
        assert_eq!(
            buzz_sdk_pkg::nip_oa::verify_auth_tag(proof, &event.pubkey).unwrap(),
            f.api.state.keys.public_key()
        );
        assert_eq!(before, f.records(), "repair is not a record rewrite");
        let requests = f.api.state.requests.lock().unwrap();
        let oa: Vec<_> = requests.iter().filter(|r| r.0 == OA).collect();
        assert_eq!(oa.len(), usize::from(!stored));
        if !stored {
            assert_eq!(oa[0].1.as_deref(), Some("credential-A"));
            assert_eq!(
                oa[0].2,
                serde_json::json!({"agent_pubkey":f.agent.public_key().to_hex(), "conditions":""})
            );
        }
        assert!(!requests.iter().any(|r| r.0 == "v1/buzz/identity/sign"));
    }
}

#[tokio::test]
async fn actual_message_repair_failure_or_stale_owner_never_publishes_or_changes_record() {
    for action in [
        "owner",
        "agent",
        "conditions",
        "signature",
        "logout",
        "replace",
        "cancel",
        "relay",
        "expiry",
    ] {
        let f = Fixture::new(false).await;
        if action == "expiry" {
            *f.api.state.expiry.lock().unwrap() =
                (Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339();
            f.api
                .login(&f.app.state::<crate::AppState>().native_auth, "A")
                .await
                .unwrap();
        }
        let before = f.records();
        f.api.hold(OA);
        let task = f.spawn();
        f.api.arrived().await;
        assert!(f
            .app
            .state::<crate::AppState>()
            .managed_agents_store_lock
            .try_lock()
            .is_ok());
        assert_eq!(before, f.records());
        match action {
            "owner" | "agent" | "conditions" | "signature" => {
                *f.api.state.oa_fault.lock().unwrap() = Some(action);
                f.api.release();
            }
            "logout" => f
                .app
                .state::<crate::AppState>()
                .native_auth
                .clear()
                .unwrap(),
            "replace" => {
                f.api
                    .login(&f.app.state::<crate::AppState>().native_auth, "B")
                    .await
                    .unwrap();
            }
            "cancel" => task.abort(),
            "relay" => {
                *f.app
                    .state::<crate::AppState>()
                    .relay_url_override
                    .lock()
                    .unwrap() = Some("http://127.0.0.1:9".into());
                f.api.release();
            }
            "expiry" => tokio::time::sleep(Duration::from_millis(750)).await,
            _ => unreachable!(),
        }
        let outcome = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap();
        if action == "cancel" {
            assert!(matches!(outcome, Err(error) if error.is_cancelled()));
        } else {
            assert!(outcome.unwrap().is_err(), "{action}");
        }
        f.api.release();
        assert_eq!(before, f.records());
        assert!(f.api.state.agent_publications.lock().unwrap().is_empty());
        assert!(f
            .api
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.0 != "events" && r.0 != "v1/buzz/identity/sign"));
        if action == "replace" {
            f.app
                .state::<crate::AppState>()
                .active_signer()
                .unwrap()
                .check_valid()
                .unwrap();
        }
    }
}

#[path = "auth_owner_corrections_tests.rs"]
mod corrections;
