//! Tests for `confirm_agent_snapshot_import` seams and helpers.
//!
//! Extracted from `import.rs` to keep that file within the 1000-line gate.
//! Included via `#[path]` from `import.rs`.

use super::{materialize_import_avatar, submit_engram_event};

const NCRYPTSEC: &str = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

/// An engram body carrying an ncryptsec must be rejected by the guard
/// before any network I/O (the target port is a discard address; a guard
/// error — not a connection error — proves the abort ordering).
#[tokio::test]
async fn blocks_ncryptsec_before_network() {
    let state = crate::app_state::build_app_state();
    let keys = nostr::Keys::generate();
    let body = format!("{{\"content\":\"{NCRYPTSEC}\"}}");
    let err = submit_engram_event(
        &state,
        &keys,
        body.as_bytes(),
        "http://127.0.0.1:9/events",
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("key-backup material"), "{err}");
}

#[tokio::test]
async fn inline_avatar_is_uploaded_and_replaced_with_hosted_url() {
    let uploaded = std::cell::Cell::new(false);
    let result = materialize_import_avatar(
        Some("data:image/png;base64,iVBORw0KGgo="),
        Some("https://sender.invalid/avatar.png"),
        |bytes| {
            uploaded.set(true);
            async move {
                assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
                Ok("https://relay.example/media/avatar.png".to_string())
            }
        },
    )
    .await
    .unwrap();

    assert!(uploaded.get());
    assert_eq!(
        result.as_deref(),
        Some("https://relay.example/media/avatar.png")
    );
}

#[tokio::test]
async fn hosted_avatar_skips_upload() {
    let result =
        materialize_import_avatar(None, Some("https://sender.example/avatar.png"), |_| async {
            panic!("hosted avatars must not be uploaded")
        })
        .await
        .unwrap();

    assert_eq!(result.as_deref(), Some("https://sender.example/avatar.png"));
}

#[tokio::test]
async fn relay_sized_inline_avatar_becomes_bounded_signed_profile() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use image::ImageEncoder;
    use nostr::JsonUtil;

    let mut pixels = vec![0_u8; 512 * 512 * 4];
    let mut seed = 0x1234_5678_u32;
    for byte in &mut pixels {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *byte = seed as u8;
    }
    let mut source = Vec::new();
    image::codecs::png::PngEncoder::new(&mut source)
        .write_image(&pixels, 512, 512, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(source.len() > 256 * 1024);
    let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&source));
    assert!(data_url.len() > 256 * 1024);

    let avatar = materialize_import_avatar(Some(&data_url), None, |bytes| async move {
        let mime = crate::commands::media::detect_and_validate_mime(&bytes)?;
        assert_eq!(mime, "image/png");
        let sanitized = crate::commands::media::sanitize_image_for_upload(bytes, &mime)?;
        image::load_from_memory(&sanitized).map_err(|error| error.to_string())?;
        Ok("https://relay.example/media/avatar.png".to_string())
    })
    .await
    .unwrap()
    .unwrap();

    let event =
        crate::events::build_profile(Some("Imported agent"), None, Some(&avatar), None, None)
            .unwrap()
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
    assert!(event.content.len() < 64 * 1024);
    assert!(!event.content.contains("data:image/"));
    assert!(event
        .content
        .contains("https://relay.example/media/avatar.png"));
    assert!(event.as_json().len() < 256 * 1024);
}

#[tokio::test]
async fn upload_failure_aborts_avatar_materialization() {
    let result = materialize_import_avatar(
        Some("data:image/png;base64,iVBORw0KGgo="),
        None,
        |_| async { Err("relay upload failed".to_string()) },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("relay upload failed"));
}

#[tokio::test]
async fn malformed_inline_avatar_fails_before_upload() {
    let result =
        materialize_import_avatar(Some("data:image/png;base64,not-base64!"), None, |_| async {
            panic!("malformed avatars must not be uploaded")
        })
        .await;

    assert_eq!(result.unwrap_err(), "Snapshot avatar data is malformed.");
}

// ── Phase-boundary seam tests (Area 3) ───────────────────────────────────────

/// Shared cross-module serialization lock for generation-sensitive tests.
/// See `managed_agents::scope::SCOPE_GENERATION_TEST_LOCK` for full rationale.
/// Re-exported here so test functions can reference it without the full path.
use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK as GENERATION_TEST_LOCK;

/// Build a minimal agent snapshot JSON for import tests.
fn minimal_agent_snapshot_json(name: &str) -> Vec<u8> {
    use crate::managed_agents::agent_snapshot::encode_snapshot_json;
    use crate::managed_agents::agent_snapshot::{
        AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotProfile,
        MemoryLevel, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
    };

    let snap = AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: name.to_string(),
            source_is_builtin: false,
            system_prompt: Some(format!("{name} prompt")),
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: name.to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: Some(format!("https://example.test/{name}.png")),
        },
        memory: AgentSnapshotMemory {
            level: MemoryLevel::None,
            entries: vec![],
        },
    };
    encode_snapshot_json(&snap).expect("encode_snapshot_json must succeed for minimal snapshot")
}

/// Build a minimal agent snapshot JSON that includes one core memory entry.
///
/// Used by tests that must exercise the Phase-4 memory-publish path and assert
/// that `MemoryPublish` carries the captured (pre-switch) relay and owner.
fn minimal_agent_snapshot_json_with_memory(name: &str) -> Vec<u8> {
    use crate::managed_agents::agent_snapshot::encode_snapshot_json;
    use crate::managed_agents::agent_snapshot::{
        AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotMemoryEntry,
        AgentSnapshotProfile, MemoryLevel, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
    };

    let snap = AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: name.to_string(),
            source_is_builtin: false,
            system_prompt: Some(format!("{name} prompt")),
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: name.to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: Some(format!("https://example.test/{name}.png")),
        },
        memory: AgentSnapshotMemory {
            level: MemoryLevel::Core,
            entries: vec![AgentSnapshotMemoryEntry {
                slug: buzz_core_pkg::engram::CORE_SLUG.to_string(),
                body: format!("# {name}\nTest memory body."),
            }],
        },
    };
    encode_snapshot_json(&snap).expect("encode_snapshot_json must succeed for memory snapshot")
}

/// Set up a mock Tauri `App` with `AppState` managed, an active workspace
/// scope, and matching owner keys.
///
/// Returns the built `App` (keeps state alive for test duration) and the
/// generated owner keys. The `App`'s `handle()` is passed as the `app`
/// parameter to core functions; `app.state::<AppState>()` gives the state
/// reference for setup mutations inside hook closures.
///
/// Uses `tauri::test::mock_builder().manage(state)` so that `app.state::<AppState>()`
/// works inside `try_regenerate_nest` and other AppHandle users called by the core.
///
/// Uses `WorkspaceAgentScope::new` with `base_dir = tmp.path()` so that
/// `definitions_dir = <tmp>/scopes/<scope_id>/`.  `retention_scope_from_captured`
/// derives the agent base two parents above `definitions_dir`, yielding
/// `<tmp>` — a writable directory — instead of `/` (which causes EPERM on Linux
/// when `definitions_dir` is set directly to the tempdir root).
///
/// Uses `next_scope_generation()` to claim the current generation slot so the
/// scope's generation matches the global counter at entry, reducing the race
/// window vs. tests that call `next_scope_generation()` concurrently.
fn setup_import_app_with_scope(
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
        .expect("failed to build mock app for import test");

    {
        use tauri::Manager;
        let s = app.state::<crate::app_state::AppState>();
        // Claim a fresh generation slot: next_scope_generation() increments the
        // global counter and returns the new value.  The active scope uses this
        // value so capture_agent_snapshot_import_entry sees a matching generation
        // when it reads current_scope_generation() at entry.
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
            .expect("failed to create scope definitions dir");
        s.commit_active_scope(scope);
    }

    (app, owner_keys)
}

/// `after_store` hook commits a genuinely different live scope + owner —
/// Phase 3b outbound must still use the OLD (captured) relay URL, not the
/// new live relay.
///
/// Thufir requirement: `after_store` must commit a genuinely different live
/// scope and owner, not merely increment a counter.  We swap the active scope
/// to a different relay + fresh owner inside the hook so that if Phase 3b
/// ever re-read live state it would see the new relay.  The injected profile
/// adapter asserts it receives the OLD captured relay, proving Phase 3b is
/// scope-independent after Phase 3a completes.
///
/// The snapshot carries one core memory entry so `submit_memory` actually
/// fires. The memory adapter asserts BOTH the captured relay URL AND that the
/// built engram event's `p` tag (owner counterpart/coordinate) matches the
/// CAPTURED owner key — not the new owner committed in `after_store`.
#[tokio::test]
// SAFETY: `#[tokio::test]` uses a single-threaded runtime by default, so
// holding `std::sync::Mutex` across `.await` points cannot deadlock.
// The lock serializes tests that advance the process-global scope generation
// counter — dropping it early would let a racing test corrupt the counter
// mid-import, causing a spurious stale-scope failure.
#[allow(clippy::await_holding_lock)]
async fn test_agent_switch_between_store_and_profile_finishes_captured_outbound() {
    // Serialize against the before_store rejection test to prevent the
    // concurrent next_scope_generation() bump from causing a spurious
    // Phase 3a stale-scope failure.
    let _gen_guard = GENERATION_TEST_LOCK.lock().unwrap();

    use crate::commands::personas::snapshot::import::{
        confirm_agent_snapshot_import_core, AgentSnapshotImportConfirm, MemoryPublish,
        ProfilePublish,
    };
    use crate::managed_agents::scope::{current_scope_generation, WorkspaceAgentScope};
    use std::sync::{Arc, Mutex};
    use tauri::Manager;

    let tmp = tempfile::tempdir().unwrap();
    let (app, owner_keys) = setup_import_app_with_scope(&tmp);
    let handle = app.handle();

    // Snapshot with one core memory entry so Phase 4 actually calls submit_memory.
    let file_bytes = minimal_agent_snapshot_json_with_memory("TestAgent");
    let input = AgentSnapshotImportConfirm {
        file_bytes,
        keep_allowlist: false,
    };

    // Relay URL embedded in the captured scope (must appear in profile_sync and
    // in the relay URL passed to submit_memory).
    let expected_relay = "wss://captured.example".to_string();
    // Captured owner pubkey — must appear in the `p` tag of the built engram event.
    let captured_owner_pubkey_hex = owner_keys.public_key().to_hex();

    // Track what the outbound adapters received.
    let profile_relay = Arc::new(Mutex::new(None::<String>));
    let memory_relay = Arc::new(Mutex::new(None::<String>));
    let memory_owner_p_tag = Arc::new(Mutex::new(None::<String>));
    let pr = profile_relay.clone();
    let mr = memory_relay.clone();
    let mop = memory_owner_p_tag.clone();

    // The after_store hook needs to commit a new scope via AppState.
    // Get the state reference from the app's managed state.
    let state = app.state::<crate::app_state::AppState>();
    // Clone the app handle for the hook to use.
    let handle_for_hook = handle.clone();

    let result = confirm_agent_snapshot_import_core(
        input,
        handle,
        &state,
        || {}, // before_store: no-op
        move || {
            // after_store: commit a genuinely DIFFERENT live scope + owner.
            // This simulates a workspace switch at the Phase-3a→3b boundary.
            // Phase 3b must still use the OLD captured relay, not this new one.
            let new_owner = nostr::Keys::generate();
            let new_scope = WorkspaceAgentScope {
                scope_id: "switched-scope".to_string(),
                relay_url: "wss://new-relay-after-switch.example".to_string(),
                owner_pubkey: new_owner.public_key().to_hex(),
                definitions_dir: std::path::PathBuf::from("/tmp/switched"),
                generation: current_scope_generation(),
            };
            let s = handle_for_hook.state::<crate::app_state::AppState>();
            s.commit_active_scope(new_scope);
        },
        move |p: ProfilePublish<'_>| {
            let relay = p.relay_url.to_string();
            *pr.lock().unwrap() = Some(relay.clone());
            Box::pin(async move {
                let _ = relay;
                Ok(())
            })
        },
        move |m: MemoryPublish<'_>| {
            // Assert: relay URL contains the captured base relay, not the switched one.
            let relay = m.relay_url.to_string();
            *mr.lock().unwrap() = Some(relay.clone());

            // Extract the `p` tag from the built engram event JSON.
            // The `p` tag must carry the CAPTURED owner's pubkey hex — not the
            // post-switch owner committed in after_store.
            let event_bytes = m.event_json.to_vec();
            let p_tag_hex = extract_p_tag_from_event_json(&event_bytes);
            *mop.lock().unwrap() = p_tag_hex;

            Box::pin(async move { Ok(()) })
        },
    )
    .await;

    // The import succeeded — agent was written to the captured scope.
    assert!(result.is_ok(), "import must succeed: {:?}", result.err());

    // Profile adapter received the OLD captured relay URL, not the new live relay.
    let profile_seen = profile_relay.lock().unwrap().clone();
    assert_eq!(
        profile_seen.as_deref(),
        Some(expected_relay.as_str()),
        "profile adapter must receive captured relay, got: {profile_seen:?}"
    );

    // Memory adapter received the OLD captured relay URL in its relay field.
    let memory_relay_seen = memory_relay.lock().unwrap().clone();
    assert!(
        memory_relay_seen
            .as_deref()
            .is_some_and(|r| r.contains("captured.example")),
        "memory adapter relay_url must contain captured relay 'captured.example', \
         got: {memory_relay_seen:?}"
    );

    // Memory event's `p` tag must equal the CAPTURED owner's pubkey hex.
    let p_tag_seen = memory_owner_p_tag.lock().unwrap().clone();
    assert_eq!(
        p_tag_seen.as_deref(),
        Some(captured_owner_pubkey_hex.as_str()),
        "engram event p-tag (owner counterpart/coordinate) must equal the captured \
         owner's pubkey, not the post-switch owner; got: {p_tag_seen:?}"
    );
}

/// Extract the first `p` tag value from a nostr event JSON byte slice.
///
/// Returns `Some(hex_pubkey)` if a `["p", "<hex>"]` tag entry is found,
/// `None` if the JSON cannot be parsed or has no `p` tag.
fn extract_p_tag_from_event_json(event_json: &[u8]) -> Option<String> {
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

/// `before_store` hook advances scope generation — Phase 3a must reject with
/// a generation mismatch error BEFORE any write.
///
/// This is the identity-switch-before-store-is-rejected test. `before_store`
/// fires after entry capture and before lock acquisition — simulating a
/// concurrent workspace switch that arrived after `capture_agent_snapshot_import_entry`
/// returned but before Phase 3a acquired the store lock.
#[tokio::test]
// SAFETY: single-threaded tokio runtime; lock held to serialize generation
// counter mutations — cannot deadlock. See sister test for full rationale.
#[allow(clippy::await_holding_lock)]
async fn test_agent_identity_switch_before_store_is_rejected() {
    // Serialize against the after_store test to prevent the generation bump
    // inside before_store from racing Phase 3a of the after_store test.
    let _gen_guard = GENERATION_TEST_LOCK.lock().unwrap();

    use crate::commands::personas::snapshot::import::{
        confirm_agent_snapshot_import_core, AgentSnapshotImportConfirm, MemoryPublish,
        ProfilePublish,
    };
    use crate::managed_agents::scope::next_scope_generation;
    use tauri::Manager;

    let tmp = tempfile::tempdir().unwrap();
    let (app, _owner_keys) = setup_import_app_with_scope(&tmp);
    let handle = app.handle();
    let state = app.state::<crate::app_state::AppState>();

    let file_bytes = minimal_agent_snapshot_json("TestAgent");
    let input = AgentSnapshotImportConfirm {
        file_bytes,
        keep_allowlist: false,
    };

    let result = confirm_agent_snapshot_import_core(
        input,
        handle,
        &state,
        move || {
            // before_store: advance generation — simulates a workspace switch
            // that raced the import after entry capture but before Phase 3a lock.
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
        "pre-store switch must cause Phase 3a rejection"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("stale") || err.contains("generation") || err.contains("mismatch"),
        "error must describe generation mismatch: {err}"
    );
}
