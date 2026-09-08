//! F1 production-seam regression: a signed kind:30178 catalog head driven
//! through the REAL inbound entrypoint `reconcile_inbound_persona_event_blocking`
//! must land an arrival-scoped retained witness and queue no outbound publish.
//!
//! Unlike the `retain_inbound_catalog_witness` unit tests, this drives the whole
//! production dispatcher over a `MockRuntime` `AppHandle` — the same fn the live
//! inbound subscription calls. Neutralizing the catalog routing decision inside
//! the reconcile (an early return for `KIND_TEAM_CATALOG` before the production
//! invocation) turns this test RED; that reversal is what proves the seam is the
//! production path and not a test-only shim.

use super::reconcile_inbound_persona_event_blocking;
use crate::app_state::build_app_state;
use crate::managed_agents::retention::{
    get_pending_sync, get_retained_event, open_retention_db, scoped_retention_db_path,
};
use crate::managed_agents::team_catalog::build_team_catalog_event;
use crate::managed_agents::{
    agent_events::build_agent_event, load_managed_agents, save_managed_agents, AgentDefinition,
    AgentKeyCustody, BackendKind, ManagedAgentRecord, RespondTo, TeamRecord,
};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
use nostr::JsonUtil;
use std::collections::BTreeMap;
use std::path::PathBuf;

const RELAY: &str = "wss://catalog-seam.example";
const TEAM_ID: &str = "team-seam";

fn member(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: display_name.to_string(),
        description: None,
        avatar_url: None,
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
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn team() -> TeamRecord {
    TeamRecord {
        id: TEAM_ID.to_string(),
        name: "Seam Team".to_string(),
        description: Some("A shared team".to_string()),
        instructions: None,
        persona_ids: vec!["m1".to_string(), "m2".to_string()],
        is_builtin: false,
        shared: true,
        catalog_source: None,
        source_dir: Some(PathBuf::from("/local/only/path")),
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

/// Build a mock `AppHandle` whose `app_data_dir` resolves under the overridden
/// `$HOME`/`$XDG_DATA_HOME`, wired with `keys` as the signing identity and
/// `RELAY` as the active workspace.
///
/// On desktop Tauri resolves `app_data_dir` from `dirs::data_dir()`, which reads
/// `$HOME` (macOS) / `$XDG_DATA_HOME` (Linux). The caller holds the path mutex
/// and overrides both so this handle's retention scope lands inside the tempdir.
fn mock_app(keys: &nostr::Keys) -> tauri::App<tauri::test::MockRuntime> {
    let state = build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(RELAY.to_string());

    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds headless")
}

/// A signed kind:30178 catalog head for `team()`, exactly as another device
/// would publish it: signed by the owner, `shared` tag set.
fn signed_catalog_head(keys: &nostr::Keys) -> nostr::Event {
    build_team_catalog_event(&team(), &[member("m1", "One"), member("m2", "Two")], true)
        .expect("catalog event builds")
        .sign_with_keys(keys)
        .expect("catalog event signs")
}

#[test]
fn inbound_catalog_head_retains_arrival_witness_through_the_production_reconcile() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &home);

    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let event = signed_catalog_head(&keys);

    let app = mock_app(&keys);
    // The arrival scope is resolved from the handle's app_data_dir; capture the
    // same path the production reconcile writes to so the assertions read the
    // exact database the seam touched.
    let base_dir = crate::managed_agents::managed_agents_base_dir(app.handle())
        .expect("resolve managed agents base dir");
    let db_path = scoped_retention_db_path(&base_dir, RELAY, &owner);

    let refresh = reconcile_inbound_persona_event_blocking(
        event.as_json(),
        RELAY.to_string(),
        app.handle().clone(),
    )
    .expect("reconcile of a signed 30178 head must succeed");

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

    assert!(
        refresh.is_none(),
        "a catalog head carries no local record — reconcile must return no runtime refresh"
    );

    let conn = open_retention_db(&db_path).unwrap();
    let witness = get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, TEAM_ID)
        .unwrap()
        .expect("the production reconcile must retain the arrival witness");
    assert!(
        !witness.pending_sync,
        "an inbound witness is already on the relay — it must not be queued for publish"
    );
    assert_eq!(
        witness.raw_event,
        event.as_json(),
        "the retained witness must be the arriving head verbatim"
    );
    assert!(
        get_pending_sync(&conn).unwrap().is_empty(),
        "retaining an inbound catalog head must queue no outbound publication (no ping-pong)"
    );
}

#[test]
fn inbound_policy_edit_never_queues_legacy_deploy_for_provider_custody() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &home);

    let keys = nostr::Keys::generate();
    let app = mock_app(&keys);
    let agent_pubkey = "ab".repeat(32);
    let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": agent_pubkey,
        "name": "Provider Agent",
        "relay_url": RELAY,
        "acp_command": "",
        "agent_command": "",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 0,
        "system_prompt": null,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    }))
    .expect("minimal managed-agent record parses");
    record.key_custody = AgentKeyCustody::Provider;
    record.backend = BackendKind::Provider {
        id: "provider".to_string(),
        config: serde_json::json!({}),
    };
    record.backend_agent_id = Some("remote-agent".to_string());
    record.respond_to = RespondTo::OwnerOnly;
    save_managed_agents(app.handle(), &[record.clone()]).expect("seed provider agent");

    let mut inbound = record;
    inbound.respond_to = RespondTo::Anyone;
    let event = build_agent_event(&inbound)
        .expect("managed-agent event builds")
        .sign_with_keys(&keys)
        .expect("managed-agent event signs");

    let refresh = reconcile_inbound_persona_event_blocking(
        event.as_json(),
        RELAY.to_string(),
        app.handle().clone(),
    )
    .expect("provider-custodied inbound policy edit reconciles");
    let saved = load_managed_agents(app.handle()).expect("reload managed agents");

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

    assert!(
        refresh.is_none(),
        "provider custody must not select the legacy provider deploy path"
    );
    assert_eq!(saved[0].respond_to, RespondTo::Anyone);
    assert!(
        !saved[0].provider_policy_pending,
        "provider custody must not persist an unrecoverable legacy deploy marker"
    );
}
