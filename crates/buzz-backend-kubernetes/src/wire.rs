//! The stdin/stdout JSON protocol (spec §Provider Protocol).
//!
//! One process per operation: one JSON object in, one JSON object out.
//! These types are this provider's view of the contract; the golden fixtures
//! in `tests/fixtures/provider-wire/` are the arbiter shared with the desktop.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The wire-contract version this provider speaks (spec §Info).
pub const PROTOCOL_VERSION: u32 = 1;

/// Request envelope. `op` discriminates; unknown ops are an in-band error.
///
/// `request_id` is deliberately absent from every variant. The desktop sends
/// it, but the exchange is one request and one response per process, so there
/// is nothing to correlate and nothing in the response schema to echo it into
/// (`:387`, `:416` — neither response shape carries it). Serde ignores it on
/// the way in, so a caller that sends it is accepted; typing it would only
/// create a field nothing reads.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Info,
    Deploy(Box<DeployRequest>),
}

#[derive(Debug, Deserialize)]
pub struct DeployRequest {
    pub agent: AgentPayload,
    #[serde(default)]
    pub provider_config: serde_json::Value,
}

/// The agent payload (spec §Deploy).
///
/// Only the fields this binding actually consumes are typed. `name` (display
/// name), `model`, `provider`, and `turn_timeout_seconds` are deliberately
/// absent: object names derive from the pubkey, not the display name
/// (`:1150-1152`), the model/provider pair arrives already resolved inside
/// `launch`, and the timeout is ignored upstream — typing any of them would
/// invite a provider-side remap the spec forbids.
#[derive(Debug, Deserialize)]
pub struct AgentPayload {
    pub relay_url: String,
    pub private_key_nsec: String,
    #[serde(default)]
    pub auth_tag: Option<String>,
    #[serde(default)]
    pub respond_to: Option<String>,
    #[serde(default)]
    pub respond_to_allowlist: Option<Vec<String>>,
    /// User env, already merged global < persona < agent by the desktop and
    /// already stripped of reserved keys. Superseded by `launch.env` when
    /// `launch` is present — a provider MUST NOT re-merge it on top
    /// (§Launch data, precedence tier 2).
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    /// The desktop-resolved launch contract. Absent only from a desktop
    /// predating Known Defect 3's fix.
    #[serde(default)]
    pub launch: Option<LaunchBlock>,
}

/// Desktop-resolved launch data (spec §Launch data).
#[derive(Debug, Default, Deserialize)]
pub struct LaunchBlock {
    /// Command *name*, resolved against the image's PATH — never a host path.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Layered env: baked → runtime metadata → definition → global → persona
    /// → agent. Precedence tier 2.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Overridable behavior defaults. Precedence tier 1 — user env beats
    /// these, matching the local spawn.
    #[serde(default)]
    pub policy_env: BTreeMap<String, String>,
    /// Resolved workspace owner (hex). The respond-to gate's one
    /// irreducible input; without it or `auth_tag` the harness cannot match
    /// `!shutdown`.
    #[serde(default)]
    pub owner_pubkey: Option<String>,
}

/// Response envelope. Serialized flat — `{"ok": true, …}` — because the
/// desktop reads `ok`, `error`, and `agent_id` off the top level.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Info(InfoResponse),
    Deploy(DeployResponse),
    Error(ErrorResponse),
}

#[derive(Debug, Serialize)]
pub struct InfoResponse {
    pub ok: bool,
    pub name: &'static str,
    pub version: &'static str,
    pub protocol_version: u32,
    pub description: &'static str,
    pub config_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct DeployResponse {
    pub ok: bool,
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub error: String,
}

impl Response {
    pub fn error(message: impl Into<String>) -> Self {
        Response::Error(ErrorResponse {
            ok: false,
            error: message.into(),
        })
    }

    /// The provider's self-description (spec §Info). Pure — no cluster
    /// contact — because the desktop calls it to render the config form
    /// before a kubeconfig is known to exist.
    pub fn info() -> Self {
        Response::Info(InfoResponse {
            ok: true,
            name: "kubernetes",
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            description: "Runs agents as pods in a Kubernetes cluster",
            config_schema: crate::config::config_schema(),
        })
    }

    pub fn deployed(agent_id: impl Into<String>) -> Self {
        Response::Deploy(DeployResponse {
            ok: true,
            agent_id: agent_id.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_request() {
        let r: Request = serde_json::from_str(r#"{"op":"info","request_id":"abc"}"#).unwrap();
        assert!(matches!(r, Request::Info));
    }

    /// The desktop sends `request_id` on every call, but the exchange is 1:1
    /// per process — a provider that hard-required it would fail a
    /// conforming-but-minimal caller for no safety gain.
    #[test]
    fn request_id_is_optional() {
        let r: Request = serde_json::from_str(r#"{"op":"info"}"#).unwrap();
        assert!(matches!(r, Request::Info));
    }

    #[test]
    fn rejects_unknown_op() {
        assert!(serde_json::from_str::<Request>(r#"{"op":"undeploy"}"#).is_err());
    }

    /// Payload fields this binding does not consume must not break parsing:
    /// the desktop sends `model`, `provider`, `system_prompt` and more, and a
    /// provider that rejected them would break on every real deploy.
    #[test]
    fn ignores_unconsumed_payload_fields() {
        let json = r#"{
            "op":"deploy","request_id":"r1",
            "agent":{
                "name":"a","relay_url":"wss://r","private_key_nsec":"nsec1x",
                "model":"gpt-5","provider":"openai","system_prompt":"hi",
                "turn_timeout_seconds":30,"parallelism":10,
                "agent_command":"goose","agent_args":[]
            },
            "provider_config":{"namespace":"ns"}
        }"#;
        let r: Request = serde_json::from_str(json).unwrap();
        let Request::Deploy(d) = r else {
            panic!("wrong op")
        };
        assert_eq!(d.agent.relay_url, "wss://r");
        assert!(d.agent.launch.is_none());
    }

    #[test]
    fn parses_launch_block() {
        let json = r#"{
            "op":"deploy",
            "agent":{
                "relay_url":"wss://r","private_key_nsec":"nsec1x",
                "launch":{
                    "command":"goose","args":["run","--x"],
                    "env":{"GOOSE_MODEL":"m"},
                    "policy_env":{"GOOSE_MODE":"auto"},
                    "owner_pubkey":"deadbeef"
                }
            }
        }"#;
        let Request::Deploy(d) = serde_json::from_str::<Request>(json).unwrap() else {
            panic!("wrong op")
        };
        let l = d.agent.launch.unwrap();
        assert_eq!(l.command.as_deref(), Some("goose"));
        assert_eq!(l.args, ["run", "--x"]);
        assert_eq!(l.env["GOOSE_MODEL"], "m");
        assert_eq!(l.policy_env["GOOSE_MODE"], "auto");
        assert_eq!(l.owner_pubkey.as_deref(), Some("deadbeef"));
    }

    /// A null `owner_pubkey`/`auth_tag` must parse (the refusal is a policy
    /// decision made later, with a specific message), not fail as a type error.
    #[test]
    fn null_owner_fields_parse() {
        let json = r#"{"op":"deploy","agent":{
            "relay_url":"wss://r","private_key_nsec":"nsec1x","auth_tag":null,
            "launch":{"owner_pubkey":null}
        }}"#;
        let Request::Deploy(d) = serde_json::from_str::<Request>(json).unwrap() else {
            panic!("wrong op")
        };
        assert!(d.agent.auth_tag.is_none());
        assert!(d.agent.launch.unwrap().owner_pubkey.is_none());
    }

    /// The desktop reads `ok`/`error`/`agent_id` off the top level, so the
    /// enum must serialize flat with no variant tag.
    #[test]
    fn responses_serialize_flat() {
        let v = serde_json::to_value(Response::deployed("buzz-agent-abc")).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["agent_id"], "buzz-agent-abc");
        assert!(v.get("Deploy").is_none());

        let v = serde_json::to_value(Response::error("boom")).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "boom");
    }
}
