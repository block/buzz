use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;

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
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub launch: Option<LaunchBlock>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LaunchBlock {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub policy_env: BTreeMap<String, String>,
    #[serde(default)]
    pub owner_pubkey: Option<String>,
}

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
        Self::Error(ErrorResponse {
            ok: false,
            error: message.into(),
        })
    }

    pub fn info() -> Self {
        Self::Info(InfoResponse {
            ok: true,
            name: "fly",
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            description: "Runs each agent in its own Fly.io app and Machine",
            config_schema: crate::config::config_schema(),
        })
    }

    pub fn deployed(agent_id: impl Into<String>) -> Self {
        Self::Deploy(DeployResponse {
            ok: true,
            agent_id: agent_id.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_desktop_request_id_and_extra_agent_fields() {
        let request: Request = serde_json::from_str(
            r#"{"op":"deploy","request_id":"r1","agent":{
                "name":"a","relay_url":"wss://relay","private_key_nsec":"nsec1x",
                "provider":"openai","model":"gpt"
            },"provider_config":{"region":"mad","image":"img@sha256:x"}}"#,
        )
        .unwrap();
        assert!(matches!(request, Request::Deploy(_)));
    }

    #[test]
    fn response_shape_is_flat() {
        let value = serde_json::to_value(Response::deployed("app/machine")).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["agent_id"], "app/machine");
    }
}
