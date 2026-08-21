//! opencode runtime behavior: ACP-mode default args and the
//! `BUZZ_DEFAULT_RUNTIME` engine override resolution.

use super::resolve_default_runtime;
use crate::managed_agents::discovery::normalize_agent_args;

#[test]
fn opencode_defaults_to_acp_mode_args() {
    // Bare `opencode` starts the TUI; ACP needs the `acp` subcommand, so the
    // empty-args default must be `["acp"]` (goose pattern).
    assert_eq!(
        normalize_agent_args("opencode", Vec::new()),
        vec!["acp".to_string()]
    );
}

#[test]
fn resolve_default_runtime_accepts_known_harness_ids() {
    // Tier-1 builtin id and a raw command form both resolve; whitespace is
    // tolerated so operators can quote the value loosely.
    assert_eq!(resolve_default_runtime("opencode"), Some("opencode".into()));
    assert_eq!(resolve_default_runtime(" goose "), Some("goose".into()));
}

#[test]
fn resolve_default_runtime_rejects_empty_and_unknown_ids() {
    // Empty/blank keeps the bundled default silently; unknown ids do too but
    // are worth a warning — pinned agents must never dangle.
    assert_eq!(resolve_default_runtime(""), None);
    assert_eq!(resolve_default_runtime("   "), None);
    assert_eq!(resolve_default_runtime("definitely-not-a-harness"), None);
}
