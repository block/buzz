use super::{apply_claude_model_env, apply_effort_env};

/// A1: BUZZ_ACP_MODEL must NOT be present in the spawned-child env after
/// `apply_claude_model_env`, even if it was set before (dual-authority defect).
/// ANTHROPIC_MODEL must be set to the resolved model.
#[test]
fn a1_buzz_acp_model_absent_anthropic_model_present_after_env_apply() {
    let mut cmd = std::process::Command::new("true");
    // Simulate descriptor.env writing BUZZ_ACP_MODEL (the pre-A1 path).
    cmd.env("BUZZ_ACP_MODEL", "claude-opus-4");
    apply_claude_model_env(&mut cmd, Some("claude-opus-4"));

    let env_map: std::collections::HashMap<_, _> = cmd.get_envs().collect();

    // BUZZ_ACP_MODEL must be removed. Command::get_envs returns None for
    // explicitly-removed keys.
    let buzz_acp = env_map.get(std::ffi::OsStr::new("BUZZ_ACP_MODEL"));
    assert!(
        buzz_acp.is_none() || buzz_acp.unwrap().is_none(),
        "BUZZ_ACP_MODEL must be absent (or explicitly removed) after A1 policy"
    );

    // ANTHROPIC_MODEL must be set to the resolved model value.
    let anthropic = env_map.get(std::ffi::OsStr::new("ANTHROPIC_MODEL"));
    assert!(anthropic.is_some(), "ANTHROPIC_MODEL must be present");
    assert_eq!(
        anthropic.unwrap().unwrap_or_default(),
        "claude-opus-4",
        "ANTHROPIC_MODEL must equal the effective model"
    );
}

/// A1: when no model is resolved, ANTHROPIC_MODEL must be removed so Claude
/// uses its own default rather than inheriting a stale env value.
#[test]
fn a1_anthropic_model_removed_when_no_effective_model() {
    let mut cmd = std::process::Command::new("true");
    // Pre-set a stale value that might have leaked in.
    cmd.env("ANTHROPIC_MODEL", "claude-3-5-sonnet");
    cmd.env("BUZZ_ACP_MODEL", "claude-3-5-sonnet");
    apply_claude_model_env(&mut cmd, None);

    let env_map: std::collections::HashMap<_, _> = cmd.get_envs().collect();

    let anthropic = env_map.get(std::ffi::OsStr::new("ANTHROPIC_MODEL"));
    assert!(
        anthropic.is_none() || anthropic.unwrap().is_none(),
        "ANTHROPIC_MODEL must be absent when no effective model"
    );
    let buzz_acp = env_map.get(std::ffi::OsStr::new("BUZZ_ACP_MODEL"));
    assert!(
        buzz_acp.is_none() || buzz_acp.unwrap().is_none(),
        "BUZZ_ACP_MODEL must always be absent after A1 policy"
    );
}

// ── B5 effort-authority contract tests ──────────────────────────────────────
//
// These tests verify that `apply_effort_env`, called after `descriptor.env`,
// makes the canonical persisted effort win over any user-supplied value.

/// B5 (local): canonical effort wins when user env supplies a conflicting value.
/// Simulates the defect scenario: descriptor.env wrote BUZZ_ACP_EFFORT_LEVEL=low,
/// then apply_effort_env is called with the canonical "high". The canonical value
/// must be what survives in the spawned-child env.
#[test]
fn b5_canonical_effort_wins_over_user_env_collision() {
    let mut cmd = std::process::Command::new("true");
    // Simulate descriptor.env writing a user-supplied value (the pre-fix
    // ordering: effort written before the loop, then loop overwrote it, or
    // equivalently: effort written post-loop but with user value also post-loop).
    cmd.env("BUZZ_ACP_EFFORT_LEVEL", "low");

    // Post-loop canonical application — the fix.
    apply_effort_env(&mut cmd, Some("high"));

    let env_map: std::collections::HashMap<_, _> = cmd.get_envs().collect();
    let effort = env_map.get(std::ffi::OsStr::new("BUZZ_ACP_EFFORT_LEVEL"));
    assert!(effort.is_some(), "BUZZ_ACP_EFFORT_LEVEL must be present");
    assert_eq!(
        effort.unwrap().unwrap_or_default(),
        "high",
        "canonical effort must win over the user-supplied 'low' — B5 authority ordering"
    );
}

/// B5 (local): when no canonical effort is persisted (effort_level is None),
/// user env passthrough is preserved — the descriptor.env entry seeds startup effort.
/// Simulates: descriptor.env wrote BUZZ_ACP_EFFORT_LEVEL=low (already in command),
/// then apply_effort_env(None) is called — user value must survive.
#[test]
fn b5_user_effort_env_survives_when_no_canonical_value() {
    let mut cmd = std::process::Command::new("true");
    // Simulate descriptor.env loop having written a user-supplied value first.
    cmd.env("BUZZ_ACP_EFFORT_LEVEL", "low");

    // No canonical value — apply_effort_env(None) is a no-op so the user
    // value already written by the descriptor.env loop survives intact.
    apply_effort_env(&mut cmd, None);

    let env_map: std::collections::HashMap<_, _> = cmd.get_envs().collect();
    let effort = env_map.get(std::ffi::OsStr::new("BUZZ_ACP_EFFORT_LEVEL"));
    assert!(effort.is_some(), "BUZZ_ACP_EFFORT_LEVEL must be present");
    assert_eq!(
        effort.unwrap().unwrap_or_default(),
        "low",
        "user-supplied effort must survive when no canonical value is persisted"
    );
}

/// B5 (local): canonical effort is present in the spawned env even when user
/// env did NOT supply a conflicting value (basic injection contract).
#[test]
fn b5_canonical_effort_injected_when_no_user_collision() {
    let mut cmd = std::process::Command::new("true");
    // No user-supplied BUZZ_ACP_EFFORT_LEVEL in descriptor.env.
    apply_effort_env(&mut cmd, Some("medium"));

    let env_map: std::collections::HashMap<_, _> = cmd.get_envs().collect();
    let effort = env_map.get(std::ffi::OsStr::new("BUZZ_ACP_EFFORT_LEVEL"));
    assert!(effort.is_some(), "BUZZ_ACP_EFFORT_LEVEL must be present");
    assert_eq!(
        effort.unwrap().unwrap_or_default(),
        "medium",
        "canonical effort must be injected when no collision"
    );
}
