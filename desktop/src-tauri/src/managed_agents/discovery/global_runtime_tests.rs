use super::tests::{persona_with_runtime, record_with};

#[test]
fn inherited_global_harness_changes_without_persisting_a_pin() {
    use crate::managed_agents::{resolve_effective_harness_descriptor, GlobalAgentConfig};
    let record = record_with(None, Some("p1"), None);
    let personas = vec![persona_with_runtime("p1", None)];
    for (runtime, command) in [("codex", "codex-acp"), ("claude", "claude-agent-acp")] {
        let global = GlobalAgentConfig {
            preferred_runtime: Some(runtime.into()),
            ..Default::default()
        };
        let resolved = resolve_effective_harness_descriptor(&record, &personas, &global).unwrap();
        assert_eq!(resolved.command, command);
        assert!(record.runtime.is_none());
        assert!(personas[0].runtime.is_none());
    }
}

#[test]
fn explicit_harness_still_wins_over_global_default() {
    use crate::managed_agents::{resolve_effective_harness_descriptor, GlobalAgentConfig};
    let global = GlobalAgentConfig {
        preferred_runtime: Some("claude".into()),
        ..Default::default()
    };
    let personas = vec![persona_with_runtime("p1", Some("codex"))];
    let record = record_with(None, Some("p1"), None);
    assert_eq!(
        resolve_effective_harness_descriptor(&record, &personas, &global)
            .unwrap()
            .command,
        "codex-acp"
    );
    let pinned = record_with(None, Some("p1"), Some("goose"));
    assert_eq!(
        resolve_effective_harness_descriptor(&pinned, &personas, &global)
            .unwrap()
            .command,
        "goose"
    );
}

#[test]
fn record_agent_command_legacy_persona_fallback() {
    // Pre-migration record: persona_id set, no runtime — resolves through
    // the legacy persona path unchanged.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let record = record_with(None, Some("p1"), None);
    assert_eq!(super::record_agent_command(&record, &personas), "goose");
}
