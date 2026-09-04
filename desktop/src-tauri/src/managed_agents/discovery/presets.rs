use std::path::PathBuf;
use std::sync::OnceLock;

use crate::managed_agents::{
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus, HarnessSource,
};

use super::normalize_agent_args;

/// Static data for a well-known tier-2 ACP harness.
pub(super) struct PresetHarness {
    pub(super) id: &'static str,
    label: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    install_instructions_url: &'static str,
    install_hint: &'static str,
    /// Vendor CLI the ACP command wraps, when the preset is an adapter.
    ///
    /// Consulted only when the adapter is absent, so `AdapterMissing` replaces
    /// `NotInstalled` when the CLI is present but the adapter is not. `None`
    /// when the command is itself the vendor CLI.
    underlying_cli: Option<&'static str>,
}

/// Build one preset catalog entry through an injectable command resolver.
pub(super) fn preset_catalog_entry(
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
        .map(|path| path.display().to_string());

    AcpRuntimeCatalogEntry {
        id: def.id.to_string(),
        label: def.label.to_string(),
        // No remote URL — all preset icons are bundled assets.
        avatar_url: String::new(),
        availability,
        command,
        binary_path,
        default_args: normalize_agent_args(
            def.command,
            def.args.iter().map(|arg| arg.to_string()).collect(),
        ),
        mcp_command: None,
        model_env_var: None,
        provider_env_var: None,
        thinking_env_var: None,
        effort_canonical_values: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        max_rounds_env_var: None,
        install_hint: def.install_hint.to_string(),
        install_instructions_url: def.install_instructions_url.to_string(),
        can_auto_install: false,
        // Presets carry one flat install hint, so builtin external-CLI copy
        // would name the wrong missing component for adapter presets.
        requires_external_cli: false,
        underlying_cli_path,
        node_required: false,
        auth_status: AuthStatus::NotApplicable,
        login_hint: None,
        source: HarnessSource::Preset,
        definition_env: Default::default(),
        // Derived from the static preset command (`def.command`). This ensures
        // unavailable entries (command: null in JSON, None here) still carry
        // the cap — the harness cap is command-keyed, not availability-gated.
        max_parallelism: crate::managed_agents::harness_max_parallelism(def.command),
    }
}

pub(super) const PRESET_HARNESSES: &[PresetHarness] = &[
    PresetHarness {
        id: "pi",
        label: "Pi",
        command: "pi-acp",
        args: &[],
        install_instructions_url: "https://github.com/svkozak/pi-acp",
        install_hint: "Buzz talks to Pi through the pi-acp adapter. Install Pi with `npm install -g --ignore-scripts @earendil-works/pi-coding-agent`, then install the adapter with `npm install -g pi-acp`.",
        underlying_cli: Some("pi"),
    },
    PresetHarness {
        id: "devin",
        label: "Devin",
        command: "devin",
        args: &["acp"],
        install_instructions_url: "https://docs.devin.ai/cli",
        install_hint: "Buzz talks to Devin through the official Devin CLI's ACP mode (devin acp).",
        underlying_cli: None,
    },
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
        install_instructions_url: "https://omp.sh/",
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
];

/// Return preset definitions for the spawn/readiness registry.
pub(crate) fn preset_harness_definitions(
) -> Vec<crate::managed_agents::custom_harnesses::HarnessDefinition> {
    PRESET_HARNESSES
        .iter()
        .map(
            |preset| crate::managed_agents::custom_harnesses::HarnessDefinition {
                id: preset.id.to_string(),
                label: preset.label.to_string(),
                command: preset.command.to_string(),
                args: preset.args.iter().map(|arg| arg.to_string()).collect(),
                env: Default::default(),
                install_instructions_url: preset.install_instructions_url.to_string(),
                install_hint: preset.install_hint.to_string(),
            },
        )
        .collect()
}

/// Return preset IDs from the catalog's single source of truth.
pub(crate) fn preset_harness_ids() -> &'static [&'static str] {
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| PRESET_HARNESSES.iter().map(|preset| preset.id).collect())
        .as_slice()
}

/// Return the primary command for a preset harness by id, or `None` if the id
/// is not a known preset.
///
/// Returns a `&'static str` so callers can use it without allocation.
pub(super) fn preset_command_for_id(id: &str) -> Option<&'static str> {
    PRESET_HARNESSES
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.command)
}

/// Return the primary harness command for a given runtime id, or `None`.
///
/// Checks static builtins, then the static preset list (always available,
/// no registry warm-up required — covers openclaw, devin, cursor, etc.),
/// then the loaded preset/custom registry.
pub(crate) fn command_for_runtime_id(id: &str) -> Option<String> {
    super::known_acp_runtime_exact(id)
        .and_then(|r| r.commands.first().copied())
        .map(str::to_string)
        .or_else(|| preset_command_for_id(id).map(str::to_string))
        .or_else(|| {
            crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id(id)
                .map(|d| d.command.clone())
        })
}

/// Resolve a harness to its canonical command accepting either a runtime id or
/// a command string (including path prefixes and aliases).
///
/// This is the pin-classification resolver for `apply_persona_snapshot`: the
/// create-time override in `record.agent_command_override` can hold any of the
/// forms a user or the harness selector might have stored — bare command
/// ("goose"), alias ("claude-code-acp"), path ("/usr/local/bin/goose"), or the
/// runtime id directly ("claude"). All three tiers are searched:
///
/// 1. **Builtins** — `known_acp_runtime(input)` matches by id, command, or
///    alias in `KNOWN_ACP_RUNTIMES`; returns its first primary command.
/// 2. **Static presets** — searched by id or by normalised command.
/// 3. **Loaded registry** — searched by id or by normalised command.
///
/// Returns `None` for inputs that do not resolve to any known harness; those
/// pins are treated as custom/unknown and always kept.
pub(crate) fn canonical_harness_command(input: &str) -> Option<String> {
    let normalized = super::normalize_command_identity(input);

    // Tier 1: builtins — matched by id, command, or alias.
    if let Some(rt) = super::known_acp_runtime(&normalized) {
        if let Some(cmd) = rt.commands.first() {
            return Some(cmd.to_string());
        }
    }

    // Tier 2: static presets — matched by id or by normalized command.
    if let Some(p) = PRESET_HARNESSES
        .iter()
        .find(|p| p.id == normalized || super::normalize_command_identity(p.command) == normalized)
    {
        return Some(p.command.to_string());
    }

    // Tier 3: loaded registry — matched by id or by normalized command.
    let reg = crate::managed_agents::custom_harnesses::loaded_harness_registry()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    reg.iter()
        .find(|d| d.id == normalized || super::normalize_command_identity(&d.command) == normalized)
        .map(|d| d.command.clone())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::managed_agents::{AcpAvailabilityStatus, AuthStatus, HarnessSource};

    use super::{preset_catalog_entry, PresetHarness, PRESET_HARNESSES};

    /// Amp-shaped preset: an ACP adapter wrapping a separately installed CLI.
    const ADAPTER_PRESET: PresetHarness = PresetHarness {
        id: "amp-test",
        label: "Amp Test",
        command: "amp-acp",
        args: &[],
        install_instructions_url: "https://example.com/install",
        install_hint: "Install the amp-acp npm adapter.",
        underlying_cli: Some("amp"),
    };

    #[test]
    fn devin_preset_uses_official_native_acp_invocation() {
        let preset = PRESET_HARNESSES
            .iter()
            .find(|preset| preset.id == "devin")
            .expect("Devin preset should be present");

        assert_eq!(preset.label, "Devin");
        assert_eq!(preset.command, "devin");
        assert_eq!(preset.args, &["acp"]);
        assert_eq!(preset.underlying_cli, None);
        assert_eq!(preset.install_instructions_url, "https://docs.devin.ai/cli");

        let entry = preset_catalog_entry(preset, |command| {
            (command == "devin").then(|| PathBuf::from("/usr/local/bin/devin"))
        });
        assert_eq!(entry.availability, AcpAvailabilityStatus::Available);
        assert_eq!(entry.command.as_deref(), Some("devin"));
        assert_eq!(entry.default_args, vec!["acp"]);
        assert_eq!(entry.binary_path.as_deref(), Some("/usr/local/bin/devin"));
        assert_eq!(entry.auth_status, AuthStatus::NotApplicable);
        assert_eq!(entry.source, HarnessSource::Preset);

        let missing_entry = preset_catalog_entry(preset, |_| None);
        assert_eq!(
            missing_entry.availability,
            AcpAvailabilityStatus::NotInstalled
        );
        assert!(missing_entry.command.is_none());
        assert_eq!(missing_entry.default_args, vec!["acp"]);
    }

    #[test]
    fn devin_preset_is_exposed_in_the_runtime_catalog() {
        use crate::managed_agents::custom_harnesses::registry_test_lock;

        // Discovery touches process-global command-resolution and the loaded
        // harness registry. Serialize with the other discovery tests.
        let _path_guard = crate::managed_agents::lock_path_mutex();
        let _registry_guard = registry_test_lock();

        let entry = super::super::discover_acp_runtimes_from(None, true)
            .into_iter()
            .find(|entry| entry.id == "devin")
            .expect("Devin preset should appear in the runtime catalog");

        assert_eq!(entry.label, "Devin");
        assert_eq!(entry.default_args, vec!["acp"]);
        assert_eq!(entry.install_instructions_url, "https://docs.devin.ai/cli");
        assert_eq!(entry.source, HarnessSource::Preset);
    }

    #[test]
    fn pi_preset_uses_zero_arg_adapter_and_reports_missing_component() {
        let preset = PRESET_HARNESSES
            .iter()
            .find(|preset| preset.id == "pi")
            .expect("Pi preset should be present");

        assert_eq!(preset.label, "Pi");
        assert_eq!(preset.command, "pi-acp");
        assert!(preset.args.is_empty());
        assert_eq!(preset.underlying_cli, Some("pi"));

        let available = preset_catalog_entry(preset, |command| match command {
            "pi-acp" => Some(PathBuf::from("/usr/local/bin/pi-acp")),
            "pi" => Some(PathBuf::from("/usr/local/bin/pi")),
            _ => None,
        });
        assert_eq!(available.availability, AcpAvailabilityStatus::Available);
        assert_eq!(available.command.as_deref(), Some("pi-acp"));
        assert!(available.default_args.is_empty());
        assert_eq!(
            available.underlying_cli_path.as_deref(),
            Some("/usr/local/bin/pi")
        );

        let adapter_missing = preset_catalog_entry(preset, |command| {
            (command == "pi").then(|| PathBuf::from("/usr/local/bin/pi"))
        });
        assert_eq!(
            adapter_missing.availability,
            AcpAvailabilityStatus::AdapterMissing
        );
        assert!(adapter_missing.command.is_none());
        assert!(adapter_missing.default_args.is_empty());

        let not_installed = preset_catalog_entry(preset, |_| None);
        assert_eq!(
            not_installed.availability,
            AcpAvailabilityStatus::NotInstalled
        );
    }

    #[test]
    fn adapter_missing_when_underlying_cli_present() {
        let entry = preset_catalog_entry(&ADAPTER_PRESET, |command| {
            (command == "amp").then(|| PathBuf::from("/usr/local/bin/amp"))
        });
        assert_eq!(entry.availability, AcpAvailabilityStatus::AdapterMissing);
        assert!(entry.command.is_none());
        assert!(entry.binary_path.is_none());
        assert_eq!(
            entry.underlying_cli_path.as_deref(),
            Some("/usr/local/bin/amp")
        );
        assert!(!entry.requires_external_cli);
        assert_eq!(entry.install_hint, "Install the amp-acp npm adapter.");
    }

    #[test]
    fn not_installed_when_adapter_and_cli_are_missing() {
        let entry = preset_catalog_entry(&ADAPTER_PRESET, |_| None);
        assert_eq!(entry.availability, AcpAvailabilityStatus::NotInstalled);
        assert!(entry.underlying_cli_path.is_none());
        assert!(!entry.requires_external_cli);
    }

    #[test]
    fn available_when_adapter_and_cli_are_present() {
        let entry = preset_catalog_entry(&ADAPTER_PRESET, |command| match command {
            "amp-acp" => Some(PathBuf::from("/usr/local/bin/amp-acp")),
            "amp" => Some(PathBuf::from("/usr/local/bin/amp")),
            _ => None,
        });
        assert_eq!(entry.availability, AcpAvailabilityStatus::Available);
        assert_eq!(entry.command.as_deref(), Some("amp-acp"));
        assert_eq!(entry.binary_path.as_deref(), Some("/usr/local/bin/amp-acp"));
        assert_eq!(
            entry.underlying_cli_path.as_deref(),
            Some("/usr/local/bin/amp")
        );
    }

    #[test]
    fn adapter_presence_is_enough_for_availability() {
        let entry = preset_catalog_entry(&ADAPTER_PRESET, |command| {
            (command == "amp-acp").then(|| PathBuf::from("/usr/local/bin/amp-acp"))
        });
        assert_eq!(entry.availability, AcpAvailabilityStatus::Available);
        assert_eq!(entry.command.as_deref(), Some("amp-acp"));
        assert_eq!(entry.binary_path.as_deref(), Some("/usr/local/bin/amp-acp"));
        assert!(entry.underlying_cli_path.is_none());
    }

    #[test]
    fn preset_without_underlying_cli_stays_simple() {
        let preset = PresetHarness {
            underlying_cli: None,
            ..ADAPTER_PRESET
        };
        let entry = preset_catalog_entry(&preset, |_| None);
        assert_eq!(entry.availability, AcpAvailabilityStatus::NotInstalled);
        assert!(!entry.requires_external_cli);
        assert!(entry.underlying_cli_path.is_none());
    }

    // ── Catalog max_parallelism: command-keyed execution policy ──────────────

    /// Unavailable OpenClaw (command not on PATH → command: null in JSON):
    /// max_parallelism must still be Some(5) — derived from the static `def.command`,
    /// not the probed `entry.command`.
    #[test]
    fn openclaw_preset_unavailable_carries_max_parallelism() {
        let openclaw = PRESET_HARNESSES
            .iter()
            .find(|p| p.id == "openclaw")
            .expect("openclaw preset must be present");

        // Simulate "not installed" — resolver always returns None.
        let entry = preset_catalog_entry(openclaw, |_| None);
        assert_eq!(entry.availability, AcpAvailabilityStatus::NotInstalled);
        assert!(
            entry.command.is_none(),
            "unavailable entry must have command: null"
        );
        assert_eq!(
            entry.max_parallelism,
            Some(crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM),
            "unavailable OpenClaw must still carry max_parallelism {}",
            crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM
        );
    }

    /// Available OpenClaw: max_parallelism present regardless of install status.
    #[test]
    fn openclaw_preset_available_carries_max_parallelism() {
        let openclaw = PRESET_HARNESSES
            .iter()
            .find(|p| p.id == "openclaw")
            .expect("openclaw preset must be present");

        let entry = preset_catalog_entry(openclaw, |cmd| {
            (cmd == openclaw.id || cmd == "openclaw")
                .then(|| std::path::PathBuf::from("/usr/local/bin/openclaw"))
        });
        assert_eq!(
            entry.max_parallelism,
            Some(crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM),
            "available OpenClaw must carry max_parallelism {}",
            crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM
        );
    }

    /// Uncapped preset (devin): max_parallelism must be None.
    #[test]
    fn uncapped_preset_has_no_max_parallelism() {
        let devin = PRESET_HARNESSES
            .iter()
            .find(|p| p.id == "devin")
            .expect("devin preset must be present");
        let entry = preset_catalog_entry(devin, |_| None);
        assert_eq!(
            entry.max_parallelism, None,
            "uncapped preset (devin) must have max_parallelism: None"
        );
    }
}
