//! What to look for when probing another machine for agent harnesses.
//!
//! A child module of `discovery` rather than new lines inside `discovery.rs`:
//! that file is already over the desktop 1000-line limit and carries a
//! documented "queued to be split" override, so new surface goes beside it. As
//! a child it still sees `discovery`'s private tables directly, so nothing had
//! to be made more visible to accommodate the move.
//!
//! The projection direction matters. These targets are derived from the same
//! compiled-in tables local discovery uses (`KNOWN_ACP_RUNTIMES` and
//! `PRESET_HARNESSES`), never from a parallel list. A hand-maintained set of
//! "harnesses we can find remotely" would drift the moment a preset is added —
//! the exact failure `preset_harness_ids()` already exists to prevent.

use super::{KNOWN_ACP_RUNTIMES, PRESET_HARNESSES};
use crate::managed_agents::types::HarnessSource;

/// One harness's probe target set, projected from the compiled-in tables.
///
/// Remote discovery needs to know *what to look for* on another machine. That
/// set must come from the same tables local discovery uses — a second,
/// hand-maintained list of harnesses would drift the moment a preset is added,
/// which is the failure mode `preset_harness_ids()` already exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessProbeTarget {
    pub id: &'static str,
    pub label: &'static str,
    /// ACP command basenames to look for, in preference order. The first one
    /// found on the remote host wins.
    pub acp_commands: &'static [&'static str],
    /// Vendor CLI the ACP command wraps, when the harness is an adapter.
    /// `None` when the ACP command *is* the vendor CLI.
    pub underlying_cli: Option<&'static str>,
    pub install_hint: &'static str,
    pub install_instructions_url: &'static str,
    pub source: HarnessSource,
}

/// Every harness a remote host can be probed for: the four builtins plus every
/// bundled preset.
///
/// Custom (tier-3) harnesses are deliberately excluded. Their definitions live
/// in the *local* user's `custom_harnesses/` directory and describe commands on
/// the local machine; projecting them onto a remote host would assert a layout
/// nothing has verified. A user who wants a custom harness discovered remotely
/// is better served by it becoming a preset.
pub fn harness_probe_targets() -> Vec<HarnessProbeTarget> {
    let mut targets: Vec<HarnessProbeTarget> = KNOWN_ACP_RUNTIMES
        .iter()
        .map(|runtime| HarnessProbeTarget {
            id: runtime.id,
            label: runtime.label,
            acp_commands: runtime.commands,
            underlying_cli: runtime.underlying_cli,
            // Builtins carry separate CLI and adapter hints. The CLI hint is the
            // useful one for a remote host: an absent adapter is only reachable
            // once the vendor CLI it wraps is present.
            install_hint: runtime.cli_install_hint,
            install_instructions_url: runtime.cli_install_instructions_url,
            source: HarnessSource::Builtin,
        })
        .collect();

    targets.extend(PRESET_HARNESSES.iter().map(|preset| HarnessProbeTarget {
        id: preset.id,
        label: preset.label,
        // A preset's `command` is the binary; its `args` are how it is invoked.
        // Only the binary is probeable, matching the local PATH probe.
        acp_commands: std::slice::from_ref(&preset.command),
        underlying_cli: preset.underlying_cli,
        install_hint: preset.install_hint,
        install_instructions_url: preset.install_instructions_url,
        source: HarnessSource::Preset,
    }));

    targets
}
