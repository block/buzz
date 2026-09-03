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

/// Env var carrying the startup replay floor to the harness. Shared with the
/// provider deploy path (`commands::agents::provider_deploy`) so the local
/// spawn and the remote `launch.policy_env` injection name the key from one
/// place.
pub(crate) const REPLAY_FLOOR_ENV_VAR: &str = "BUZZ_ACP_REPLAY_FLOOR";

/// Apply the publish-first replay floor: inject [`REPLAY_FLOOR_ENV_VAR`] from
/// `replay_floor_unix` (or leave the key untouched if `None`).
///
/// Must be called **after** `descriptor.env` is written so this send's floor
/// wins over any user-supplied `BUZZ_ACP_REPLAY_FLOOR` entry — the same
/// authority ordering [`super::apply_effort_env`] asserts for effort, and the
/// same shadow strip `apply_replay_floor` performs on the provider payload's
/// `launch.env` tier. Without it a persona/global/agent env entry would
/// override the floor and the harness's startup watermark would be computed
/// from a stale (or `now`-clamped future) value, missing the mention that
/// triggered the spawn.
///
/// When `replay_floor_unix` is `None` there is no floor to assert; the key is
/// left as `descriptor.env` wrote it, matching the provider path where a
/// user-supplied `launch.env` value passes through on a floorless deploy. The
/// caller strips the ambient parent-process value before the `descriptor.env`
/// loop, so `None` never inherits a floor from the environment Desktop itself
/// was launched with.
pub(crate) fn apply_replay_floor_env(
    command: &mut std::process::Command,
    replay_floor_unix: Option<u64>,
) {
    if let Some(floor) = replay_floor_unix {
        command.env(REPLAY_FLOOR_ENV_VAR, floor.to_string());
    }
    // None: no floor to assert — leave whatever descriptor.env wrote intact.
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

/// Level names that, standing alone, set `EnvFilter`'s global default rather
/// than a target. The parser accepts the numeric forms too.
const GLOBAL_LEVELS: [&str; 12] = [
    "off", "error", "warn", "info", "debug", "trace", "0", "1", "2", "3", "4", "5",
];

/// Whether any directive in `filter` is a bare level, i.e. the operator set a
/// global default instead of naming targets.
fn sets_global_level(filter: &str) -> bool {
    filter.split(',').any(|directive| {
        let directive = directive.trim();
        GLOBAL_LEVELS
            .iter()
            .any(|level| directive.eq_ignore_ascii_case(level))
    })
}

/// Build the `RUST_LOG` value forwarded to the agent child: keep an existing
/// filter that already mentions `buzz_acp` or sets a global level, append
/// `buzz_acp=info` to any other non-empty filter, and default to
/// `buzz_acp=info` when unset.
pub(crate) fn child_rust_log_filter() -> String {
    child_rust_log_filter_from(std::env::var("RUST_LOG").ok())
}

/// Taking the ambient value as an argument keeps the policy testable without a
/// test mutating process environment.
fn child_rust_log_filter_from(existing: Option<String>) -> String {
    match existing {
        Some(existing) if existing.contains("buzz_acp") => existing,
        // A bare level is the global default, and a target directive outranks
        // it. Appending ours would override the operator in both directions:
        // `off` would still log this crate at info, and `trace` would be
        // narrowed to info for the one crate the operator was debugging.
        Some(existing) if sets_global_level(&existing) => existing,
        Some(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_replay_floor_env, child_rust_log_filter_from, resolve_session_title,
        REPLAY_FLOOR_ENV_VAR,
    };

    fn replay_floor_of(cmd: &std::process::Command) -> Option<String> {
        cmd.get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new(REPLAY_FLOOR_ENV_VAR))
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    }

    /// The publish-first floor must win over a persona/global/agent env entry
    /// written by the `descriptor.env` loop. Before the post-loop application
    /// the saved value shadowed the floor and the harness booted blind to the
    /// mention that triggered the spawn.
    #[test]
    fn caller_replay_floor_wins_over_user_env_collision() {
        let mut cmd = std::process::Command::new("true");
        // Simulate the descriptor.env loop writing a saved user value.
        cmd.env(REPLAY_FLOOR_ENV_VAR, "1");

        apply_replay_floor_env(&mut cmd, Some(1_756_600_000));

        assert_eq!(
            replay_floor_of(&cmd).as_deref(),
            Some("1756600000"),
            "this send's floor must win over the user-supplied value"
        );
    }

    /// No caller floor: the user value passes through, matching the provider
    /// payload path where a floorless deploy leaves `launch.env` untouched.
    #[test]
    fn user_replay_floor_env_survives_when_no_caller_floor() {
        let mut cmd = std::process::Command::new("true");
        cmd.env(REPLAY_FLOOR_ENV_VAR, "1756600000");

        apply_replay_floor_env(&mut cmd, None);

        assert_eq!(
            replay_floor_of(&cmd).as_deref(),
            Some("1756600000"),
            "a user-supplied floor must survive when the caller supplies none"
        );
    }

    /// The ambient strip the spawn does before the `descriptor.env` loop must
    /// stay stripped when neither the caller nor user env supplies a floor.
    #[test]
    fn removed_replay_floor_stays_removed_without_caller_floor() {
        let mut cmd = std::process::Command::new("true");
        // Simulate the spawn's pre-loop ambient strip with no user env entry.
        cmd.env_remove(REPLAY_FLOOR_ENV_VAR);

        apply_replay_floor_env(&mut cmd, None);

        assert_eq!(
            replay_floor_of(&cmd),
            None,
            "a floorless spawn must not inherit an ambient parent-process floor"
        );
    }

    /// A caller floor re-asserts the key even after the pre-loop ambient strip
    /// removed it — the common publish-first send with no saved user entry.
    #[test]
    fn caller_replay_floor_injected_after_ambient_strip() {
        let mut cmd = std::process::Command::new("true");
        cmd.env_remove(REPLAY_FLOOR_ENV_VAR);

        apply_replay_floor_env(&mut cmd, Some(42));

        assert_eq!(replay_floor_of(&cmd).as_deref(), Some("42"));
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

    #[test]
    fn bare_global_level_is_forwarded_untouched() {
        // `off` must stay silence and `trace` must stay trace: appending a
        // target directive would outrank the level the operator chose.
        for value in ["off", "TRACE", " debug ", "0", "5"] {
            assert_eq!(
                child_rust_log_filter_from(Some(value.to_string())),
                value,
                "global level must reach the child unchanged"
            );
        }
    }

    #[test]
    fn global_level_alongside_targets_is_still_respected() {
        let filter = child_rust_log_filter_from(Some("debug,hyper=warn".to_string()));
        assert_eq!(filter, "debug,hyper=warn");
    }

    #[test]
    fn target_only_filter_still_gains_the_harness_default() {
        let filter = child_rust_log_filter_from(Some("hyper=warn".to_string()));
        assert_eq!(filter, "hyper=warn,buzz_acp=info");
    }

    #[test]
    fn explicit_buzz_acp_filter_and_unset_are_unchanged() {
        assert_eq!(
            child_rust_log_filter_from(Some("buzz_acp=debug".to_string())),
            "buzz_acp=debug"
        );
        assert_eq!(child_rust_log_filter_from(None), "buzz_acp=info");
        assert_eq!(
            child_rust_log_filter_from(Some("  ".to_string())),
            "buzz_acp=info"
        );
    }
}
