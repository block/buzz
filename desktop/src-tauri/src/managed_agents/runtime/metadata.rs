use std::collections::BTreeMap;

const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
const OPENAI_COMPAT_API_KEY: &str = "OPENAI_COMPAT_API_KEY";

fn configured_openai_api_key(env: &BTreeMap<String, String>) -> Option<&str> {
    if env.contains_key(OPENAI_API_KEY) {
        return None;
    }
    env.get(OPENAI_COMPAT_API_KEY)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
}

/// Ensure every harness receives an explicitly configured OpenAI credential
/// under OpenAI's standard environment variable name.
///
/// Buzz's built-in agent still reads `OPENAI_COMPAT_API_KEY`, so Desktop keeps
/// that configured value and mirrors it to `OPENAI_API_KEY` at the common spawn
/// boundary. An explicit `OPENAI_API_KEY` in the layered agent environment is
/// never replaced, including when it was deliberately set to an empty value.
pub(crate) fn apply_openai_api_key(
    command: &mut std::process::Command,
    env: &BTreeMap<String, String>,
) {
    if let Some(value) = configured_openai_api_key(env) {
        command.env(OPENAI_API_KEY, value);
    }
}

/// Returns the (key, value) env var pairs that should be forwarded to the
/// agent process for model and provider selection.
///
/// Model injection is unconditional — even agents that support ACP model
/// switching need the initial bootstrap value. Provider injection is skipped
/// when `provider_locked` is true (e.g. Claude runtimes that only work with
/// Anthropic).
pub(crate) fn runtime_metadata_env_vars<'a>(
    model_env_var: Option<&'a str>,
    provider_env_var: Option<&'a str>,
    provider_locked: bool,
    effective_model: Option<&'a str>,
    effective_provider: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut vars = Vec::new();
    if let (Some(env_key), Some(model)) = (model_env_var, effective_model) {
        vars.push((env_key, model));
    }
    if !provider_locked {
        if let (Some(env_key), Some(provider)) = (provider_env_var, effective_provider) {
            vars.push((env_key, provider));
        }
    }
    vars
}

/// Env var carrying the session title to the harness. Shared with
/// `spawn_snapshot` so the restart badge records the same key the spawn writes.
pub(crate) const SESSION_TITLE_ENV_VAR: &str = "BUZZ_ACP_SESSION_TITLE";
/// Stable agent display name forwarded to the ACP tool surface for git
/// attribution and private-conversation provenance.
pub(crate) const DISPLAY_NAME_ENV_VAR: &str = "BUZZ_ACP_DISPLAY_NAME";

/// Apply the shared stable agent name to both session display metadata and
/// git attribution, clearing both keys when no usable name is available.
pub(crate) fn apply_agent_display_env(command: &mut std::process::Command, title: Option<String>) {
    if let Some(title) = title {
        command
            .env(SESSION_TITLE_ENV_VAR, &title)
            .env(DISPLAY_NAME_ENV_VAR, title);
    } else {
        command
            .env_remove(SESSION_TITLE_ENV_VAR)
            .env_remove(DISPLAY_NAME_ENV_VAR);
    }
}

/// Resolve the session title for an agent: its `display_name` when it has one,
/// otherwise its unique `name` handle. `None` when both are blank, so the
/// caller clears the env var rather than exporting an empty title.
///
/// Control characters are stripped **here**, not left to the harness: an
/// interior NUL cannot cross the environment boundary at all, so
/// `Command::env` fails the whole spawn rather than passing it through (see
/// the same guard applied to user-supplied env in `env_vars::merged_user_env`).
/// A display name that is nothing but control characters therefore falls back
/// to `name` instead of turning display chrome into a spawn failure.
///
/// The harness still owns whitespace collapsing, the length cap, and channel
/// qualification — see `sanitize_session_title` and `compose_session_title` in
/// `buzz-acp`.
pub(crate) fn resolve_session_title(display_name: Option<&str>, name: &str) -> Option<String> {
    [display_name, Some(name)]
        .into_iter()
        .flatten()
        .map(|value| {
            value
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{configured_openai_api_key, resolve_session_title};

    #[test]
    fn configured_compat_key_is_exposed_under_openai_standard_name() {
        let env = BTreeMap::from([(
            "OPENAI_COMPAT_API_KEY".to_string(),
            "configured-key".to_string(),
        )]);
        assert_eq!(configured_openai_api_key(&env), Some("configured-key"));
    }

    #[test]
    fn explicit_standard_openai_key_is_preserved() {
        let env = BTreeMap::from([
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                "configured-key".to_string(),
            ),
            ("OPENAI_API_KEY".to_string(), "explicit-key".to_string()),
        ]);
        assert_eq!(configured_openai_api_key(&env), None);
    }

    #[test]
    fn blank_compat_key_is_not_exported() {
        let env = BTreeMap::from([("OPENAI_COMPAT_API_KEY".to_string(), "  ".to_string())]);
        assert_eq!(configured_openai_api_key(&env), None);
    }

    #[test]
    fn resolve_session_title_prefers_display_name() {
        assert_eq!(
            resolve_session_title(Some("Fizz"), "fizz-1").as_deref(),
            Some("Fizz")
        );
    }

    #[test]
    fn resolve_session_title_falls_back_to_name_when_display_name_blank() {
        assert_eq!(
            resolve_session_title(None, "fizz-1").as_deref(),
            Some("fizz-1")
        );
        assert_eq!(
            resolve_session_title(Some("  "), "fizz-1").as_deref(),
            Some("fizz-1")
        );
    }

    #[test]
    fn resolve_session_title_returns_none_when_both_are_blank() {
        assert_eq!(resolve_session_title(Some(""), "   "), None);
    }

    #[test]
    fn resolve_session_title_trims_surrounding_whitespace() {
        assert_eq!(
            resolve_session_title(Some("  Fizz  "), "fizz-1").as_deref(),
            Some("Fizz")
        );
    }

    /// An interior NUL cannot cross the env boundary — `Command::env` returns
    /// `Err` for the whole spawn. Stripping it here keeps a corrupted record
    /// from turning display chrome into a spawn failure.
    #[test]
    fn resolve_session_title_strips_control_chars_that_would_fail_the_spawn() {
        let title = resolve_session_title(Some("Fi\u{0}zz\u{7}"), "fizz-1")
            .expect("a name with strippable controls still yields a title");
        assert_eq!(title, "Fizz");
        assert!(!title.contains('\u{0}'));
    }

    /// A display name that is *only* control characters is not a title, so the
    /// unique handle takes over rather than exporting an empty value.
    #[test]
    fn resolve_session_title_falls_back_to_name_when_display_name_is_all_control_chars() {
        assert_eq!(
            resolve_session_title(Some("\u{0}\u{1}"), "fizz-1").as_deref(),
            Some("fizz-1")
        );
    }

    /// Both candidates unusable — the caller clears the env var instead of
    /// exporting a NUL-bearing or empty title.
    #[test]
    fn resolve_session_title_returns_none_when_both_candidates_are_control_chars_only() {
        assert_eq!(resolve_session_title(Some("\u{0}"), "\u{0}"), None);
    }
}
