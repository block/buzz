use std::path::PathBuf;

use super::{normalize_command_identity, resolve_command};

pub(super) fn available_harness_command(id: &str, preferred: &str) -> String {
    resolve_preset_command(id, preferred, &resolve_command)
        .map(|(command, _)| command.to_string())
        .unwrap_or_else(|| preferred.to_string())
}

pub(super) fn resolve_preset_command<'a>(
    id: &str,
    preferred: &'a str,
    resolve: &impl Fn(&str) -> Option<PathBuf>,
) -> Option<(&'a str, PathBuf)> {
    resolve(preferred)
        .map(|path| (preferred, path))
        .or_else(|| {
            (id == "hermes")
                .then(|| resolve("hermes").map(|path| ("hermes", path)))
                .flatten()
        })
}

pub(super) fn default_agent_args(command: &str) -> Option<Vec<String>> {
    match normalize_command_identity(command).as_str() {
        "goose" | "hermes" => Some(vec!["acp".to_string()]),
        "hermes-acp" | "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp"
        | "claude-code" | "claudecode" | "buzz-agent" => Some(Vec::new()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{default_agent_args, resolve_preset_command};
    use std::path::PathBuf;

    #[test]
    fn hermes_prefers_acp_shim_and_falls_back_to_cli_subcommand() {
        let preferred = resolve_preset_command("hermes", "hermes-acp", &|command| {
            matches!(command, "hermes-acp" | "hermes").then(|| PathBuf::from(command))
        });
        assert_eq!(preferred.unwrap().0, "hermes-acp");

        let fallback = resolve_preset_command("hermes", "hermes-acp", &|command| {
            (command == "hermes").then(|| PathBuf::from(command))
        });
        assert_eq!(fallback.unwrap().0, "hermes");
        assert_eq!(default_agent_args("hermes").unwrap(), ["acp"]);
    }

    #[test]
    fn fallback_is_not_applied_to_other_harnesses() {
        let resolved = resolve_preset_command("goose", "goose", &|command| {
            (command == "hermes").then(|| PathBuf::from(command))
        });
        assert!(resolved.is_none());
    }
}
