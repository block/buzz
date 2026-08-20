//! Phase-boundary seam tests for team snapshot import (Area 3).
//!
//! Split from `tests.rs` to keep each file under the 1000-line ratchet.
//! Included via `#[path = "seam_tests.rs"] mod seam_tests;` from `tests.rs`.
//! `use super::*` gives access to all items in `tests.rs`.
use super::*;

// ── Phase-boundary seam tests (Area 3 — team) ─────────────────────────────

/// Serializes tests that modify the process-global scope generation counter.
/// See the equivalent comment in `import_tests.rs` for rationale.
use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK as GENERATION_TEST_LOCK;

/// Build a team-member snapshot with a core memory entry.
///
/// Used by tests that must exercise the Phase-5 memory loop in the team import
/// core and assert that `submit_memory` carries the captured relay + owner.
fn member_with_memory(name: &str) -> AgentSnapshot {
    use crate::managed_agents::agent_snapshot::{AgentSnapshotMemoryEntry, MemoryLevel};
    let mut m = member(name);
    m.memory = crate::managed_agents::agent_snapshot::AgentSnapshotMemory {
        level: MemoryLevel::Core,
        entries: vec![AgentSnapshotMemoryEntry {
            slug: buzz_core_pkg::engram::CORE_SLUG.to_string(),
            body: format!("# {name}\nTeam member memory body."),
        }],
    };
    m
}

/// Extract the first `p` tag value from a nostr event JSON byte slice.
///
/// Returns `Some(hex_pubkey)` if a `["p", "<hex>"]` tag entry is found,
/// `None` if the JSON cannot be parsed or has no `p` tag.
fn extract_p_tag_from_memory_event(event_json: &[u8]) -> Option<String> {
    let val: serde_json::Value = serde_json::from_slice(event_json).ok()?;
    let tags = val.get("tags")?.as_array()?;
    for tag in tags {
        if let Some(arr) = tag.as_array() {
            if arr.first().and_then(|v| v.as_str()) == Some("p") {
                if let Some(hex) = arr.get(1).and_then(|v| v.as_str()) {
                    return Some(hex.to_string());
                }
            }
        }
    }
    None
}

fn setup_team_import_app_with_scope(
    tmp: &tempfile::TempDir,
) -> (tauri::App<tauri::test::MockRuntime>, nostr::Keys) {
    use crate::managed_agents::scope::{next_scope_generation, WorkspaceAgentScope};

    let owner_keys = nostr::Keys::generate();
    let state = crate::app_state::build_app_state();
    {
        let mut locked = state.identity_lifecycle_keys_guard().unwrap();
        *locked = owner_keys.clone();
    }

    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app for team import test");

    {
        use tauri::Manager;
        let s = app.state::<crate::app_state::AppState>();
        let gen = next_scope_generation();
        // Use WorkspaceAgentScope::new so definitions_dir has the production
        // shape: <tmp>/scopes/<scope_id>/.  retention_scope_from_captured derives
        // the agent base two parents above definitions_dir — with this layout it
        // resolves to <tmp> (writable) rather than / (which causes EPERM on Linux).
        let scope = WorkspaceAgentScope::new(
            "wss://captured.example".to_string(),
            owner_keys.public_key().to_hex(),
            tmp.path(),
            gen,
        );
        // Ensure the definitions directory exists so the core can write into it.
        std::fs::create_dir_all(&scope.definitions_dir)
            .expect("failed to create team scope definitions dir");
        s.commit_active_scope(scope);
    }

    (app, owner_keys)
}

/// `after_store` hook commits a genuinely different live scope + owner —
/// Phase 4/5 outbound must use the OLD (captured) relay URL, not the new
/// live relay.
///
/// Thufir requirement: `after_store` must commit a genuinely different live
/// scope and owner, not merely increment a counter. We swap the active scope
/// to a different relay + fresh owner inside the hook; all per-member profile
/// adapters must receive the old captured relay URL.
///
/// One member carries a core memory entry so the Phase-5 memory loop in
/// `team_snapshot.rs` fires. The memory adapter asserts BOTH the captured
/// relay URL AND that the built engram event's `p` tag (owner counterpart)
/// matches the CAPTURED owner's pubkey — not the post-switch owner committed
/// in `after_store`. This validates the team core's independently implemented
/// Phase-5 memory loop (`team_snapshot.rs:815-860`) which uses
/// `captured_owner_keys` at `:553`.
#[tokio::test]
// SAFETY: single-threaded tokio runtime; lock held to serialize generation
// counter mutations — cannot deadlock. See import_tests.rs for full rationale.
#[allow(clippy::await_holding_lock)]
async fn test_confirm_team_snapshot_import_switch_between_store_and_profile() {
    let _gen_guard = GENERATION_TEST_LOCK.lock().unwrap();

    use crate::commands::personas::snapshot::import::{MemoryPublish, ProfilePublish};
    use crate::commands::team_snapshot::confirm_team_snapshot_import_core;
    use crate::managed_agents::scope::{current_scope_generation, WorkspaceAgentScope};
    use std::sync::{Arc, Mutex};
    use tauri::Manager;

    let tmp = tempfile::tempdir().unwrap();
    let (app, owner_keys) = setup_team_import_app_with_scope(&tmp);
    let handle = app.handle();

    // One plain member + one member with a core memory entry so the Phase-5
    // memory loop fires for the second member.
    let snap = snapshot(vec![member("Alice"), member_with_memory("Bob")]);
    let encoded = crate::managed_agents::team_snapshot::encode_team_snapshot_json(&snap).unwrap();
    let input = TeamSnapshotImportConfirm {
        file_bytes: encoded,
        keep_allowlist: false,
    };

    // Captured owner pubkey — must appear in the `p` tag of memory events.
    let captured_owner_pubkey_hex = owner_keys.public_key().to_hex();
    let expected_relay = "wss://captured.example".to_string();

    let profile_relays: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let memory_relays: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let memory_p_tags: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let pr = profile_relays.clone();
    let mr = memory_relays.clone();
    let mp = memory_p_tags.clone();

    let state = app.state::<crate::app_state::AppState>();
    let handle_for_hook = handle.clone();

    let result = confirm_team_snapshot_import_core(
        input,
        handle,
        &state,
        || {},
        move || {
            // after_store: commit a genuinely DIFFERENT live scope + owner.
            let new_owner = nostr::Keys::generate();
            let new_scope = WorkspaceAgentScope::new(
                "wss://new-relay-after-switch.example".to_string(),
                new_owner.public_key().to_hex(),
                std::path::Path::new("/tmp/switched"),
                current_scope_generation(),
            );
            let s = handle_for_hook.state::<crate::app_state::AppState>();
            s.commit_active_scope(new_scope);
        },
        move |p: ProfilePublish<'_>| {
            let relay = p.relay_url.to_string();
            pr.lock().unwrap().push(relay.clone());
            Box::pin(async move {
                let _ = relay;
                Ok(())
            })
        },
        move |m: MemoryPublish<'_>| {
            // Assert: relay URL contains the captured relay, not the switched one.
            let relay = m.relay_url.to_string();
            mr.lock().unwrap().push(relay.clone());
            // Extract the `p` tag — must carry the CAPTURED owner pubkey.
            let event_bytes = m.event_json.to_vec();
            if let Some(p_tag) = extract_p_tag_from_memory_event(&event_bytes) {
                mp.lock().unwrap().push(p_tag);
            }
            Box::pin(async move { Ok(()) })
        },
    )
    .await;

    assert!(
        result.is_ok(),
        "team import must succeed: {:?}",
        result.err()
    );

    // Every member's profile adapter received the captured relay URL.
    let seen_profiles = profile_relays.lock().unwrap();
    assert_eq!(
        seen_profiles.len(),
        2,
        "profile adapter must be called once per member"
    );
    for relay in seen_profiles.iter() {
        assert_eq!(
            relay, &expected_relay,
            "profile adapter must receive captured relay, got: {relay}"
        );
    }

    // Memory adapter was called for the member with memory entries.
    let seen_memory_relays = memory_relays.lock().unwrap();
    assert!(
        !seen_memory_relays.is_empty(),
        "memory adapter must be called for the member with memory entries"
    );
    for relay in seen_memory_relays.iter() {
        assert!(
            relay.contains("captured.example"),
            "memory adapter relay_url must contain captured relay 'captured.example', got: {relay}"
        );
    }

    // Memory event's `p` tag must equal the CAPTURED owner's pubkey — not the
    // post-switch owner committed in `after_store`.
    let seen_p_tags = memory_p_tags.lock().unwrap();
    assert!(
        !seen_p_tags.is_empty(),
        "at least one memory event must carry a `p` tag"
    );
    for p_tag in seen_p_tags.iter() {
        assert_eq!(
            p_tag.as_str(),
            captured_owner_pubkey_hex.as_str(),
            "engram event p-tag must equal the captured owner's pubkey (not post-switch owner); \
             got: {p_tag}"
        );
    }
}

/// `before_store` hook advances scope generation — Phase 3 must reject BEFORE
/// any write and BEFORE any outbound call.
#[tokio::test]
// SAFETY: single-threaded tokio runtime; lock held to serialize generation
// counter mutations — cannot deadlock. See import_tests.rs for full rationale.
#[allow(clippy::await_holding_lock)]
async fn test_team_identity_switch_before_store_is_rejected() {
    let _gen_guard = GENERATION_TEST_LOCK.lock().unwrap();

    use crate::commands::personas::snapshot::import::{MemoryPublish, ProfilePublish};
    use crate::commands::team_snapshot::confirm_team_snapshot_import_core;
    use crate::managed_agents::scope::next_scope_generation;
    use tauri::Manager;

    let tmp = tempfile::tempdir().unwrap();
    let (app, _owner_keys) = setup_team_import_app_with_scope(&tmp);
    let handle = app.handle();
    let state = app.state::<crate::app_state::AppState>();

    let snap = snapshot(vec![member("Alice")]);
    let encoded = crate::managed_agents::team_snapshot::encode_team_snapshot_json(&snap).unwrap();
    let input = TeamSnapshotImportConfirm {
        file_bytes: encoded,
        keep_allowlist: false,
    };

    let result = confirm_team_snapshot_import_core(
        input,
        handle,
        &state,
        move || {
            next_scope_generation();
        },
        || {},
        |_p: ProfilePublish<'_>| {
            Box::pin(async { panic!("profile must not be called: store rejected") })
        },
        |_m: MemoryPublish<'_>| {
            Box::pin(async { panic!("memory must not be called: store rejected") })
        },
    )
    .await;

    assert!(
        result.is_err(),
        "pre-store switch must cause Phase 3 rejection"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("stale") || err.contains("generation") || err.contains("mismatch"),
        "error must describe generation mismatch: {err}"
    );
}
