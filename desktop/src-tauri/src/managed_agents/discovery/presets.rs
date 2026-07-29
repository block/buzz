use std::path::PathBuf;

use crate::managed_agents::{
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus, HarnessSource,
};

use super::normalize_agent_args;

pub(super) struct PresetHarness {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) command: &'static str,
    pub(super) args: &'static [&'static str],
    pub(super) install_instructions_url: &'static str,
    pub(super) install_hint: &'static str,
    pub(super) underlying_cli: Option<&'static str>,
}

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
    let default_args = normalize_agent_args(
        def.command,
        def.args.iter().map(|arg| arg.to_string()).collect(),
    );

    AcpRuntimeCatalogEntry {
        id: def.id.to_string(),
        label: def.label.to_string(),
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
        id: "cybara",
        label: "Cybara",
        command: "cybara",
        args: &["acp"],
        install_instructions_url: "https://cybara.ai/download#cli-tui",
        install_hint: "Buzz talks to Cybara through its CLI's ACP mode (cybara acp).",
        underlying_cli: None,
    },
];

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
                env: std::collections::BTreeMap::new(),
                install_instructions_url: preset.install_instructions_url.to_string(),
                install_hint: preset.install_hint.to_string(),
            },
        )
        .collect()
}

pub(crate) fn preset_harness_ids() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static IDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| PRESET_HARNESSES.iter().map(|preset| preset.id).collect())
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::preset_harness_definitions;

    #[test]
    fn cybara_launches_native_acp_mode() {
        let cybara = preset_harness_definitions()
            .into_iter()
            .find(|preset| preset.id == "cybara")
            .expect("Cybara preset should exist");

        assert_eq!(cybara.label, "Cybara");
        assert_eq!(cybara.command, "cybara");
        assert_eq!(cybara.args, vec!["acp"]);
        assert_eq!(
            cybara.install_instructions_url,
            "https://cybara.ai/download#cli-tui"
        );
        assert!(cybara.install_hint.contains("cybara acp"));
        assert!(cybara.env.is_empty());
    }
}
