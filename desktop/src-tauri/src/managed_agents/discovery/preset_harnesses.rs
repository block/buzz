// ── Tier-2 preset harnesses ────────────────────────────────────────────────
//
// Static data for well-known ACP harnesses that have bundled logos and
// verified command/args. PATH-probed at discovery time (Detected badge);
// not editable or deletable by users. Logos are bundled assets referenced
// by id in the frontend `RUNTIME_LOGOS` map.

use std::path::PathBuf;

use super::normalize_agent_args;
use crate::managed_agents::{
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus, HarnessSource,
};

pub(crate) struct PresetHarness {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub install_instructions_url: &'static str,
    pub install_hint: &'static str,
    /// Vendor CLI the ACP command wraps, when the preset is an adapter
    /// (e.g. Amp's `amp-acp` wraps the separately-installed `amp` CLI).
    /// Consulted only when the adapter is absent, so `AdapterMissing`
    /// replaces the misleading `NotInstalled` when the CLI is present but
    /// the adapter is not. Deliberately NOT fed through the builtins'
    /// full `classify_runtime` predicate: that would flip
    /// adapter-present/CLI-absent from today's `Available` to `CliMissing`
    /// (unselectable), and presets carry a single flat `install_hint`, so
    /// the `CliMissing` copy would tell the user to install the adapter
    /// they already have. `None` when the command IS the vendor CLI.
    pub underlying_cli: Option<&'static str>,
}

/// Build the catalog entry for one preset harness through an injectable
/// resolver — the seam the preset loop consumes and tests bind.
///
/// Availability consumes only the adapter-missing arm of the builtin
/// predicate: adapter presence alone decides `Available` (exactly today's
/// behavior — an `amp-acp` without `amp` stays selectable), and
/// `underlying_cli` is consulted only when the adapter is absent, to
/// distinguish `AdapterMissing` (vendor CLI present) from `NotInstalled`
/// (neither found). See the `underlying_cli` field doc for why the full
/// `classify_runtime` predicate is deliberately not used here.
pub(crate) fn preset_catalog_entry(
    def: &PresetHarness,
    resolve: impl Fn(&str) -> Option<PathBuf>,
) -> AcpRuntimeCatalogEntry {
    let (availability, command, binary_path) = match resolve(def.command) {
        Some(path) => (
            AcpAvailabilityStatus::Available,
            Some(def.command.to_string()),
            Some(path.display().to_string()),
        ),
        None => {
            let underlying_cli_found = def
                .underlying_cli
                .map(|cli| resolve(cli).is_some())
                .unwrap_or(false);
            if underlying_cli_found {
                (AcpAvailabilityStatus::AdapterMissing, None, None)
            } else {
                (AcpAvailabilityStatus::NotInstalled, None, None)
            }
        }
    };
    let underlying_cli_path = def
        .underlying_cli
        .and_then(resolve)
        .map(|p| p.display().to_string());

    let default_args = normalize_agent_args(
        def.command,
        def.args.iter().map(|s| s.to_string()).collect(),
    );

    AcpRuntimeCatalogEntry {
        id: def.id.to_string(),
        label: def.label.to_string(),
        // No remote URL — all preset icons are bundled assets.
        avatar_url: String::new(),
        availability,
        command,
        binary_path,
        default_args,
        mcp_command: None,
        model_env_var: None,
        provider_env_var: None,
        thinking_env_var: None,
        install_hint: def.install_hint.to_string(),
        install_instructions_url: def.install_instructions_url.to_string(),
        can_auto_install: false,
        // Kept false even for adapter presets: presets carry one flat
        // install_hint (the adapter's), so the requiresExternalCli
        // "CLI is missing" wording would pair the wrong noun with it.
        // The builtin path, with per-availability hints, is the only
        // consumer of the true case.
        requires_external_cli: false,
        underlying_cli_path,
        node_required: false,
        auth_status: AuthStatus::NotApplicable,
        login_hint: None,
        source: HarnessSource::Preset,
        // Preset entries have static, non-editable env; definition_env is empty.
        definition_env: Default::default(),
    }
}

pub(crate) const PRESET_HARNESSES: &[PresetHarness] = &[
    PresetHarness {
        id: "cursor",
        label: "Cursor",
        command: "cursor-agent",
        args: &["acp"],
        install_instructions_url: "https://cursor.com/downloads",
        install_hint: "Buzz talks to Cursor through the cursor-agent CLI's ACP mode.",
        underlying_cli: None,
    },
    PresetHarness {
        id: "omp",
        label: "Oh My Pi",
        command: "omp",
        args: &["acp"],
        install_instructions_url: "https://github.com/can1357/oh-my-pi",
        install_hint: "Buzz talks to Oh My Pi through its CLI's ACP mode (omp acp).",
        underlying_cli: None,
    },
    PresetHarness {
        id: "grok",
        label: "Grok Build",
        command: "grok",
        args: &["agent", "--always-approve", "stdio"],
        install_instructions_url: "https://build.x.ai/docs",
        install_hint: "Buzz talks to Grok Build through its CLI's agent stdio mode.",
        underlying_cli: None,
    },
    PresetHarness {
        id: "opencode",
        label: "OpenCode",
        command: "opencode",
        args: &["acp"],
        install_instructions_url: "https://opencode.ai/docs",
        install_hint: "Buzz talks to OpenCode through its CLI's ACP mode (opencode acp).",
        underlying_cli: None,
    },
    PresetHarness {
        id: "kimi",
        label: "Kimi Code",
        command: "kimi",
        args: &["acp"],
        install_instructions_url: "https://kimi.ai/download",
        install_hint: "Buzz talks to Kimi Code through its CLI's ACP mode (kimi acp).",
        underlying_cli: None,
    },
    PresetHarness {
        id: "amp",
        label: "Amp",
        command: "amp-acp",
        args: &[],
        install_instructions_url: "https://github.com/tao12345666333/amp-acp",
        install_hint: "Buzz talks to the Amp CLI through the amp-acp adapter. Follow the setup guide to install the adapter so the amp-acp command is on your PATH.",
        underlying_cli: Some("amp"),
    },
    PresetHarness {
        id: "hermes",
        label: "Hermes Agent",
        command: "hermes-acp",
        args: &[],
        install_instructions_url: "https://hermes-agent.nousresearch.com",
        install_hint: "Buzz talks to Hermes Agent through its hermes-acp command.",
        underlying_cli: None,
    },
    PresetHarness {
        id: "openclaw",
        label: "OpenClaw",
        command: "openclaw",
        args: &["acp"],
        install_instructions_url: "https://docs.openclaw.ai/start/getting-started",
        install_hint: "Buzz talks to OpenClaw through its ACP mode (openclaw acp), which relies on the OpenClaw Gateway daemon. Follow the setup guide to install both.\n\n\
            ⚠️  Execution-locus note: `openclaw acp` runs tools inside the \
            OpenClaw Gateway daemon, not in the Desktop process. \
            Desktop-injected BUZZ_* env vars are visible to the `openclaw` \
            harness process itself, but do NOT automatically reach the \
            Gateway's execution environment. If your tools or agent logic \
            needs BUZZ_* credentials at execution time, set them on the \
            Gateway's own environment separately.",
        underlying_cli: None,
    },
    PresetHarness {
        id: "cline",
        label: "Cline",
        command: "cline",
        args: &["--acp"],
        install_instructions_url: "https://docs.cline.bot/cli/overview",
        install_hint: "Buzz talks to Cline through its CLI's ACP mode (cline --acp).",
        underlying_cli: None,
    },
];

/// Return the static preset harness definitions as `HarnessDefinition` values.
///
/// Used by `warm_harness_registry_from_dir` to seed the loaded-harness registry
/// at startup before the frontend triggers a full discovery run.
pub(crate) fn preset_harness_definitions(
) -> Vec<crate::managed_agents::custom_harnesses::HarnessDefinition> {
    PRESET_HARNESSES
        .iter()
        .map(
            |p| crate::managed_agents::custom_harnesses::HarnessDefinition {
                id: p.id.to_string(),
                label: p.label.to_string(),
                command: p.command.to_string(),
                args: p.args.iter().map(|s| s.to_string()).collect(),
                env: std::collections::BTreeMap::new(),
                install_instructions_url: p.install_instructions_url.to_string(),
                install_hint: p.install_hint.to_string(),
            },
        )
        .collect()
}

/// Return the static slice of preset harness IDs.
///
/// Used by `check_id_collision` in `custom_harnesses` to derive the reserved-ID
/// set from the single source of truth (`PRESET_HARNESSES`) rather than a
/// hand-maintained copy.  Adding a preset automatically reserves its ID.
pub(crate) fn preset_harness_ids() -> &'static [&'static str] {
    // `PRESET_HARNESSES` is `'static`; we project its `id` fields.
    // Computed once via OnceLock to avoid repeated allocations on hot paths.
    use std::sync::OnceLock;
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| PRESET_HARNESSES.iter().map(|p| p.id).collect())
        .as_slice()
}
