//! `/_mesh` status data model.
//!
//! The relay's axum handler can serialize [`MeshStatus`] directly as JSON.

use serde::Serialize;

/// Top-level `/_mesh` status payload, serialized directly as JSON.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MeshStatus {
    /// Whether the mesh is enabled for this runtime.
    pub enabled: bool,
    /// This runtime's mesh identity (hex).
    pub local_runtime_id: String,
    /// Whether this runtime is draining.
    pub draining: bool,
    /// Number of known peers.
    pub peer_count: usize,
    /// Per-peer status entries.
    pub peers: Vec<MeshPeerStatus>,
    /// Aggregate counters.
    pub counters: MeshCounters,
}

/// Status of a single peer as surfaced to operators.
#[derive(Clone, Debug, Serialize)]
pub struct MeshPeerStatus {
    /// The peer's mesh identity (hex).
    pub runtime_id: String,
    /// Dialable endpoint address strings.
    pub endpoint_addrs: Vec<String>,
    /// Protocol version the peer speaks.
    pub proto_version: u16,
    /// Whether the peer is draining.
    pub draining: bool,
    /// Last observed connection state.
    pub connection_state: ConnectionState,
    /// Phi-accrual suspicion score, if enough heartbeats observed.
    pub phi: Option<f64>,
    /// Advisory load factor gossiped by the peer.
    pub load: f32,
    /// Last gossiped record version.
    pub record_version: u64,
    /// Last heartbeat timestamp (ms since UNIX_EPOCH).
    pub last_heartbeat_millis: u64,
    /// Per-peer counters.
    pub counters: MeshPeerCounters,
}

/// Observed transport state of a peer connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// No live connection to the peer.
    #[default]
    Disconnected,
    /// A connection attempt is in flight.
    Connecting,
    /// Authenticated connection is established.
    Connected,
    /// Peer is considered suspect by the phi-accrual detector.
    Suspect,
}

/// Aggregate counters across all peers.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MeshCounters {
    /// Frames rejected because their generation was stale.
    pub stale_generation_rejections: u64,
    /// Ready-registry seeds rejected because their `relay_pubkey` did not
    /// match this deployment's relay identity (or no anchor was configured).
    pub foreign_relay_rejections: u64,
    /// Per-peer counters, in the same order as `MeshStatus::peers`.
    pub peers: Vec<MeshPeerCounters>,
}

/// Counters for a single peer.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MeshPeerCounters {
    /// The peer's mesh identity (hex).
    pub runtime_id: String,
    /// Reliable streams opened to the peer.
    pub streams_opened: u64,
    /// Reliable streams received from the peer.
    pub streams_received: u64,
    /// Datagrams sent to the peer.
    pub datagrams_sent: u64,
    /// Datagrams received from the peer.
    pub datagrams_received: u64,
    /// Gossip control frames sent to the peer.
    pub gossip_frames_sent: u64,
    /// Gossip control frames received from the peer.
    pub gossip_frames_received: u64,
    /// Stale-generation fence rejections attributed to the peer.
    pub stale_generation_rejections: u64,
}
