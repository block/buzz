/// Normalize a configured ACP command to the executable identity used by
/// runtime presets. Paths, Windows suffixes, case, spaces, and underscores do
/// not change the identity.
pub(super) fn normalize_command_identity(command: &str) -> String {
    let normalized = command.trim().replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    let lower = basename
        .chars()
        .map(|character| match character {
            ' ' | '_' => '-',
            _ => character.to_ascii_lowercase(),
        })
        .collect::<String>();
    let lower = lower.strip_suffix(".exe").unwrap_or(&lower).to_string();

    if let Some(suffix) = std::env::consts::EXE_SUFFIX.strip_prefix('.') {
        return lower
            .strip_suffix(&format!(".{suffix}"))
            .unwrap_or(&lower)
            .to_string();
    }
    if !std::env::consts::EXE_SUFFIX.is_empty() {
        return lower
            .strip_suffix(std::env::consts::EXE_SUFFIX)
            .unwrap_or(&lower)
            .to_string();
    }
    lower
}

/// Whether this adapter contract requires Buzz to select durable per-thread
/// queue identity before lazy startup accepts its first event.
pub(crate) fn requires_durable_thread_sessions(command: &str) -> bool {
    normalize_command_identity(command) == "buzz-pi-agent"
}

#[cfg(test)]
mod tests {
    use super::requires_durable_thread_sessions;

    #[test]
    fn pi_requires_durable_thread_sessions_for_bare_and_resolved_commands() {
        assert!(requires_durable_thread_sessions("buzz-pi-agent"));
        assert!(requires_durable_thread_sessions(
            "/Users/example/.local/bin/buzz-pi-agent"
        ));
        assert!(requires_durable_thread_sessions("BUZZ_PI_AGENT.EXE"));
        assert!(!requires_durable_thread_sessions("pi"));
        assert!(!requires_durable_thread_sessions("goose"));
    }
}
