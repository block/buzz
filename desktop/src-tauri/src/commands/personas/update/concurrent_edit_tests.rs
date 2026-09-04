//! Regression for Carl round-4 P1: the persona edit must reject a
//! compare-and-swap conflict BEFORE it mutates or writes, so a stale
//! full-replacement input cannot clobber a concurrent writer.
//!
//! Three layers of coverage:
//!
//! 1. **Comparison logic** — four unit tests against `find_persona_for_update`
//!    directly: stale rejection, matching-revision pass, absent-revision skip,
//!    and not-found vs conflict distinction.
//!
//! 2. **Command-path wiring** — one async test drives the real
//!    `update_persona_with` against a file-backed store under a
//!    `MockRuntime` `AppHandle`. Writer A commits R1→R2 through the command
//!    path; writer B then submits with expected R1, is rejected with
//!    `PERSONA_REVISION_CONFLICT`, and the store still reads R2. Turns RED if
//!    `update_persona_with` removes the guard call entirely.
//!
//! 3. **Lock-scope assertion** — a test-only observer (`PRE_GUARD_OBSERVER`)
//!    fires inside the `spawn_blocking` body right before `find_persona_for_update`
//!    while `managed_agents_store_lock` is held. The observer asserts
//!    `try_lock()` fails — proving the guard and the comparison share the same
//!    lock scope. Moving the lock acquisition to after the comparison (recreating
//!    the TOCTOU race) means the observer fires before the lock is held and
//!    `try_lock()` succeeds, turning this test RED.

use super::{
    find_persona_for_update, update_persona_with, PERSONA_REVISION_CONFLICT, PRE_GUARD_OBSERVER,
};
use crate::app_state::build_app_state;
use crate::managed_agents::{save_personas, AgentDefinition, UpdatePersonaRequest};

/// A persisted persona at revision `updated_at`. The guard reads only `id`,
/// `display_name`, and `updated_at`.
fn persona(id: &str, display_name: &str, updated_at: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: None,
        description: None,
        system_prompt: "Do the work.".to_string(),
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
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
    }
}

const R1: &str = "2026-01-01T00:00:00Z";
const R2: &str = "2026-06-01T00:00:00Z";

#[test]
fn two_writer_overwrite_is_rejected_and_the_newer_revision_survives() {
    // Writer B seeded its editor at R1. Writer A then committed R2, so the
    // persisted record now reads R2 when B submits. B's compare-and-swap
    // (expected R1) must be rejected before any mutation — proving A's R2
    // survives because the caller never reaches the write.
    let mut personas = vec![persona("p1", "Alice", R2)];

    let err = find_persona_for_update(&mut personas, "p1", Some(R1))
        .expect_err("a stale expected revision must be rejected");

    assert!(
        err.starts_with(PERSONA_REVISION_CONFLICT),
        "rejection must carry the conflict marker so the UI shows the drift toast; got: {err}"
    );
    assert!(
        err.contains("Alice"),
        "rejection names the persona; got: {err}"
    );
    // R2 survives untouched: the guard returned before handing out a mutable
    // handle, so nothing was overwritten.
    assert_eq!(
        personas[0].updated_at, R2,
        "the newer revision is preserved"
    );
    assert_eq!(personas[0].display_name, "Alice");
}

#[test]
fn matching_revision_resolves_the_record_for_the_write() {
    // No concurrent writer: the persisted revision still equals the seed, so
    // the guard hands back the record and the caller proceeds to write.
    let mut personas = vec![persona("p1", "Alice", R1)];

    let resolved = find_persona_for_update(&mut personas, "p1", Some(R1))
        .expect("a matching revision must resolve the record");

    assert_eq!(resolved.id, "p1");
}

#[test]
fn absent_expected_revision_skips_the_guard() {
    // Legacy callers and instance-only saves send no expected revision; the
    // guard must be inert and still resolve the record regardless of drift.
    let mut personas = vec![persona("p1", "Alice", R2)];

    let resolved = find_persona_for_update(&mut personas, "p1", None)
        .expect("no expected revision skips the compare-and-swap");

    assert_eq!(resolved.updated_at, R2);
}

#[test]
fn missing_persona_reports_not_found_not_a_conflict() {
    // A resolve miss is a plain not-found error, distinct from the revision
    // conflict — the UI must not show the drift toast for a deleted persona.
    let mut personas = vec![persona("p1", "Alice", R1)];

    let err = find_persona_for_update(&mut personas, "ghost", Some(R1))
        .expect_err("an unknown id must error");

    assert!(
        !err.starts_with(PERSONA_REVISION_CONFLICT),
        "not a conflict"
    );
    assert!(
        err.contains("ghost") && err.contains("not found"),
        "got: {err}"
    );
}

/// Build a headless `MockRuntime` `AppHandle` wired with `build_app_state`.
/// The app resolves its data dir from `$HOME` / `$XDG_DATA_HOME`; the caller
/// holds the path mutex and has overridden both so all store reads/writes land
/// inside its tempdir.
fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
    let state = build_app_state();
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds headless")
}

/// Build a minimal `UpdatePersonaRequest` for persona `id` with an optional
/// expected revision. Only `display_name` and `system_prompt` are required;
/// all other fields default to absent.
fn update_request(id: &str, display_name: &str, expected: Option<&str>) -> UpdatePersonaRequest {
    UpdatePersonaRequest {
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: None,
        description: None,
        system_prompt: "Do the work.".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        env_vars: None,
        behavior: None,
        expected_updated_at: expected.map(str::to_string),
    }
}

/// Command-path regression suite: runs in a single async test to prevent
/// test-isolation races on the process-global `HOME`/`XDG_DATA_HOME`
/// environment variables and the `PRE_GUARD_OBSERVER` slot.
///
/// **Layer 2 — wiring:** Writer A commits R1→R2 through the real
/// `update_persona_with`; Writer B submits with expected R1 and is rejected;
/// the persisted store still reads R2 with A's fields intact. Turns RED if
/// the guard call is removed entirely from `update_persona_with`.
///
/// **Layer 3 — lock scope:** the `PRE_GUARD_OBSERVER` hook fires inside
/// `spawn_blocking` right before `find_persona_for_update` while
/// `_store_guard` is in scope. The observer asserts `try_lock()` fails,
/// proving the guard and the comparison are inside the same lock scope.
/// Moving the lock acquisition to after the comparison (the TOCTOU shape)
/// means `try_lock()` succeeds and the assertion panics — turning this test
/// **RED**.
#[test]
fn command_path_and_lock_scope_regressions() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    // Hold the path mutex for the entire test so concurrent tests that also
    // mutate HOME cannot race against our store reads/writes.
    let _path_guard = crate::managed_agents::lock_path_mutex();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &home);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");

    rt.block_on(async {
        let app = mock_app();

        // ── Layer 3: lock-scope assertion ─────────────────────────────────────
        // Install the observer BEFORE seeding so it fires on the first real write.
        let observer_fired = Arc::new(AtomicBool::new(false));
        let observer_fired_clone = observer_fired.clone();
        {
            let mut slot = PRE_GUARD_OBSERVER.lock().expect("observer slot");
            *slot = Some(Box::new(move |state: &crate::app_state::AppState| {
                assert!(
                    state.managed_agents_store_lock.try_lock().is_err(),
                    "managed_agents_store_lock must already be held when find_persona_for_update \
                 runs — a successful try_lock means the comparison happens outside the lock, \
                 which recreates the TOCTOU race"
                );
                observer_fired_clone.store(true, Ordering::SeqCst);
            }));
        }

        // Seed: one persona at R1 in the persisted store.
        save_personas(app.handle(), &[persona("p1", "Alice", R1)])
            .expect("seed write must succeed");

        // Layer 3 probe: a successful write fires the observer while the lock is held.
        let probe_result = update_persona_with(
            update_request("p1", "Alice probe", Some(R1)),
            app.handle().clone(),
            |_app, _state, _persona| Ok(()),
        )
        .await;

        // Clear the observer immediately — must happen before any assertion that
        // could panic, so it is never left installed for subsequent invocations.
        {
            let mut slot = PRE_GUARD_OBSERVER.lock().expect("observer slot");
            *slot = None;
        }

        probe_result.expect("probe write must succeed — revision matches the seed");
        assert!(
            observer_fired.load(Ordering::SeqCst),
            "the pre-guard observer must have fired — if it did not, the hook is not wired"
        );

        // ── Layer 2: command-path wiring ──────────────────────────────────────
        // Writer A: now at the probe's updated_at (R2). Capture it so B can use
        // the original R1 as a stale seed.
        //
        // Re-seed at R1 so the two-writer scenario starts from a known revision.
        save_personas(app.handle(), &[persona("p1", "Alice", R1)])
            .expect("re-seed write must succeed");

        let a_result = update_persona_with(
            update_request("p1", "Alice A", Some(R1)),
            app.handle().clone(),
            |_app, _state, _persona| Ok(()),
        )
        .await;
        let (a_persona, ()) = a_result.expect("writer A must succeed — revision matches the seed");
        let r2 = a_persona.updated_at.clone();
        assert_ne!(r2, R1, "the commit must advance the revision past R1");

        // Writer B: seeded at R1, submits after A has committed R2.
        let b_result = update_persona_with(
            update_request("p1", "Alice B (must not land)", Some(R1)),
            app.handle().clone(),
            |_app, _state, _persona| Ok(()),
        )
        .await;
        let b_err =
            b_result.expect_err("writer B must be rejected — its seed revision R1 is stale");

        assert!(
            b_err.starts_with(PERSONA_REVISION_CONFLICT),
            "rejection must carry the conflict marker; got: {b_err}"
        );

        // Reload from the persisted store and confirm A's R2 survived B's attempt.
        let persisted =
            crate::managed_agents::load_personas(app.handle()).expect("reload must succeed");
        let stored = persisted
            .iter()
            .find(|p| p.id == "p1")
            .expect("persona must still exist after B's rejection");

        assert_eq!(
            stored.updated_at, r2,
            "A's committed revision must survive B's stale write attempt"
        );
        assert_eq!(
            stored.display_name, "Alice A",
            "A's committed display_name must survive B's stale write attempt"
        );
    }); // rt.block_on

    // ── Cleanup ───────────────────────────────────────────────────────────
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_DATA_HOME");
    match old_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    match old_xdg {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}

/// P1-1 regression: the retain callback (the "publish" step inside
/// `update_persona_with`) runs AFTER `save_personas`, so a retain failure
/// occurs when the persona is already durable on disk.
///
/// The command-path contract: `update_persona_with` must return `Err(_)` when
/// the retain callback returns `Err(_)`, even though the persona write already
/// persisted. The error must reach the coordinator so it can set `publishFailed`
/// and refuse to close the dialog as full success. If the retain error were
/// swallowed — e.g. the callback's `?` were removed and it returned `Ok(())` —
/// this test would become GREEN on a broken code path and catch it.
#[test]
fn retain_failure_after_persist_is_returned_not_swallowed() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    let _path_guard = crate::managed_agents::lock_path_mutex();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &home);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");

    rt.block_on(async {
        let app = mock_app();

        // Seed one persona at R1 in the persisted store.
        save_personas(app.handle(), &[persona("p1", "Alice", R1)])
            .expect("seed write must succeed");

        // Submit via update_persona_with with a retain callback that always fails.
        // The persona save runs first (line ~210 in update.rs: `save_personas`),
        // then the retain callback is called. If the retain error propagates, we
        // get Err; if it is silently discarded, we get Ok — the test catches both.
        let result = update_persona_with(
            update_request("p1", "Alice retain-fail", Some(R1)),
            app.handle().clone(),
            |_app, _state, _persona| -> Result<(), String> {
                Err("simulated retain / publish failure after persona persisted".to_string())
            },
        )
        .await;

        assert!(
            result.is_err(),
            "update_persona_with must return Err when the retain callback fails — \
         the save coordinator must see this error, not a silent success; \
         got: {:?}",
            result.ok()
        );

        let err = result.unwrap_err();
        assert!(
            err.contains("simulated retain"),
            "the error text must propagate from the retain callback; got: {err}"
        );

        // Verify the persona DID persist (save_personas ran before retain) so we
        // confirm the test scenario actually exercises the post-persist failure path.
        let persisted =
            crate::managed_agents::load_personas(app.handle()).expect("reload must succeed");
        let stored = persisted
            .iter()
            .find(|p| p.id == "p1")
            .expect("persona must exist");
        assert_eq!(
            stored.display_name, "Alice retain-fail",
            "persona fields must have persisted before retain was called"
        );
    }); // rt.block_on

    // Cleanup
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_DATA_HOME");
    match old_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    match old_xdg {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}

/// P2 regression: linked managed-agent name and avatar propagation must
/// complete even when the retain/publish callback returns an error.
///
/// Before the fix: `retain(&app, &state, &result)?` propagated the error
/// immediately, skipping the avatar/name propagation block and `save_managed_agents`.
/// A publish-only retry (`set_persona_shared`) then published the persona but
/// the linked instance still carried the old name/avatar.
///
/// After the fix: `retain_result` is captured without `?`, all linked
/// persistence runs to completion, and THEN the retain error is propagated.
///
/// Mutation acceptance: restoring `retain(&app, &state, &result)?` before
/// the avatar/name propagation block causes `save_managed_agents` to never run
/// and the assertion on the stored agent name turns RED.
#[test]
fn linked_instance_rename_completes_before_retain_error_propagates() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home2");
    std::fs::create_dir_all(&home).unwrap();

    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    // Hold the path mutex for the entire test so concurrent tests that also
    // mutate HOME cannot race against our store reads/writes.
    let _path_guard = crate::managed_agents::lock_path_mutex();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &home);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");

    rt.block_on(async {
        let app = mock_app();

        // Seed persona "Alice" at R1.
        save_personas(app.handle(), &[persona("p1", "Alice", R1)]).expect("seed must succeed");

        // Seed a linked agent whose name matches the persona's display_name.
        let agent_record = crate::managed_agents::ManagedAgentRecord {
            pubkey: "pk-alice-p2".to_string(),
            name: "Alice".to_string(),
            persona_id: Some("p1".to_string()),
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: String::new(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: false,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: Default::default(),
            respond_to_allowlist: vec![],
            display_name: Some("Alice".to_string()),
            description: None,
            slug: None,
            runtime: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_parallelism: None,
            relay_mesh: None,
            effort_level: None,
        };
        crate::managed_agents::save_managed_agents(app.handle(), &[agent_record])
            .expect("agent seed must succeed");

        // Submit a rename with a retain callback that always fails — simulates a
        // strict publication/enqueue failure AFTER save_personas ran.
        let result = update_persona_with(
            UpdatePersonaRequest {
                id: "p1".to_string(),
                display_name: "Alice Renamed".to_string(),
                avatar_url: None,
                description: None,
                system_prompt: "Do the work.".to_string(),
                runtime: None,
                model: None,
                provider: None,
                name_pool: Vec::new(),
                env_vars: None,
                behavior: None,
                expected_updated_at: Some(R1.to_string()),
            },
            app.handle().clone(),
            |_app, _state, _persona| -> Result<(), String> {
                Err("simulated publish failure after persona persisted".to_string())
            },
        )
        .await;

        // The retain error must propagate so the coordinator sees publishFailed.
        assert!(
            result.is_err(),
            "update_persona_with must return Err when retain fails; \
         if this is Ok the retain error was swallowed"
        );

        // The linked agent must have been renamed BEFORE the error propagated.
        // If save_managed_agents was skipped (the pre-fix path), the name is still "Alice".
        let agents = crate::managed_agents::load_managed_agents(app.handle())
            .expect("agent reload must succeed");
        let stored_agent = agents
            .iter()
            .find(|a| a.pubkey == "pk-alice-p2")
            .expect("linked agent must still exist");

        assert_eq!(
            stored_agent.name, "Alice Renamed",
            "linked instance name must be propagated before the retain error is returned; \
         restoring `retain()?` before the propagation block turns this RED"
        );
    }); // rt.block_on

    // Cleanup
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_DATA_HOME");
    match old_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    match old_xdg {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}

/// P2 relay-sync regression: the relay kind:0 profile sync for linked agents
/// must complete even when the retain/publish callback returns an error.
///
/// Before the fix: `retain_result?` applied `?` inside the blocking phase's
/// return tuple, exiting before `profile_sync_params` was returned to the outer
/// function. Phase 2 (the async `sync_managed_agent_profile` loop) never ran —
/// the linked relay identity remained stale even though the local record updated.
///
/// After the fix: `retain_result` is returned un-`?`d from the blocking phase,
/// phase 2 runs to completion, and THEN the retain error is propagated.
///
/// Mutation acceptance: restoring `retain_result?` inside `Ok((result, retain_result?, …))`
/// causes the blocking phase to exit before `profile_sync_params` is returned,
/// so the counter server receives 0 requests and this test turns RED.
#[test]
fn linked_instance_relay_profile_syncs_despite_retain_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tauri::Manager;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home3");
    std::fs::create_dir_all(&home).unwrap();

    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    let _path_guard = crate::managed_agents::lock_path_mutex();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &home);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");

    rt.block_on(async {
        let app = mock_app();

        // Spawn a local HTTP server that counts POST /events requests.
        // sync_managed_agent_profile posts kind:0 profile events here.
        let post_count = Arc::new(AtomicUsize::new(0));
        let post_count_clone = post_count.clone();
        let relay_server = {
            use axum::{routing::post, Router};
            let app_router = Router::new().route(
                "/events",
                post(move |_body: String| {
                    let counter = post_count_clone.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        serde_json::json!({
                            "event_id": "test-event-id",
                            "accepted": true,
                            "message": ""
                        })
                        .to_string()
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app_router).await.ok();
            });
            format!("http://{addr}")
        };

        // Point the workspace relay override at the counter server so that
        // relay_ws_url_with_override (used to build relay_url per record)
        // routes profile syncs to our counter, not the real relay.
        {
            let state = app.state::<crate::app_state::AppState>();
            let mut override_slot = state
                .relay_url_override
                .lock()
                .expect("relay_url_override must be lockable");
            *override_slot = Some(relay_server.clone());
        }

        // Seed persona "Alice" at R1.
        save_personas(app.handle(), &[persona("p1", "Alice", R1)]).expect("seed must succeed");

        // Seed a linked agent with valid NSEC keys so sync_managed_agent_profile
        // can sign and submit the kind:0 profile event.
        let agent_keys = nostr::Keys::generate();
        let agent_record = crate::managed_agents::ManagedAgentRecord {
            pubkey: agent_keys.public_key().to_hex(),
            name: "Alice".to_string(),
            persona_id: Some("p1".to_string()),
            private_key_nsec: agent_keys.secret_key().to_secret_hex(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: String::new(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: false,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: Default::default(),
            respond_to_allowlist: vec![],
            display_name: Some("Alice".to_string()),
            description: None,
            slug: None,
            runtime: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_parallelism: None,
            relay_mesh: None,
            effort_level: None,
        };
        crate::managed_agents::save_managed_agents(app.handle(), &[agent_record])
            .expect("agent seed must succeed");

        // Submit a rename with a retain closure that always fails — simulates
        // strict publication failure after persona save. The display_name
        // change triggers name propagation and relay profile sync.
        let result = update_persona_with(
            UpdatePersonaRequest {
                id: "p1".to_string(),
                display_name: "Alice Renamed".to_string(),
                avatar_url: None,
                description: None,
                system_prompt: "Do the work.".to_string(),
                runtime: None,
                model: None,
                provider: None,
                name_pool: Vec::new(),
                env_vars: None,
                behavior: None,
                expected_updated_at: Some(R1.to_string()),
            },
            app.handle().clone(),
            |_app, _state, _persona| -> Result<(), String> {
                Err("simulated strict publication failure after persona persisted".to_string())
            },
        )
        .await;

        // The retain error must propagate so the coordinator sees publishFailed.
        assert!(
            result.is_err(),
            "update_persona_with must return Err when retain fails"
        );

        // Phase 2 must have run: the counter server must have received exactly
        // one kind:0 profile-sync POST for the renamed linked agent.
        // Before the fix (retain_result? inside the blocking Ok): profile_sync_params
        // is never returned to the outer fn — count is 0, test RED.
        // After the fix (retain_result? after phase 2): count is 1, test GREEN.
        assert_eq!(
            post_count.load(Ordering::SeqCst),
            1,
            "relay kind:0 profile sync must fire despite retain failure; \
             restoring `retain_result?` before phase 2 (the pre-fix shape) turns this RED"
        );
    }); // rt.block_on

    // Cleanup: restore HOME/XDG after the relay-profile-syncs test
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_DATA_HOME");
    match old_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    match old_xdg {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}
