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
    }
}

pub(super) const PRESET_HARNESSES: &[PresetHarness] = &[
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

        let entry = super::super::discover_acp_runtimes_from(None)
            .into_iter()
            .find(|entry| entry.id == "devin")
            .expect("Devin preset should appear in the runtime catalog");

        assert_eq!(entry.label, "Devin");
        assert_eq!(entry.default_args, vec!["acp"]);
        assert_eq!(entry.install_instructions_url, "https://docs.devin.ai/cli");
        assert_eq!(entry.source, HarnessSource::Preset);
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
}
