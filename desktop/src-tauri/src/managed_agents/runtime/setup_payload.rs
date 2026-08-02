//! Setup-listener payload construction.
//!
//! When the desktop determines an agent is not ready to run, it serializes the
//! missing requirements into `BUZZ_ACP_SETUP_PAYLOAD`. `buzz-acp` detects that
//! env var at startup and enters the minimal setup-listener mode instead of the
//! agent pool.
//!
//! The desktop is the sole readiness source; `buzz-acp` only transports the
//! payload. The JSON shape mirrors `setup_mode::SetupPayload` in `buzz-acp`:
//!
//! ```json
//! { "agent_name": "…", "agent_pubkey": "…", "requirements": [{ "surface": "…", … }] }
//! ```
//!
//! Kept pure (no `AppHandle`, no `Command`) so the surface mapping is
//! unit-testable without spawning a process.

use crate::managed_agents::Requirement;

/// Serialize one missing requirement into its wire form, tagged with the UI
/// surface that owns it.
fn requirement_json(requirement: Requirement) -> serde_json::Value {
    match requirement {
        Requirement::NormalizedField { field } => serde_json::json!({
            "surface": "normalized_field",
            "field": field,
        }),
        Requirement::EnvKey { key } => serde_json::json!({
            "surface": "env_key",
            "key": key,
        }),
        Requirement::CliLogin {
            probe_args,
            setup_copy,
            availability,
        } => serde_json::json!({
            "surface": "cli_login",
            "probe_args": probe_args,
            "setup_copy": setup_copy,
            "availability": availability,
        }),
        Requirement::CliConfigInvalid {
            probe_args,
            setup_copy,
            diagnostic,
        } => serde_json::json!({
            "surface": "cli_config_invalid",
            "probe_args": probe_args,
            "setup_copy": setup_copy,
            "diagnostic": diagnostic,
        }),
        Requirement::GitBash => serde_json::json!({
            "surface": "git_bash",
        }),
        Requirement::MissingBinary { command } => serde_json::json!({
            "surface": "missing_binary",
            "command": command,
        }),
    }
}

/// Build the `BUZZ_ACP_SETUP_PAYLOAD` JSON for an agent that is not ready.
///
/// Returns `None` when serialization fails — the caller then spawns normally
/// rather than in setup mode, which is the safe degradation: a failed payload
/// must not silently strand the agent in a listener with no requirements.
pub(super) fn build_setup_payload_json(
    agent_name: &str,
    agent_pubkey: &str,
    requirements: Vec<Requirement>,
) -> Option<String> {
    let reqs: Vec<serde_json::Value> = requirements.into_iter().map(requirement_json).collect();
    let payload = serde_json::json!({
        "agent_name": agent_name,
        "agent_pubkey": agent_pubkey,
        "requirements": reqs,
    });
    match serde_json::to_string(&payload) {
        Ok(json) => Some(json),
        Err(e) => {
            eprintln!("buzz-desktop: failed to serialize setup payload for {agent_name}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_serializes_its_surface_and_command() {
        let json = requirement_json(Requirement::MissingBinary {
            command: "my-acp-agent".to_string(),
        });
        assert_eq!(json["surface"], "missing_binary");
        assert_eq!(json["command"], "my-acp-agent");
    }

    #[test]
    fn git_bash_serializes_surface_only() {
        let json = requirement_json(Requirement::GitBash);
        assert_eq!(json["surface"], "git_bash");
    }

    #[test]
    fn payload_carries_identity_and_every_requirement() {
        let json = build_setup_payload_json(
            "Ada",
            "npub-abc",
            vec![
                Requirement::EnvKey {
                    key: "ANTHROPIC_API_KEY".to_string(),
                },
                Requirement::MissingBinary {
                    command: "my-acp-agent".to_string(),
                },
            ],
        )
        .expect("payload serializes");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["agent_name"], "Ada");
        assert_eq!(parsed["agent_pubkey"], "npub-abc");
        assert_eq!(parsed["requirements"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["requirements"][0]["surface"], "env_key");
        assert_eq!(parsed["requirements"][1]["surface"], "missing_binary");
    }

    #[test]
    fn ready_agent_payload_has_no_requirements() {
        let json = build_setup_payload_json("Ada", "npub-abc", vec![]).expect("payload serializes");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert!(parsed["requirements"].as_array().unwrap().is_empty());
    }
}
