use serde::{Deserialize, Serialize};

use super::MeshHealth;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeshNodeMode {
    Serve,
    Client,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeshNodeState {
    Off,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartMeshNodeRequest {
    pub mode: MeshNodeMode,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub max_vram_gb: Option<u64>,
    #[serde(default)]
    pub join_token: Option<String>,
    /// Stable, relay-scoped mesh name injected by the Buzz backend. It is not
    /// accepted from the frontend and contains no relay address.
    #[serde(default, skip_deserializing)]
    pub mesh_name: Option<String>,
    /// Buzz community relay this runtime is pinned to. Backend-owned: changing
    /// the active community must not silently move an existing mesh runtime.
    #[serde(default, skip_deserializing)]
    pub relay_url: Option<String>,
    /// Mesh owner ids admitted to this node (the member roster from
    /// member-signed discovery notes). `None` = caller did not resolve a roster
    /// (tests, direct invocations): the node runs without allowlist
    /// enforcement, matching an open relay. `Some` = enforce
    /// `TrustPolicy::Allowlist` over exactly these owners (self is always
    /// included by the caller).
    #[serde(default)]
    pub trusted_owner_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshNodeStatus {
    pub state: MeshNodeState,
    pub mode: Option<MeshNodeMode>,
    pub health: MeshHealth,
    pub api_base_url: Option<String>,
    pub console_url: Option<String>,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// Community relay this runtime is serving or consuming through.
    pub community_relay_url: Option<String>,
}

pub fn stopped_status() -> MeshNodeStatus {
    MeshNodeStatus {
        state: MeshNodeState::Off,
        mode: None,
        health: MeshHealth::ok(),
        api_base_url: None,
        console_url: None,
        model_id: None,
        model_name: None,
        invite_token: None,
        endpoint_id: None,
        device_id: None,
        device_name: None,
        community_relay_url: None,
    }
}
