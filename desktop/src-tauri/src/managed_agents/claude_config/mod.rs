//! Claude Code agent spawn-time env helpers.
//!
//! A1 contract: `ANTHROPIC_MODEL` is the single startup model authority for
//! local Claude Code agents. `BUZZ_ACP_MODEL` is removed from the spawned
//! env so the harness never sees two model authorities simultaneously.
//!
//! B5 contract: `BUZZ_ACP_EFFORT_LEVEL` is the canonical persisted startup
//! effort authority for all local agents. Written after `descriptor.env` so
//! user-supplied entries cannot shadow a persisted canonical value.

/// The spawn-time env var carrying startup effort. Shared by the spawn
/// application ([`apply_effort_env`]) and the snapshot projection
/// (`spawn_snapshot::effective_effort`) so the value the harness receives and
/// the value the restart badge compares are named from one place.
pub const EFFORT_LEVEL_ENV_VAR: &str = "BUZZ_ACP_EFFORT_LEVEL";

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

/// Apply the B5 effort authority: inject `BUZZ_ACP_EFFORT_LEVEL` from
/// `effort_level` (or leave it untouched if `None`).
///
/// Must be called after `descriptor.env` is written so the canonical persisted
/// value wins over any user-supplied `BUZZ_ACP_EFFORT_LEVEL` entry. When
/// `effort_level` is `None` there is no canonical value to assert; the command
/// env is left untouched so a user-supplied value from `descriptor.env`
/// legitimately seeds startup effort.
pub fn apply_effort_env(command: &mut std::process::Command, effort_level: Option<&str>) {
    if let Some(e) = effort_level {
        command.env(EFFORT_LEVEL_ENV_VAR, e);
    }
    // None: no canonical value — leave whatever descriptor.env wrote intact.
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
