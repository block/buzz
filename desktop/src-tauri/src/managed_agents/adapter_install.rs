//! Actionable errors for a missing ACP adapter binary.
//!
//! A runtime's ACP adapter (e.g. `claude-agent-acp`) is installed separately
//! from the bundled `buzz-acp` harness, so it can be absent on an otherwise
//! healthy install. Model discovery used to spawn the harness anyway and
//! surface the subprocess failure verbatim, which the desktop UI could not
//! distinguish from any other discovery error — the model field fell through
//! to "Could not load live models", with no hint about what to install.
//!
//! This module turns that case into a typed error carrying the runtime's own
//! install metadata.

use super::known_acp_runtime;

/// Prefix of the typed missing-adapter error produced by [`adapter_missing_error`].
///
/// Like `DANGLING_HARNESS_PREFIX`, this sentinel is a contract with a single
/// consumer — the desktop model-discovery status formatter
/// (`personaModelDiscoveryStatus.ts`), which strips the prefix and renders the
/// JSON payload as the runtime's install hint. It must never be shown raw.
pub const ADAPTER_MISSING_PREFIX: &str = "ADAPTER_MISSING:";

/// Typed error for "this runtime's ACP adapter binary is not installed".
///
/// Returns `None` when `command` is not a known ACP runtime, or when the
/// runtime ships no adapter to install (goose, buzz-agent) — those cases keep
/// the generic discovery failure they have today.
///
/// The payload carries the runtime's own catalog entries so the frontend
/// renders the install hint, command, and documentation link without holding a
/// second copy of the runtime catalog.
pub fn adapter_missing_error(command: &str) -> Option<String> {
    let runtime = known_acp_runtime(command)?;
    if runtime.adapter_install_commands.is_empty() {
        return None;
    }

    let payload = serde_json::json!({
        "runtimeId": runtime.id,
        "runtimeLabel": runtime.label,
        "hint": runtime.adapter_install_hint,
        "commands": runtime.adapter_install_commands,
        "url": runtime.adapter_install_instructions_url,
    });

    Some(format!("{ADAPTER_MISSING_PREFIX}{payload}"))
}
