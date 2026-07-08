//! `/_mesh` status data model.
//!
//! The relay's axum handler can serialize [`MeshStatus`] directly as JSON.

use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct MeshStatus {
    pub enabled: bool,
    pub local_runtime_id: String,
    pub draining: bool,
    pub peer_count: usize,
    pub peers: Vec<MeshPeerStatus>,
    pub counters: MeshCounters,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeshPeerStatus {
    pub runtime_id: String,
    pub endpoint_addrs: Vec<String>,
    pub proto_version: u16,
    pub draining: bool,
    pub connection_state: ConnectionState,
    pub phi: Option<f64>,
    pub load: f32,
    pub record_version: u64,
    pub last_heartbeat_millis: u64,
    pub counters: MeshPeerCounters,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Suspect,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MeshCounters {
    pub stale_generation_rejections: u64,
    pub peers: Vec<MeshPeerCounters>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MeshPeerCounters {
    pub runtime_id: String,
    pub streams_opened: u64,
    pub streams_received: u64,
    pub datagrams_sent: u64,
    pub datagrams_received: u64,
    pub gossip_frames_sent: u64,
    pub gossip_frames_received: u64,
    pub stale_generation_rejections: u64,
}
