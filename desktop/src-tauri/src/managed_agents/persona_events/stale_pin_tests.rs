//! Stale-pin drop tests for `apply_persona_snapshot`.
//!
//! Covers the `canonical_harness_command` resolver used to classify a
//! create-time `agent_command_override` before deciding whether it should be
//! dropped when the persona switches to a different harness.

use super::tests::{sample_persona, sample_record};
use crate::managed_agents::persona_events::apply_persona_snapshot;
use crate::managed_agents::types::AgentDefinition;

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
