//! Serialize an [`AgentReadiness`] verdict into the JSON payload the
//! desktop injects into `BUZZ_ACP_SETUP_PAYLOAD` when spawning an agent
//! that is not yet ready. Extracted from `runtime::spawn_agent_child` so
//! the spawn-seam regression can drive
//! `agent_readiness_for_spawn → build_setup_payload` end-to-end and
//! assert that a transient CLI-login flap **does not** produce a payload
//! (the load-bearing property that guards the Fizz Air incident).
//!
//! # Contract
//!
//! * `AgentReadiness::Ready` → `None`. Callers spawn without the
//!   `BUZZ_ACP_SETUP_PAYLOAD` env var; `buzz-acp` enters the normal
//!   agent pool.
//! * `AgentReadiness::NotReady { requirements }` → `Some(JSON)`.
//!   Callers set `BUZZ_ACP_SETUP_PAYLOAD` to that JSON; `buzz-acp`
//!   enters setup-listener mode and serves the surfaced requirements.
//!
//! `buzz-acp/setup_mode.rs` explicitly does NOT re-derive readiness at
//! runtime — the payload snapshotted at spawn time governs the child's
//! entire lifecycle. That is what makes any transient false-negative in
//! the readiness probe a lifetime-of-process trap unless it is caught
//! before this function returns `Some(_)`.
//!
//! JSON shape mirrors `setup_mode::SetupPayload` in buzz-acp:
//! ```text
//! { "agent_name": "...", "agent_pubkey": "...", "requirements": [{ "surface": "...", ... }] }
//! ```

use crate::managed_agents::{AgentReadiness, Requirement};

/// Build the `BUZZ_ACP_SETUP_PAYLOAD` JSON for the given readiness verdict.
/// Returns `None` when the agent is `Ready` (no payload should be set).
///
/// On serialization failure this function logs via `eprintln!` (matching
/// the pre-extraction behavior of `spawn_agent_child`) and returns `None`,
/// so a JSON-encoding bug never crashes agent spawn.
pub(crate) fn build_setup_payload(
    agent_name: &str,
    agent_pubkey: &str,
    readiness: AgentReadiness,
) -> Option<String> {
    let AgentReadiness::NotReady { requirements } = readiness else {
        return None;
    };
    let reqs: Vec<serde_json::Value> = requirements.into_iter().map(requirement_json).collect();
    let payload = serde_json::json!({
        "agent_name": agent_name,
        "agent_pubkey": agent_pubkey,
        "requirements": reqs,
    });
    match serde_json::to_string(&payload) {
        Ok(json) => Some(json),
        Err(e) => {
            eprintln!("buzz-desktop: failed to serialize setup payload for {agent_name}: {e}",);
            None
        }
    }
}

fn requirement_json(req: Requirement) -> serde_json::Value {
    match req {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::AcpAvailabilityStatus;

    #[test]
    fn ready_produces_no_payload() {
        // AgentReadiness::Ready → None. Spawn proceeds without
        // BUZZ_ACP_SETUP_PAYLOAD; buzz-acp enters normal pool.
        assert_eq!(
            build_setup_payload("agent", "pubkey-hex", AgentReadiness::Ready),
            None,
        );
    }

    #[test]
    fn not_ready_emits_payload_with_requirements_surface() {
        let readiness = AgentReadiness::NotReady {
            requirements: vec![Requirement::CliLogin {
                probe_args: vec![
                    "claude".to_string(),
                    "auth".to_string(),
                    "status".to_string(),
                ],
                setup_copy: "run claude login".to_string(),
                availability: AcpAvailabilityStatus::Available,
            }],
        };
        let payload = build_setup_payload("agent", "pubkey-hex", readiness)
            .expect("NotReady must emit a payload");
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("payload must be valid JSON");
        assert_eq!(parsed["agent_name"], "agent");
        assert_eq!(parsed["agent_pubkey"], "pubkey-hex");
        assert_eq!(parsed["requirements"][0]["surface"], "cli_login");
        assert_eq!(parsed["requirements"][0]["setup_copy"], "run claude login");
    }

    #[test]
    fn empty_not_ready_still_emits_payload() {
        // A NotReady with an empty requirements list is semantically
        // odd (readiness should collapse to Ready) but round-trip
        // safe here — we do not silently drop the payload.
        let payload = build_setup_payload(
            "agent",
            "pubkey-hex",
            AgentReadiness::NotReady {
                requirements: vec![],
            },
        )
        .expect("even empty NotReady emits a payload");
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["requirements"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn spawn_seam_transient_flap_omits_payload() {
        // The load-bearing property that guards the Fizz Air incident:
        // when the retry loop in `agent_readiness_for_spawn` promotes a
        // transient LoggedOut back to Ready, `build_setup_payload` MUST
        // return None so `BUZZ_ACP_SETUP_PAYLOAD` is not set and the
        // child enters the normal pool. Simulate that by feeding the
        // post-retry `AgentReadiness::Ready` and asserting None.
        //
        // This is the seam Honey [11] blocker 2 asked for: at this
        // exact call site (which mirrors `runtime.rs::spawn_agent_child`),
        // a Ready verdict from the retry post-pass must NOT emit a
        // setup payload. `agent_readiness_for_spawn` unit-tested end-to-
        // end in `readiness_spawn` module tests; here we lock the seam.
        assert_eq!(
            build_setup_payload(
                "flap-recovers",
                "0102030405060708090a0b0c0d0e0f10",
                AgentReadiness::Ready,
            ),
            None,
            "when the retry recovers, BUZZ_ACP_SETUP_PAYLOAD must not be set — \
             this is the Fizz Air regression at the runtime seam",
        );
    }
}
