//! Backend tests for the pin→inherit effort clear (PR #4625, plan-of-record
//! item 1): the sentinel transition clears the canonical column eagerly and the
//! update boundary strips the record effort env aliases AFTER caller `env_vars`
//! is applied. Split out of `discovery/tests.rs` to hold that file under the
//! desktop file-size ratchet.
//!
//! `use super::*` pulls the parent test module's helpers (`record_with`,
//! `persona_with_runtime`, `record_agent_command`) and its imported command
//! surface (`apply_agent_command_update`, `apply_env_vars_then_effort_transition`,
//! `remove_record_effort_aliases`).

use super::*;

#[test]
fn apply_agent_command_update_inherit_sentinel_clears_pin_runtime_and_column() {
    // Choosing Inherit on a persona-linked record clears the explicit pin, the
    // materialized runtime, AND the per-instance effort column, so resolution
    // falls through to the live definition immediately — not on the next spawn.
    // The transition flag fires so the caller strips the record effort env
    // aliases after `env_vars` is applied.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let mut record = record_with(Some("claude"), Some("p1"), Some("codex-acp"));
    record.effort_level = Some("high".into());

    let transition = apply_agent_command_update(&mut record, &personas, "", false);

    assert!(transition, "the pin→inherit transition must be signalled");
    assert_eq!(record.agent_command_override, None);
    assert_eq!(record.runtime, None);
    assert_eq!(
        record.effort_level, None,
        "the effort column must be cleared"
    );
    assert_eq!(record_agent_command(&record, &personas), "goose");
}

#[test]
fn apply_agent_command_update_sentinel_keeps_runtime_for_definition_less_record() {
    // For a record with no persona link the materialized runtime is the only
    // harness source left once the pin is cleared — a stray empty
    // agent_command must not change what the agent runs, nor clear its effort.
    let mut record = record_with(Some("claude"), None, Some("codex-acp"));
    record.effort_level = Some("high".into());

    let transition = apply_agent_command_update(&mut record, &[], "", false);

    assert!(
        !transition,
        "a definition-less stray sentinel is not a pin→inherit transition"
    );
    assert_eq!(record.agent_command_override, None);
    assert_eq!(record.runtime.as_deref(), Some("claude"));
    assert_eq!(
        record.effort_level.as_deref(),
        Some("high"),
        "a definition-less record must preserve its effort column"
    );
    assert_eq!(record_agent_command(&record, &[]), "claude-agent-acp");
}

#[test]
fn apply_agent_command_update_concrete_pin_keeps_materialized_runtime_and_column() {
    // A concrete pick only sets the pin; the materialized runtime and the
    // effort column are left intact (no ownership transition). The pin shadows
    // the runtime in resolution either way.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let mut record = record_with(Some("claude"), Some("p1"), None);
    record.effort_level = Some("high".into());

    let transition = apply_agent_command_update(&mut record, &personas, "codex-acp", true);

    assert!(
        !transition,
        "a concrete pin is not a pin→inherit transition"
    );
    assert_eq!(record.agent_command_override.as_deref(), Some("codex-acp"));
    assert_eq!(record.runtime.as_deref(), Some("claude"));
    assert_eq!(
        record.effort_level.as_deref(),
        Some("high"),
        "a concrete pin must preserve the effort column"
    );
    assert_eq!(record_agent_command(&record, &personas), "codex-acp");
}

#[test]
fn remove_record_effort_aliases_strips_all_known_and_legacy_keys() {
    // The update-boundary alias strip: after `env_vars` is applied on the
    // pin→inherit transition, every known native effort key and the legacy
    // alias must be removed, while unrelated env survives. This proves the
    // second half of the atomic clear that a helper-only column clear cannot.
    let mut env: std::collections::BTreeMap<String, String> = [
        ("GOOSE_THINKING_EFFORT", "high"),
        ("BUZZ_AGENT_THINKING_EFFORT", "high"),
        ("BUZZ_ACP_EFFORT_LEVEL", "high"),
        ("UNRELATED_KEY", "keep"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    remove_record_effort_aliases(&mut env);

    assert!(!env.contains_key("GOOSE_THINKING_EFFORT"));
    assert!(!env.contains_key("BUZZ_AGENT_THINKING_EFFORT"));
    assert!(!env.contains_key("BUZZ_ACP_EFFORT_LEVEL"));
    assert_eq!(
        env.get("UNRELATED_KEY").map(String::as_str),
        Some("keep"),
        "unrelated env must survive the effort-alias strip"
    );
}

#[test]
fn update_boundary_inherit_sentinel_with_alias_bearing_env_vars_strips_after_apply() {
    // The update-boundary ORDERING invariant (Thufir pass-3): on the pin→inherit
    // transition, a SAME-REQUEST `env_vars` map carrying a stale effort alias
    // must NOT survive. `apply_agent_command_update` clears the column eagerly;
    // then `apply_env_vars_then_effort_transition` applies the caller env FIRST
    // and strips the aliases AFTER — so the alias the request tried to
    // reintroduce is gone. A helper-only test cannot prove this order.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let mut record = record_with(Some("claude"), Some("p1"), Some("codex-acp"));
    record.effort_level = Some("high".into());

    let transition = apply_agent_command_update(&mut record, &personas, "", false);
    assert!(
        transition,
        "empty command on a persona-linked record is inherit"
    );

    // The request replaces env_vars with a map that re-pins effort via an alias
    // plus an unrelated key.
    let request_env: std::collections::BTreeMap<String, String> = [
        ("GOOSE_THINKING_EFFORT", "max"),
        ("BUZZ_ACP_EFFORT_LEVEL", "max"),
        ("UNRELATED_KEY", "keep"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    apply_env_vars_then_effort_transition(&mut record, Some(request_env), transition);

    assert_eq!(record.effort_level, None, "column stays cleared");
    assert!(
        !record.env_vars.contains_key("GOOSE_THINKING_EFFORT"),
        "same-request native alias must not survive the transition"
    );
    assert!(
        !record.env_vars.contains_key("BUZZ_ACP_EFFORT_LEVEL"),
        "same-request ACP sentinel must not survive the transition"
    );
    assert_eq!(
        record.env_vars.get("UNRELATED_KEY").map(String::as_str),
        Some("keep"),
        "unrelated env from the same request is preserved"
    );
}

#[test]
fn update_boundary_concrete_pin_preserves_alias_bearing_env_vars() {
    // No transition (concrete pin): the caller `env_vars` — including any effort
    // alias — is applied verbatim and NOT stripped. Effort env is only cleared
    // on the ownership transition, never on an ordinary env edit.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let mut record = record_with(Some("claude"), Some("p1"), None);

    let transition = apply_agent_command_update(&mut record, &personas, "codex-acp", true);
    assert!(!transition, "a concrete pin is not a transition");

    let request_env: std::collections::BTreeMap<String, String> =
        [("GOOSE_THINKING_EFFORT", "max")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

    apply_env_vars_then_effort_transition(&mut record, Some(request_env), transition);

    assert_eq!(
        record
            .env_vars
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("max"),
        "without a transition the caller effort env is preserved"
    );
}
