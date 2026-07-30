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
/// `spawn_hash` so the restart badge hashes the same key the spawn writes.
pub(crate) const SESSION_TITLE_ENV_VAR: &str = "BUZZ_ACP_SESSION_TITLE";

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

/// Resolve effective prompt/model/provider using definition-authoritative
/// semantics for linked instances.
///
/// Used by `agent_config.rs` to inject persona defaults into the config surface
/// before running the reader.
pub(crate) fn resolve_effective_prompt_model_provider(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    record_prompt: Option<String>,
    record_model: Option<String>,
    record_provider: Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    match persona_id.and_then(|pid| personas.iter().find(|p| p.id == pid)) {
        Some(p) => {
            fn non_blank(v: Option<&str>) -> Option<String> {
                v.filter(|s| !s.trim().is_empty()).map(str::to_owned)
            }
            let prompt = non_blank(Some(&p.system_prompt));
            let model = non_blank(p.model.as_deref());
            let provider = non_blank(p.provider.as_deref());
            (prompt, model, provider)
        }
        None => (record_prompt, record_model, record_provider),
    }
}

/// The control-plane status string for a backend Buzz does not run in-process.
///
/// Returns `None` for [`BackendKind::Local`], whose status is derived from live
/// process state by the caller. Split out of `build_managed_agent_summary` so the
/// per-backend mapping is testable without an `AppHandle`.
///
/// [`BackendKind::Local`]: crate::managed_agents::BackendKind::Local
pub(crate) fn remote_backend_status(
    backend: &crate::managed_agents::BackendKind,
    backend_agent_id: Option<&str>,
) -> Option<&'static str> {
    use crate::managed_agents::BackendKind;
    match backend {
        BackendKind::Local => None,
        // External agents have no control-plane axis at all: Buzz never deploys
        // them, so `backend_agent_id` is always None and the provider-style
        // "deployed"/"not_deployed" pair would read "not_deployed" forever. The
        // only real signal is the live axis — relay presence (kind:20001),
        // polled by the frontend and shown as a PresenceDot.
        BackendKind::External => Some("external"),
        // Two-axis status model for provider-deployed agents:
        //
        //   Control-plane (this field): "deployed" = provider has been invoked and
        //   returned a backend_agent_id. "not_deployed" = no deploy call yet (or it
        //   failed). This axis tracks whether infrastructure *exists*, not whether
        //   the process is currently running.
        //
        //   Live axis (relay presence, polled by frontend): online/away/offline.
        //   Shown as a PresenceDot next to the agent name. This is the real-time
        //   signal for whether the harness is connected.
        //
        // After !shutdown the agent goes offline (presence) but stays "deployed"
        // (infrastructure still exists). This is intentional — the provider may
        // have allocated a VM/container that persists across process restarts.
        // A future provider `undeploy` operation (v2) will handle teardown.
        BackendKind::Provider { .. } => Some(if backend_agent_id.is_some() {
            "deployed"
        } else {
            "not_deployed"
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{remote_backend_status, resolve_session_title};
    use crate::managed_agents::BackendKind;

    #[test]
    fn remote_backend_status_is_none_for_local_so_caller_derives_from_process() {
        assert_eq!(
            remote_backend_status(&BackendKind::Local, None),
            None,
            "local status comes from live process state, not this mapping"
        );
    }

    #[test]
    fn remote_backend_status_reports_external_regardless_of_agent_id() {
        // External never gets a backend_agent_id (no deploy step). The provider
        // mapping would therefore pin it to "not_deployed" forever — this is the
        // regression that variant exists to avoid.
        assert_eq!(
            remote_backend_status(&BackendKind::External, None),
            Some("external")
        );
        assert_eq!(
            remote_backend_status(&BackendKind::External, Some("ignored")),
            Some("external")
        );
    }

    #[test]
    fn remote_backend_status_tracks_provider_deploy_state() {
        let provider = BackendKind::Provider {
            id: "blox".to_string(),
            config: serde_json::json!({}),
        };
        assert_eq!(remote_backend_status(&provider, None), Some("not_deployed"));
        assert_eq!(
            remote_backend_status(&provider, Some("agent-123")),
            Some("deployed")
        );
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
