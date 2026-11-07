use super::{lift_instance_acp_commands_in_dir, SENTINEL};
use crate::managed_agents::ManagedAgentRecord;
use crate::migration::test_support::write_agents_json;
use std::path::Path;

fn record_json(name: &str, pubkey: &str, acp_command: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "pubkey": pubkey,
        "relay_url": "ws://localhost:3000",
        "acp_command": acp_command,
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "parallelism": 4,
        "system_prompt": "You are Janet.",
        "model": null,
        "provider": null,
        "respond_to": "owner-only",
        "env_vars": {},
        "start_on_app_launch": true,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    })
}

fn definition_json(slug: &str, acp_command: &str) -> serde_json::Value {
    let mut value = record_json(slug, "", acp_command);
    value["slug"] = serde_json::json!(slug);
    value
}

fn instance_json(name: &str, pubkey: &str, slug: &str, acp_command: &str) -> serde_json::Value {
    let mut value = record_json(name, pubkey, acp_command);
    value["persona_id"] = serde_json::json!(slug);
    value
}

fn base(dir: &Path) -> std::path::PathBuf {
    dir.join("agents")
}

fn load(dir: &Path) -> Vec<ManagedAgentRecord> {
    let content = std::fs::read_to_string(base(dir).join("managed-agents.json")).unwrap();
    serde_json::from_str(&content).unwrap()
}

fn definition<'a>(records: &'a [ManagedAgentRecord], slug: &str) -> &'a ManagedAgentRecord {
    records
        .iter()
        .find(|r| r.pubkey.is_empty() && r.slug.as_deref() == Some(slug))
        .unwrap()
}

fn version(record: &ManagedAgentRecord) -> String {
    super::definition_version(record).unwrap()
}

#[test]
fn lifts_instance_only_custom_command_into_stock_definition() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "b".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition_json("janet", "buzz-acp"),
            instance_json("Bad Janet", &pubkey, "janet", "buzz-janet-acp"),
        ]),
    );
    // Instance was in sync with the pre-lift definition: no drift after either.
    let mut records = load(dir.path());
    let before = version(definition(&records, "janet"));
    records
        .iter_mut()
        .find(|r| !r.pubkey.is_empty())
        .unwrap()
        .persona_source_version = Some(before.clone());
    std::fs::write(
        base(dir.path()).join("managed-agents.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();

    assert_eq!(
        lift_instance_acp_commands_in_dir(&base(dir.path())).unwrap(),
        1
    );

    let records = load(dir.path());
    let lifted = definition(&records, "janet");
    assert_eq!(lifted.acp_command, "buzz-janet-acp");
    let view = lifted.to_definition_view().unwrap();
    assert_eq!(view.acp_command.as_deref(), Some("buzz-janet-acp"));
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.acp_command, "buzz-janet-acp", "instance untouched");
    let after = version(lifted);
    assert_ne!(before, after);
    assert_eq!(
        instance.persona_source_version.as_deref(),
        Some(after.as_str())
    );
    assert!(base(dir.path())
        .join("managed-agents.json.pre-acp-lift.bak")
        .exists());
    assert!(base(dir.path()).join(SENTINEL).exists());
}

#[test]
fn stale_instance_version_is_not_advanced() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "b".repeat(64);
    let mut instance = instance_json("Bad Janet", &pubkey, "janet", "buzz-janet-acp");
    instance["persona_source_version"] = serde_json::json!("older-hash");
    write_agents_json(
        dir.path(),
        &serde_json::json!([definition_json("janet", "buzz-acp"), instance]),
    );

    assert_eq!(
        lift_instance_acp_commands_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load(dir.path());
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(
        instance.persona_source_version.as_deref(),
        Some("older-hash")
    );
}

#[test]
fn definition_with_custom_command_is_never_modified() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition_json("janet", "my-wrapper-acp"),
            instance_json("Bad Janet", &"b".repeat(64), "janet", "buzz-janet-acp"),
        ]),
    );

    assert_eq!(
        lift_instance_acp_commands_in_dir(&base(dir.path())).unwrap(),
        0
    );
    assert_eq!(
        definition(&load(dir.path()), "janet").acp_command,
        "my-wrapper-acp"
    );
    assert!(!base(dir.path())
        .join("managed-agents.json.pre-acp-lift.bak")
        .exists());
}

#[test]
fn conflicting_instance_commands_leave_definition_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition_json("janet", "buzz-acp"),
            instance_json("One", &"b".repeat(64), "janet", "buzz-janet-acp"),
            instance_json("Two", &"c".repeat(64), "janet", "other-acp"),
            // A stock sibling does not count as a conflict.
            instance_json("Three", &"d".repeat(64), "janet", "buzz-acp"),
        ]),
    );

    assert_eq!(
        lift_instance_acp_commands_in_dir(&base(dir.path())).unwrap(),
        0
    );
    assert_eq!(
        definition(&load(dir.path()), "janet").acp_command,
        "buzz-acp"
    );
}

#[test]
fn runs_once_so_a_later_reset_to_stock_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition_json("janet", "buzz-acp"),
            instance_json("Bad Janet", &"b".repeat(64), "janet", "buzz-janet-acp"),
        ]),
    );
    std::fs::write(base(dir.path()).join(SENTINEL), "").unwrap();

    assert_eq!(
        lift_instance_acp_commands_in_dir(&base(dir.path())).unwrap(),
        0
    );
    assert_eq!(
        definition(&load(dir.path()), "janet").acp_command,
        "buzz-acp"
    );
}

#[test]
fn missing_store_writes_sentinel_without_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base(dir.path())).unwrap();
    assert_eq!(
        lift_instance_acp_commands_in_dir(&base(dir.path())).unwrap(),
        0
    );
    assert!(base(dir.path()).join(SENTINEL).exists());
}
