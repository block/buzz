//! Stale-pin drop tests for `apply_persona_snapshot`.
//!
//! Covers the `canonical_harness_command` resolver used to classify a
//! create-time `agent_command_override` before deciding whether it should be
//! dropped when the persona switches to a different harness.

use super::tests::{sample_persona, sample_record};
use crate::managed_agents::persona_events::apply_persona_snapshot;
use crate::managed_agents::types::{AgentDefinition, ManagedAgentRecord};
use crate::managed_agents::{
    create_time_agent_command_override, effective_agent_command, record_agent_command,
};

// ── Stale-pin drop: OpenClaw↔Goose (preset↔builtin) ─────────────────────────

/// Persona→OpenClaw: stale Goose override dropped.
/// Regression for the original preset stale-pin fix.
#[test]
fn apply_persona_snapshot_goose_to_openclaw_drops_stale_goose_pin() {
    let mut record = sample_record();
    record.agent_command_override = Some("goose".to_string());
    apply_persona_snapshot(
        &mut record,
        &AgentDefinition {
            runtime: Some("openclaw".to_string()),
            ..sample_persona()
        },
    );
    assert_eq!(
        record.agent_command_override, None,
        "stale goose pin must be dropped when persona switches to openclaw"
    );
}

/// Persona→Goose: stale OpenClaw override dropped.
#[test]
fn apply_persona_snapshot_openclaw_to_goose_drops_stale_openclaw_pin() {
    let mut record = sample_record();
    record.agent_command_override = Some("openclaw".to_string());
    apply_persona_snapshot(
        &mut record,
        &AgentDefinition {
            runtime: Some("goose".to_string()),
            ..sample_persona()
        },
    );
    assert_eq!(
        record.agent_command_override, None,
        "stale openclaw pin must be dropped when persona switches to goose"
    );
}

// ── Stale-pin drop: alias pin (command ≠ id) ─────────────────────────────────

/// Persona→OpenClaw; record has a stale `claude-code-acp` alias pin (id="claude",
/// command="claude-agent-acp"). The canonical resolver must recognise the alias
/// as the Claude harness and drop it when the persona switches to a different
/// harness (OpenClaw).
///
/// This is the correctness case that motivated the `canonical_harness_command`
/// resolver: the old pointer-comparison code treated the alias as a
/// custom/unknown pin and kept it — the agent kept running Claude instead of
/// OpenClaw.
#[test]
fn apply_persona_snapshot_claude_alias_pin_to_openclaw_drops_stale_alias() {
    let mut record = sample_record();
    // "claude-code-acp" is an alias of the Claude runtime (id="claude").
    record.agent_command_override = Some("claude-code-acp".to_string());
    apply_persona_snapshot(
        &mut record,
        &AgentDefinition {
            runtime: Some("openclaw".to_string()),
            ..sample_persona()
        },
    );
    assert_eq!(
        record.agent_command_override, None,
        "stale claude-code-acp alias pin must be dropped when persona switches to openclaw"
    );
}

// ── Stale-pin keep: same harness, path/alias override ───────────────────────

/// Same-harness case: record has an explicit path override pointing at the same
/// harness as the new persona runtime. The pin must NOT be dropped — it is a
/// deliberate per-instance configuration (e.g. a specific goose binary path).
#[test]
fn apply_persona_snapshot_same_harness_path_pin_is_kept() {
    let mut record = sample_record();
    // Explicit path override for goose — same harness as the persona runtime.
    record.agent_command_override = Some("/usr/local/bin/goose".to_string());
    apply_persona_snapshot(
        &mut record,
        &AgentDefinition {
            runtime: Some("goose".to_string()),
            ..sample_persona()
        },
    );
    assert_eq!(
        record.agent_command_override.as_deref(),
        Some("/usr/local/bin/goose"),
        "same-harness path override must NOT be dropped"
    );
}

// ── Stale-pin drop: builtin pin → loaded custom harness (tier-1→tier-3) ──────

/// Persona→CustomHarness: stale Goose override dropped.
///
/// This is the custom-direction regression: before `canonical_harness_command`
/// the destination lookup (`known_acp_runtime_exact`) only saw the four
/// tier-1 builtins, so a switch to a loaded custom harness left any stale
/// builtin pin authoritative.
///
/// Tier-3 (loaded custom harness) is reached via `lookup_loaded_harness_by_id`,
/// which reads the in-process registry — so we must populate it via
/// `update_loaded_harness_registry` under `registry_test_lock()`.
#[test]
fn apply_persona_snapshot_goose_to_custom_harness_drops_stale_goose_pin() {
    use crate::managed_agents::custom_harnesses::{
        registry_test_lock, update_loaded_harness_registry, HarnessDefinition,
    };
    use std::collections::BTreeMap;

    let _lock = registry_test_lock();

    // Register a custom harness definition so the resolver finds it at tier 3.
    update_loaded_harness_registry(vec![HarnessDefinition {
        id: "my-custom-harness".to_string(),
        label: "My Custom Harness".to_string(),
        command: "my-custom-bin".to_string(),
        args: vec![],
        env: BTreeMap::new(),
        install_instructions_url: String::new(),
        install_hint: String::new(),
    }]);

    let mut record = sample_record();
    record.agent_command_override = Some("goose".to_string());
    apply_persona_snapshot(
        &mut record,
        &AgentDefinition {
            runtime: Some("my-custom-harness".to_string()),
            ..sample_persona()
        },
    );
    assert_eq!(
        record.agent_command_override, None,
        "stale goose pin must be dropped when persona switches to a loaded custom harness"
    );

    // Clean up the registry so parallel tests start from a known state.
    update_loaded_harness_registry(vec![]);
}

// ── Deliberate create-time pin: create → storage → persona edit → spawn ──────

const PINNED_ARGS: [&str; 2] = ["--profile", "work"];

/// A record created with a deliberate pin, as read back from storage.
fn stored_record_pinned_to(pin: &str, persona: &AgentDefinition) -> ManagedAgentRecord {
    let mut record = sample_record();
    record.persona_id = Some(persona.id.clone());
    record.agent_command_override = create_time_agent_command_override(
        Some(persona.id.as_str()),
        std::slice::from_ref(persona),
        Some(pin),
        true,
    );
    record.agent_args = PINNED_ARGS.map(str::to_string).to_vec();
    let stored = serde_json::to_string(&record).expect("record serializes");
    serde_json::from_str(&stored).expect("record deserializes")
}

/// An edit that stays on the same harness keeps launching the exact pin with
/// the args configured for it, for both a full path and an alias.
#[test]
fn deliberate_pin_keeps_command_and_args_through_a_same_harness_persona_edit() {
    for (runtime, pin) in [
        ("goose", "/opt/homebrew/bin/goose"),
        ("claude", "claude-code-acp"),
    ] {
        let persona = AgentDefinition {
            runtime: Some(runtime.to_string()),
            ..sample_persona()
        };
        let mut record = stored_record_pinned_to(pin, &persona);
        let edited = AgentDefinition {
            display_name: "Renamed".to_string(),
            ..persona
        };

        apply_persona_snapshot(&mut record, &edited);

        assert_eq!(record_agent_command(&record, &[edited]), pin);
        assert_eq!(record.agent_args, PINNED_ARGS);
    }
}

/// A switch to a different known harness drops the pin, so the agent follows
/// the persona. The args configured for the old harness must not follow it.
#[test]
fn deliberate_pin_drops_with_its_args_when_the_persona_switches_harness() {
    for (runtime, pin) in [
        ("goose", "/opt/homebrew/bin/goose"),
        ("claude", "claude-code-acp"),
    ] {
        let persona = AgentDefinition {
            runtime: Some(runtime.to_string()),
            ..sample_persona()
        };
        let mut record = stored_record_pinned_to(pin, &persona);
        let switched = AgentDefinition {
            runtime: Some("openclaw".to_string()),
            ..persona
        };

        apply_persona_snapshot(&mut record, &switched);

        assert_eq!(record.agent_command_override, None);
        assert!(record.agent_args.is_empty());
        let personas = [switched];
        assert_eq!(
            record_agent_command(&record, &personas),
            effective_agent_command(Some(personas[0].id.as_str()), &personas, None)
        );
    }
}
