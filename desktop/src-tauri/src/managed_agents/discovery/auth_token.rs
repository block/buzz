//! Token-in-environment auth evidence for ACP runtimes.
//!
//! Some runtimes (notably `claude`) spawn an adapter that honours an OAuth
//! token env var the sibling CLI probe cannot see. When that token is set, it
//! is evidence about the process that will actually authenticate, so it wins
//! over the probe and short-circuits it.

use crate::managed_agents::{AcpAvailabilityStatus, AuthStatus};

use super::PartialEntry;

/// True when any declared env var is present and non-empty after trim.
pub(crate) fn auth_token_env_var_present(
    env_vars: &[&str],
    lookup: impl Fn(&str) -> Option<String>,
) -> bool {
    env_vars
        .iter()
        .any(|name| lookup(name).is_some_and(|value| !value.trim().is_empty()))
}

/// Mark available runtimes with a set auth token as logged in before probing.
pub(super) fn apply_token_auth_evidence(partials: &mut [PartialEntry]) {
    for partial in partials {
        if partial.entry.availability == AcpAvailabilityStatus::Available
            && auth_token_env_var_present(partial.runtime.auth_token_env_vars, |name| {
                std::env::var(name).ok()
            })
        {
            partial.entry.auth_status = AuthStatus::LoggedIn;
            partial.entry.login_hint = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::auth_token_env_var_present;
    use super::super::KNOWN_ACP_RUNTIMES;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'static {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn a_set_token_counts_as_auth_evidence() {
        assert!(auth_token_env_var_present(
            &["CLAUDE_CODE_OAUTH_TOKEN"],
            env_from(&[("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-abc")]),
        ));
    }

    #[test]
    fn an_unset_token_is_not_evidence() {
        assert!(!auth_token_env_var_present(
            &["CLAUDE_CODE_OAUTH_TOKEN"],
            env_from(&[]),
        ));
    }

    #[test]
    fn an_empty_or_whitespace_token_is_not_evidence() {
        for value in ["", "   ", "\t\n"] {
            assert!(
                !auth_token_env_var_present(
                    &["CLAUDE_CODE_OAUTH_TOKEN"],
                    env_from(&[("CLAUDE_CODE_OAUTH_TOKEN", value)]),
                ),
                "{value:?} must not count"
            );
        }
    }

    #[test]
    fn a_runtime_declaring_no_token_vars_is_never_short_circuited() {
        assert!(!auth_token_env_var_present(
            &[],
            env_from(&[("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-abc")]),
        ));
    }

    #[test]
    fn any_one_of_several_declared_vars_is_enough() {
        assert!(auth_token_env_var_present(
            &["FIRST_TOKEN", "SECOND_TOKEN"],
            env_from(&[("SECOND_TOKEN", "value")]),
        ));
    }

    #[test]
    fn claude_declares_the_adapter_token_and_other_runtimes_do_not() {
        let claude = KNOWN_ACP_RUNTIMES
            .iter()
            .find(|runtime| runtime.id == "claude")
            .expect("claude runtime");
        assert_eq!(claude.auth_token_env_vars, &["CLAUDE_CODE_OAUTH_TOKEN"]);

        for runtime in KNOWN_ACP_RUNTIMES.iter().filter(|r| r.id != "claude") {
            assert!(
                runtime.auth_token_env_vars.is_empty(),
                "{} must not claim token evidence without a documented reason",
                runtime.id
            );
        }
    }
}
