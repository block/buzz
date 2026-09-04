//! Claude Code agent spawn-time env helpers.
//!
//! A1 contract: `ANTHROPIC_MODEL` is the single startup model authority for
//! local Claude Code agents. `BUZZ_ACP_MODEL` is removed from the spawned
//! env so the harness never sees two model authorities simultaneously.
//!
//! Startup effort is no longer applied here: the harness-agnostic effort
//! projection (`config_bridge::effort`) runs inside the descriptor resolver, so
//! `descriptor.env` already carries exactly one effort key. See that module for
//! the single-authority contract, including the ACP-startup key constant.

/// Apply the A1 model authority: inject `ANTHROPIC_MODEL` from `effective_model`
/// (or remove it if `None`) and strip `BUZZ_ACP_MODEL` from the spawned env.
///
/// Must be called after `descriptor.env` is written so that any user-supplied
/// `ANTHROPIC_MODEL` is overridden by the Buzz-resolved value.
pub fn apply_claude_model_env(command: &mut std::process::Command, effective_model: Option<&str>) {
    // Remove BUZZ_ACP_MODEL — the catalog-switch path is for live ACP switches
    // only; at spawn time ANTHROPIC_MODEL is the sole authority.
    command.env_remove("BUZZ_ACP_MODEL");
    match effective_model {
        Some(m) => {
            command.env("ANTHROPIC_MODEL", m);
        }
        None => {
            command.env_remove("ANTHROPIC_MODEL");
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
