use crate::managed_agents::{AcpAvailabilityStatus, InstallStepResult};

pub(super) fn run(runtime_id: &str, steps: &mut Vec<InstallStepResult>) {
    // Observe PATH changes and binaries added after Buzz launched.
    crate::managed_agents::refresh_login_shell_path();
    crate::managed_agents::clear_resolve_cache();

    let availability = crate::managed_agents::discover_acp_runtimes()
        .into_iter()
        .find(|entry| entry.id == runtime_id)
        .map(|entry| entry.availability);
    if let Some(failure) = failure(runtime_id, availability) {
        steps.push(failure);
    }
}

fn failure(
    runtime_id: &str,
    availability: Option<AcpAvailabilityStatus>,
) -> Option<InstallStepResult> {
    if availability == Some(AcpAvailabilityStatus::Available) {
        return None;
    }

    let observed = availability
        .map(|status| format!("{status:?}"))
        .unwrap_or_else(|| "missing from the runtime catalog".to_string());
    Some(InstallStepResult {
        step: "verify".to_string(),
        command: format!("discover {runtime_id}"),
        success: false,
        stdout: String::new(),
        stderr: format!(
            "The installer finished, but Buzz still could not use {runtime_id} (observed: {observed})."
        ),
        exit_code: None,
        hint: Some(
            "Buzz requires the vendor CLI executable, not only its desktop app. If the CLI was installed while Buzz was open, restart Buzz and check again."
                .to_string(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_available_runtime() {
        assert!(failure("goose", Some(AcpAvailabilityStatus::Available)).is_none());
    }

    #[test]
    fn rejects_unresolved_runtime() {
        let failure = failure("goose", Some(AcpAvailabilityStatus::NotInstalled))
            .expect("not-installed runtime must fail verification");

        assert_eq!(failure.step, "verify");
        assert!(!failure.success);
        assert!(failure.stderr.contains("NotInstalled"));
        assert!(failure
            .hint
            .as_deref()
            .is_some_and(|hint| hint.contains("desktop app")));
    }
}
