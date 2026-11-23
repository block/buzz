//! Re-probe CLI-login requirements while an agent waits in setup mode.
//!
//! The desktop runs its readiness probe once, at spawn. If a CLI login is
//! missing at that moment (expired OAuth session, keychain not yet updated),
//! the agent enters setup mode. Without a re-probe it stays there until a
//! manual restart, even after the user fixes the login — the guard hides the
//! only recovery path (AGENTS.md rule 6).
//!
//! Only `cli_login` requirements with `availability: available` can heal
//! without a respawn: the fix happens outside the process (the user logs in)
//! and the probe sees it. Every other requirement (env keys, model fields,
//! missing binaries, invalid config) needs new spawn-time config, so a payload
//! that contains one is never re-probed.

use std::process::Stdio;
use std::time::Duration;

use crate::setup_mode::{AcpAvailabilityStatus, RequirementPayload, SetupPayload};

/// Delay before the first re-probe after entering setup mode.
pub(crate) const INITIAL_REPROBE_DELAY: Duration = Duration::from_secs(30);
/// Upper bound for the re-probe backoff.
pub(crate) const MAX_REPROBE_DELAY: Duration = Duration::from_secs(300);
/// Hard limit for one probe process; a hung CLI must not stall the loop.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Returns the probe commands when every requirement can heal in place.
///
/// `None` means the payload has no requirements, or at least one requirement
/// that only a respawn with new config can satisfy.
pub(crate) fn retryable_probes(payload: &SetupPayload) -> Option<Vec<Vec<String>>> {
    if payload.requirements.is_empty() {
        return None;
    }
    payload
        .requirements
        .iter()
        .map(|requirement| match requirement {
            RequirementPayload::CliLogin {
                probe_args,
                availability: AcpAvailabilityStatus::Available,
                ..
            } if !probe_args.is_empty() => Some(probe_args.clone()),
            _ => None,
        })
        .collect()
}

/// Exponential backoff for the next re-probe, capped at [`MAX_REPROBE_DELAY`].
pub(crate) fn next_reprobe_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_REPROBE_DELAY)
}

/// Returns `true` when every probe exits successfully.
///
/// Probes run in sequence and stop at the first failure.
pub(crate) async fn all_probes_pass(probes: &[Vec<String>]) -> bool {
    for probe in probes {
        if !probe_passes(probe, PROBE_TIMEOUT).await {
            return false;
        }
    }
    true
}

/// Run one probe. Exit 0 means logged in — the same rule as the desktop probe.
///
/// Spawn errors, non-zero exits and timeouts all count as "not yet". Output is
/// discarded, so a noisy CLI cannot grow memory.
async fn probe_passes(probe_args: &[String], timeout: Duration) -> bool {
    let Some((program, args)) = probe_args.split_first() else {
        return false;
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::debug!("setup-mode: re-probe `{program}` could not start: {e}");
            return false;
        }
    };
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(e)) => {
            tracing::debug!("setup-mode: re-probe `{program}` wait failed: {e}");
            false
        }
        Err(_) => {
            tracing::warn!(
                "setup-mode: re-probe `{program}` exceeded {}s — killed",
                timeout.as_secs()
            );
            // kill_on_drop reaps the child when it goes out of scope.
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(json: &str) -> SetupPayload {
        SetupPayload::from_raw_env_value(Some(json.to_string()))
            .expect("valid payload")
            .expect("payload present")
    }

    fn cli_login(availability: &str) -> String {
        format!(
            r#"{{"surface":"cli_login","probe_args":["claude","auth","status"],"setup_copy":"x","availability":"{availability}"}}"#
        )
    }

    #[test]
    fn available_cli_login_is_retryable() {
        let p = payload(&format!(
            r#"{{"agent_name":"Opus","agent_pubkey":"pk","requirements":[{}]}}"#,
            cli_login("available")
        ));
        assert_eq!(
            retryable_probes(&p),
            Some(vec![vec![
                "claude".to_string(),
                "auth".to_string(),
                "status".to_string()
            ]])
        );
    }

    #[test]
    fn missing_tooling_is_not_retryable() {
        let p = payload(&format!(
            r#"{{"agent_name":"Opus","agent_pubkey":"pk","requirements":[{}]}}"#,
            cli_login("cli_missing")
        ));
        assert_eq!(retryable_probes(&p), None);
    }

    #[test]
    fn mixed_requirements_are_not_retryable() {
        let p = payload(&format!(
            r#"{{"agent_name":"Opus","agent_pubkey":"pk","requirements":[{},{{"surface":"env_key","key":"ANTHROPIC_API_KEY"}}]}}"#,
            cli_login("available")
        ));
        assert_eq!(retryable_probes(&p), None);
    }

    #[test]
    fn empty_requirements_are_not_retryable() {
        let p = payload(r#"{"agent_name":"Opus","agent_pubkey":"pk","requirements":[]}"#);
        assert_eq!(retryable_probes(&p), None);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(
            next_reprobe_delay(INITIAL_REPROBE_DELAY),
            Duration::from_secs(60)
        );
        assert_eq!(
            next_reprobe_delay(Duration::from_secs(200)),
            MAX_REPROBE_DELAY
        );
        assert_eq!(next_reprobe_delay(MAX_REPROBE_DELAY), MAX_REPROBE_DELAY);
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".to_string(), "-c".to_string(), script.to_string()]
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_exit_zero_passes() {
        assert!(all_probes_pass(&[sh("exit 0")]).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_exit_nonzero_fails() {
        assert!(!all_probes_pass(&[sh("exit 0"), sh("exit 1")]).await);
    }

    #[tokio::test]
    async fn missing_binary_fails() {
        assert!(!all_probes_pass(&[vec!["buzz-no-such-binary-xyz".to_string()]]).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hung_probe_times_out() {
        let started = std::time::Instant::now();
        assert!(!probe_passes(&sh("sleep 30"), Duration::from_millis(200)).await);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
