use super::normalize_command_identity;

fn default_agent_args(command: &str) -> Option<Vec<String>> {
    match normalize_command_identity(command).as_str() {
        "goose" => Some(vec!["acp".to_string()]),
        "qoder" | "qoder-cli" | "qodercli" => Some(vec!["--acp".to_string()]),
        "codex" | "codex-acp" | "claude-agent-acp" | "claude-code-acp" | "claude-code"
        | "claudecode" | "buzz-agent" => Some(Vec::new()),
        _ => None,
    }
}

pub fn normalize_agent_args(command: &str, agent_args: Vec<String>) -> Vec<String> {
    let normalized = agent_args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();

    let Some(default_args) = default_agent_args(command) else {
        return normalized;
    };

    if normalized.is_empty() {
        return default_args;
    }

    if normalized.len() == 1 && normalized[0].eq_ignore_ascii_case("acp") {
        return default_args;
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::normalize_agent_args;

    #[test]
    fn normalizes_buzz_agent_args_to_empty() {
        assert_eq!(
            normalize_agent_args("buzz-agent", Vec::new()),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args("buzz-agent", vec!["acp".into()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn normalizes_claude_and_codex_args_to_empty() {
        for command in ["claude-agent-acp", "claude-code-acp", "codex-acp"] {
            assert_eq!(
                normalize_agent_args(command, vec!["acp".into()]),
                Vec::<String>::new()
            );
        }
    }

    #[test]
    fn normalizes_qoder_args_to_acp_flag() {
        assert_eq!(normalize_agent_args("qodercli", Vec::new()), vec!["--acp"]);
        assert_eq!(
            normalize_agent_args("/usr/local/bin/qodercli", vec!["acp".into()]),
            vec!["--acp"]
        );
        assert_eq!(
            normalize_agent_args("Qoder CLI", vec!["--acp".into()]),
            vec!["--acp"]
        );
    }
}
