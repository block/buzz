//! Command-level tests for the observer-frame runtime-capability seam (P23-C1).
//!
//! Included via `#[path = "agent_config_capability_tests.rs"] mod ...;` at the
//! bottom of `agent_config.rs`, so `use super::*` reaches `put_agent_session_config`
//! and `get_agent_config_surface_for`.
//!
//! These drive the real command entry points against a `tauri::test::MockRuntime`
//! app with a committed workspace scope and a seeded runtime, proving the
//! capability contract end to end: a `session_config_captured` frame mutates a
//! runtime's `session_config` ONLY when its `{pubkey, relay_url, start_nonce}`
//! resolves to a still-live runtime whose `scope_id` equals the current active
//! scope, and `get_agent_config_surface` reads that cache back only through the
//! same still-current runtime. Because the cache lives on the runtime entry, a
//! frame that misses the gate mutates nothing, and runtime removal destroys the
//! embedded cache with no separate clear step.

use super::*;
use crate::managed_agents::{
    scope::{current_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK},
    ManagedAgentPairRuntime, ManagedAgentProcess, ManagedAgentRuntimeKey,
};

const TEST_RELAY: &str = "ws://localhost:3000";

/// Long-lived child so `try_wait()` reports the process as still running for
/// the duration of a test (avoids sync eviction / the dead-process reject).
fn spawn_live_child() -> std::process::Child {
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .args(["-c", "while true; do sleep 1; done"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived test child (sh loop)")
    }
    #[cfg(windows)]
    {
        std::process::Command::new("ping")
            .args(["-n", "100000", "127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived test child (ping)")
    }
}

/// An immediately-exited child so `try_wait()` reports `Some(status)` — the
/// dead-process reject arm of the capability gate.
fn spawn_dead_child() -> std::process::Child {
    #[cfg(not(windows))]
    let program = "/usr/bin/true";
    #[cfg(windows)]
    let program = "cmd";
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    cmd.args(["/C", "exit", "0"]);
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn short-lived test child");
    child.wait().expect("reap short-lived test child");
    child
}

fn test_record(pubkey: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "capability-test",
            "relay_url": "{TEST_RELAY}",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 300,
            "system_prompt": "",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#,
    ))
    .unwrap()
}

fn make_process(record: &ManagedAgentRecord, start_nonce: &str) -> ManagedAgentProcess {
    ManagedAgentProcess {
        child: spawn_live_child(),
        log_path: std::path::PathBuf::new(),
        spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
            record,
            &[],
            &[],
            TEST_RELAY,
            &Default::default(),
            false,
        ),
        setup_mode: false,
        adapter_availability: None,
        start_nonce: start_nonce.to_string(),
        #[cfg(windows)]
        job: None,
    }
}

/// A `session_config_captured` observer payload carrying `startNonce` +
/// `relayUrl` (the pair identity the harness attaches) plus a model so the
/// surfaced config is assertable. `model_overridden` makes the ACP model the
/// live winner over the record's structured model, so its presence is a direct
/// signal the cache was consumed.
fn frame(start_nonce: &str, model_id: &str) -> serde_json::Value {
    serde_json::json!({
        "startNonce": start_nonce,
        "relayUrl": TEST_RELAY,
        "modelOverridden": true,
        "models": {
            "currentModelId": model_id,
            "availableModels": [{ "modelId": model_id, "name": model_id }],
        },
    })
}

struct Harness {
    _tmp: tempfile::TempDir,
    app: tauri::App<tauri::test::MockRuntime>,
    scope_id: String,
}

impl Harness {
    /// Build a mock app, write the record to the tmp store, and commit an
    /// active scope pointing at it. Holds no runtime yet.
    fn new(record: &ManagedAgentRecord) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        crate::managed_agents::save_managed_agents_at(tmp.path(), std::slice::from_ref(record))
            .unwrap();
        std::fs::write(tmp.path().join("personas.json"), b"[]").unwrap();
        std::fs::write(tmp.path().join("global-agent-config.json"), b"{}").unwrap();

        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("failed to build mock app");

        // The surface's runtime-key resolution goes through the active
        // workspace relay (`relay_ws_url_with_override`), matching production
        // where the harness attaches that same relay to the frame. Pin it to
        // TEST_RELAY so the read key equals the seeded runtime key.
        {
            use tauri::Manager;
            *app.state::<crate::app_state::AppState>()
                .relay_url_override
                .lock()
                .unwrap() = Some(TEST_RELAY.to_string());
        }

        let scope_id = "scope-a".to_string();
        Self::commit_scope(&app, &scope_id, tmp.path());
        Self {
            _tmp: tmp,
            app,
            scope_id,
        }
    }

    fn commit_scope(
        app: &tauri::App<tauri::test::MockRuntime>,
        scope_id: &str,
        definitions_dir: &std::path::Path,
    ) {
        use tauri::Manager;
        let scope = WorkspaceAgentScope {
            scope_id: scope_id.to_string(),
            relay_url: TEST_RELAY.to_string(),
            owner_pubkey: "aa".repeat(32),
            definitions_dir: definitions_dir.to_path_buf(),
            generation: current_scope_generation(),
        };
        app.state::<crate::app_state::AppState>()
            .commit_active_scope(scope);
    }

    /// Re-commit the active scope under a new `scope_id` pointing at the same
    /// store — the shape of an A→B same-relay/different-owner switch as far as
    /// the capability gate sees it (the gate compares `scope_id`).
    fn switch_scope_to(&self, scope_id: &str) {
        Self::commit_scope(&self.app, scope_id, self._tmp.path());
    }

    fn state(&self) -> tauri::State<'_, crate::app_state::AppState> {
        use tauri::Manager;
        self.app.state::<crate::app_state::AppState>()
    }

    /// Insert a live runtime for `record` keyed on the current relay, stamped
    /// with `scope_id` and `start_nonce`.
    fn seed_runtime(&self, record: &ManagedAgentRecord, scope_id: &str, start_nonce: &str) {
        let key = ManagedAgentRuntimeKey::new(&record.pubkey, TEST_RELAY).unwrap();
        let process = make_process(record, start_nonce);
        let state = self.state();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        runtimes.insert(
            key,
            ManagedAgentPairRuntime::starting(process, Some(scope_id.to_string())),
        );
    }

    fn surface(&self, pubkey: &str) -> RuntimeConfigSurface {
        get_agent_config_surface_for(pubkey.to_string(), self.app.handle(), &self.state())
            .expect("surface must resolve")
    }

    /// White-box: does the tracked runtime hold a cached session config?
    fn runtime_has_cache(&self, pubkey: &str) -> bool {
        let key = ManagedAgentRuntimeKey::new(pubkey, TEST_RELAY).unwrap();
        self.state()
            .managed_agent_processes
            .lock()
            .unwrap()
            .get(&key)
            .map(|rt| rt.session_config.is_some())
            .unwrap_or(false)
    }

    /// Kill and reap every seeded child so no OS process leaks past the test.
    fn reap(&self) {
        let state = self.state();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        for (_, rt) in runtimes.iter_mut() {
            let _ = rt.child.kill();
            let _ = rt.child.wait();
        }
        runtimes.clear();
    }
}

/// Guard: serialise against every other test that reads/writes the global scope
/// generation, and against the shared process map, since these tests commit
/// scopes and seed runtimes on the same static-free but shared `AppState` shape.
fn gen_guard() -> std::sync::MutexGuard<'static, ()> {
    SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Positive control (no switch): a frame whose `{pubkey, relay, start_nonce}`
/// matches a live same-scope runtime is cached, and the surface serves it.
#[test]
fn matching_frame_on_live_runtime_is_cached_and_served() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    h.seed_runtime(&record, &h.scope_id, "nonce-1");

    assert!(h.surface(&pubkey).is_pre_spawn, "no frame yet ⇒ pre-spawn");

    put_agent_session_config(pubkey.clone(), frame("nonce-1", "model-x"), h.state());

    assert!(
        h.runtime_has_cache(&pubkey),
        "matching frame must be cached"
    );
    let surface = h.surface(&pubkey);
    assert!(!surface.is_pre_spawn, "cached frame ⇒ post-spawn surface");
    assert_eq!(
        surface.normalized.model.and_then(|m| m.value).as_deref(),
        Some("model-x"),
        "surface must serve the cached model"
    );
    h.reap();
}

/// A→B same-relay/different-owner switch: A's valid frame arrives after the
/// workspace has switched to B (same pubkey/relay, different scope). The frame
/// is rejected at the scope check — no mutation — and B's surface shows no A
/// value until B's own runtime, under B's nonce, emits its own frame.
#[test]
fn frame_after_scope_switch_is_rejected_until_new_runtime_emits() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);

    // A is live under scope-a; the workspace then switches to B (scope-b), and
    // A's runtime is drained (removed) as part of the transition.
    h.seed_runtime(&record, &h.scope_id, "nonce-a");
    h.reap(); // drain A: its runtime entry (and any cache) is destroyed
    h.switch_scope_to("scope-b");

    // A's delayed frame arrives: no tracked runtime at all ⇒ rejected, nothing
    // cached, and B's surface is pre-spawn with no A value.
    put_agent_session_config(pubkey.clone(), frame("nonce-a", "model-a"), h.state());
    assert!(
        !h.runtime_has_cache(&pubkey),
        "no runtime exists ⇒ delayed A frame cannot cache"
    );
    let surface_b = h.surface(&pubkey);
    assert!(
        surface_b.is_pre_spawn,
        "B surface must be pre-spawn, no A value"
    );

    // B spawns its own runtime under scope-b with a fresh nonce. A stale A frame
    // (wrong nonce) still rejects; B's own frame is served.
    h.seed_runtime(&record, "scope-b", "nonce-b");
    put_agent_session_config(pubkey.clone(), frame("nonce-a", "model-a"), h.state());
    assert!(
        !h.runtime_has_cache(&pubkey),
        "A's nonce must not cache onto B's runtime"
    );
    assert!(
        h.surface(&pubkey).is_pre_spawn,
        "still pre-spawn after stale frame"
    );

    put_agent_session_config(pubkey.clone(), frame("nonce-b", "model-b"), h.state());
    assert!(h.runtime_has_cache(&pubkey), "B's own frame must cache");
    assert_eq!(
        h.surface(&pubkey)
            .normalized
            .model
            .and_then(|m| m.value)
            .as_deref(),
        Some("model-b"),
        "surface must reflect B's payload"
    );
    h.reap();
}

/// A frame whose scope no longer matches the current active scope — the runtime
/// survived but the workspace rotated — is rejected with zero mutation.
#[test]
fn frame_with_stale_scope_is_rejected() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    // Runtime stamped scope-a, but the active scope is now scope-b.
    h.seed_runtime(&record, "scope-a", "nonce-1");
    h.switch_scope_to("scope-b");

    put_agent_session_config(pubkey.clone(), frame("nonce-1", "model-x"), h.state());
    assert!(
        !h.runtime_has_cache(&pubkey),
        "scope mismatch must reject the frame"
    );
    // The read gate also refuses: runtime scope-a != active scope-b.
    assert!(h.surface(&pubkey).is_pre_spawn);
    h.reap();
}

/// N→N+1 same-scope respawn: a frame from the old process generation (stale
/// `start_nonce`) arriving after respawn is rejected; the new runtime's cache
/// stays empty until the new nonce's frame lands.
#[test]
fn stale_nonce_after_respawn_is_rejected() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    // Respawn: the live runtime now carries nonce-2 (the old process was nonce-1).
    h.seed_runtime(&record, &h.scope_id, "nonce-2");

    put_agent_session_config(pubkey.clone(), frame("nonce-1", "model-old"), h.state());
    assert!(
        !h.runtime_has_cache(&pubkey),
        "stale-generation frame must be rejected"
    );
    assert!(h.surface(&pubkey).is_pre_spawn);

    put_agent_session_config(pubkey.clone(), frame("nonce-2", "model-new"), h.state());
    assert!(
        h.runtime_has_cache(&pubkey),
        "current-nonce frame must cache"
    );
    h.reap();
}

/// Control: a frame whose pair is untracked (no runtime entry) mutates nothing.
#[test]
fn untracked_pair_frame_mutates_nothing() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    // No runtime seeded.
    put_agent_session_config(pubkey.clone(), frame("nonce-1", "model-x"), h.state());
    assert!(!h.runtime_has_cache(&pubkey));
    assert!(h.surface(&pubkey).is_pre_spawn);
}

/// Control: a frame for a runtime whose process has exited (`try_wait` Some) is
/// rejected — a dead generation cannot publish config.
#[test]
fn dead_process_frame_is_rejected() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    // Seed a runtime whose child has already exited.
    {
        let key = ManagedAgentRuntimeKey::new(&pubkey, TEST_RELAY).unwrap();
        let mut process = make_process(&record, "nonce-1");
        let _ = process.child.kill();
        let _ = process.child.wait();
        process.child = spawn_dead_child();
        let state = h.state();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        runtimes.insert(
            key,
            ManagedAgentPairRuntime::starting(process, Some(h.scope_id.clone())),
        );
    }
    put_agent_session_config(pubkey.clone(), frame("nonce-1", "model-x"), h.state());
    assert!(
        !h.runtime_has_cache(&pubkey),
        "exited-process frame must be rejected"
    );
    h.reap();
}

/// Control: a missing-nonce frame (old harness) is dropped — no relay-only
/// fallback that could recreate an ownerless write.
#[test]
fn missing_nonce_frame_is_dropped() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    h.seed_runtime(&record, &h.scope_id, "nonce-1");

    let payload = serde_json::json!({
        "relayUrl": TEST_RELAY,
        "models": { "currentModelId": "model-x", "availableModels": [] },
    });
    put_agent_session_config(pubkey.clone(), payload, h.state());
    assert!(
        !h.runtime_has_cache(&pubkey),
        "a frame with no startNonce must be dropped"
    );
    h.reap();
}

/// Atomic cache destruction: removing the runtime entry destroys the embedded
/// cache — there is no separate clear step, and no map for a stale cache to
/// linger in. After removal the surface is pre-spawn again.
#[test]
fn runtime_removal_destroys_embedded_cache() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    h.seed_runtime(&record, &h.scope_id, "nonce-1");
    put_agent_session_config(pubkey.clone(), frame("nonce-1", "model-x"), h.state());
    assert!(h.runtime_has_cache(&pubkey), "precondition: frame cached");

    // Remove the runtime entry (drain/removal/exit-prune all funnel here).
    {
        let key = ManagedAgentRuntimeKey::new(&pubkey, TEST_RELAY).unwrap();
        let state = h.state();
        let mut runtimes = state.managed_agent_processes.lock().unwrap();
        if let Some(mut rt) = runtimes.remove(&key) {
            let _ = rt.child.kill();
            let _ = rt.child.wait();
        }
    }
    assert!(
        !h.runtime_has_cache(&pubkey),
        "removing the runtime destroys the embedded cache"
    );
    assert!(
        h.surface(&pubkey).is_pre_spawn,
        "no runtime ⇒ pre-spawn surface"
    );
}

/// §3.3a sibling binding: `put_managed_agent_runtime_lifecycle` shares this
/// runtime-capability sub-class. Its base checks already reject a stale-nonce
/// frame; this binds the declaration to the suite so the consistency check is
/// covered where the capability contract is tested.
#[test]
fn lifecycle_sibling_rejects_stale_nonce() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    h.seed_runtime(&record, &h.scope_id, "nonce-2");

    let payload = crate::managed_agents::ManagedAgentRuntimeLifecycleObserverPayload {
        pubkey: pubkey.clone(),
        relay_url: TEST_RELAY.to_string(),
        start_nonce: "nonce-1".to_string(),
        lifecycle: crate::managed_agents::ManagedAgentRuntimeLifecycle::Ready,
        error: None,
    };
    let result = crate::managed_agents::put_managed_agent_runtime_lifecycle_for(
        pubkey.clone(),
        payload,
        h.app.handle(),
    );
    assert!(
        result.is_err(),
        "stale-generation lifecycle frame must be rejected"
    );
    h.reap();
}

/// §3.3a sibling scope binding (P3B-I1): a same-pair/same-nonce frame on a
/// still-live runtime whose `scope_id` no longer matches the active scope — the
/// runtime survived but the workspace rotated — is rejected with zero mutation
/// and no status emit. This is the lifecycle-command analogue of
/// `frame_with_stale_scope_is_rejected`; the base checks (pair/nonce/liveness)
/// all pass, so only the scope check can reject it.
#[test]
fn lifecycle_sibling_rejects_stale_scope() {
    let _guard = gen_guard();
    let pubkey = "aa".repeat(32);
    let record = test_record(&pubkey);
    let h = Harness::new(&record);
    // Runtime stamped scope-a and live under the current nonce; the active
    // scope then rotates to scope-b.
    h.seed_runtime(&record, "scope-a", "nonce-1");
    h.switch_scope_to("scope-b");

    let payload = crate::managed_agents::ManagedAgentRuntimeLifecycleObserverPayload {
        pubkey: pubkey.clone(),
        relay_url: TEST_RELAY.to_string(),
        start_nonce: "nonce-1".to_string(),
        lifecycle: crate::managed_agents::ManagedAgentRuntimeLifecycle::Ready,
        error: None,
    };
    let result = crate::managed_agents::put_managed_agent_runtime_lifecycle_for(
        pubkey.clone(),
        payload,
        h.app.handle(),
    );
    assert!(
        result.is_err(),
        "stale-scope lifecycle frame must be rejected even with matching pair/nonce/liveness"
    );
    // Runtime unmutated: still Starting (seed), never advanced to the frame's
    // Ready. An unchanged lifecycle also proves no status was emitted — the
    // emit only runs on the Ok path after the mutation.
    {
        let key = ManagedAgentRuntimeKey::new(&pubkey, TEST_RELAY).unwrap();
        let state = h.state();
        let runtimes = state.managed_agent_processes.lock().unwrap();
        let rt = runtimes.get(&key).expect("runtime still tracked");
        assert_eq!(
            rt.lifecycle,
            crate::managed_agents::ManagedAgentRuntimeLifecycle::Starting,
            "rejected frame must not mutate the runtime lifecycle"
        );
        assert!(
            rt.error.is_none(),
            "rejected frame must not write an error onto the runtime"
        );
    }
    h.reap();
}
