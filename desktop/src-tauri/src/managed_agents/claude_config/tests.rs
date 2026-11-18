use super::apply_claude_model_env;

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

// ── B5 effort-authority contract ─────────────────────────────────────────────
//
// Startup-effort application moved out of this module into the single
// harness-agnostic projection (`config_bridge::effort`). Its authority,
// collision, and single-key contract is exercised by
// `config_bridge::effort::tests`; there is no longer a Claude-local effort
// helper to test here.
