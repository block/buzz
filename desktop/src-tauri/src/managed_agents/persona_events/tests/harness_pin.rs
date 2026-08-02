//! `apply_persona_snapshot`: the harness pin is backend-scoped.
//!
//! A provider-backed record's `agent_command_override` names a harness on the
//! remote host, so the "is this a known local runtime?" test that clears a
//! stale local pin cannot apply to it. These cases pin that asymmetry.

use super::*;

fn snapshot_record(backend: BackendKind) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "p".repeat(64),
        name: "agent".into(),
        persona_id: Some("test-persona".into()),
        private_key_nsec: "nsec1fake".into(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".into(),
        avatar_url: None,
        acp_command: "buzz-acp".into(),
        agent_command: "claude-code-acp".into(),
        // A create-time pin naming a known runtime — the shape the clear targets.
        agent_command_override: Some("claude-code-acp".into()),
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: Some("You are a test agent.".into()),
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: Some("goose".into()),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        shared: false,
        catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    }
}

/// Local records keep today's behaviour: a pin naming a runtime the definition
/// no longer asks for is stale local preference and is dropped.
#[test]
fn local_record_drops_a_stale_known_runtime_pin() {
    let mut record = snapshot_record(BackendKind::Local);
    let persona = sample_persona(); // runtime = "goose"

    apply_persona_snapshot(&mut record, &persona);

    assert_eq!(
        record.agent_command_override, None,
        "a local pin diverging from the definition runtime is stale"
    );
}

/// Provider records must NOT be cleared. The pin is the only channel by which
/// the harness chosen from the remote host's catalog reaches that host; the
/// comparison above is against the LOCAL registry, which knows nothing about
/// the remote machine. Clearing would re-resolve the record to a locally-known
/// command — ultimately `default_agent_command()` = "buzz-agent" — and the next
/// deploy would silently provision the wrong harness.
#[test]
fn provider_record_keeps_its_remote_harness_pin() {
    let mut record = snapshot_record(BackendKind::Provider {
        id: "ssh".into(),
        config: serde_json::json!({ "host": "example" }),
    });
    let persona = sample_persona(); // runtime = "goose", diverges from the pin

    apply_persona_snapshot(&mut record, &persona);

    assert_eq!(
        record.agent_command_override.as_deref(),
        Some("claude-code-acp"),
        "a remote harness pin must survive persona reconciliation"
    );
}

/// The rest of the snapshot still applies to a provider record — only the
/// harness fields are exempt.
#[test]
fn provider_record_still_takes_the_definition_quad() {
    let mut record = snapshot_record(BackendKind::Provider {
        id: "ssh".into(),
        config: serde_json::Value::Null,
    });
    let persona = sample_persona();

    apply_persona_snapshot(&mut record, &persona);

    assert_eq!(record.model.as_deref(), Some("claude-opus-4"));
    assert_eq!(record.provider.as_deref(), Some("anthropic"));
}

/// `record.runtime` is harness state, scoped exactly like the pin above. It
/// names the harness the deploy resolves once no pin is present, and the
/// definition's id was chosen against the LOCAL catalog — so a persona sync
/// must not redirect a remote agent's harness with it.
#[test]
fn provider_record_keeps_its_runtime_through_a_persona_sync() {
    let mut record = snapshot_record(BackendKind::Provider {
        id: "ssh".into(),
        config: serde_json::json!({ "host": "example" }),
    });
    record.runtime = Some("claude".into());
    let persona = sample_persona(); // runtime = "goose", a local id

    apply_persona_snapshot(&mut record, &persona);

    assert_eq!(
        record.runtime.as_deref(),
        Some("claude"),
        "a local snapshot must not overwrite a remote record's harness"
    );
}

/// The local half of the same rule: a local record still mirrors the
/// definition, so a definition edit propagates on the next spawn.
#[test]
fn local_record_mirrors_the_definition_runtime() {
    let mut record = snapshot_record(BackendKind::Local);
    record.runtime = Some("claude".into());
    let persona = sample_persona(); // runtime = "goose"

    apply_persona_snapshot(&mut record, &persona);

    assert_eq!(record.runtime.as_deref(), Some("goose"));
}
