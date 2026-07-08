//! buzz-relay-mesh — the inter-relay QUIC mesh.
//!
//! One iroh endpoint per relay runtime (identity = a boot-unique mesh
//! keypair, attested by the relay's signing key — see [`wire::RuntimeId`]),
//! a warm full mesh of authenticated connections, scuttlebutt membership
//! gossip on a control substream, and a fenced wire contract that carries
//! tunnel traffic (reliable streams + realtime datagrams) between pods.
//!
//! The relay consumes this crate exclusively through two seams:
//!
//! - [`RelayMeshMembership`] — "who is alive / draining / dialable?"
//! - [`RelayPeerTransport`] — "move these bytes to that runtime."
//!
//! The seams are what keep single-instance deployments and same-pod sessions
//! mesh-free: when `BUZZ_MESH=off` or no peers exist, the relay never
//! constructs a mesh and the in-process fast path is untouched.
//!
//! **The law:** mesh membership is a hint; the Redis fenced generation is the
//! arbiter. Nothing in this crate grants ownership — see [`wire::FencedHeader`].

pub mod endpoint;
pub mod gossip;
pub mod membership;
pub mod peer;
pub mod registry;
pub mod runtime;
pub mod status;
pub mod wire;

// Lane modules — one owner per file (see the mesh thread for lane map):
//   endpoint.rs, peer.rs        — Mari (transport core)
//   registry.rs, gossip.rs,
//   membership.rs, status.rs    — Max (membership + /_mesh)
// Session directory + tunnel routing live relay-side (Perci), consuming the
// seams below; huddle fan-out lives in buzz-relay's audio module (Dawn).

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

pub use gossip::{GossipDigestEntry, GossipMessage, GossipRecord, GossipState, PhiAccrual};
pub use membership::MeshMembership;
pub use registry::{ReadyHeartbeat, ReadyRecord, ReadyRegistry, RuntimeAttestation};
pub use runtime::MeshRuntime;
pub use status::{ConnectionState, MeshCounters, MeshPeerCounters, MeshPeerStatus, MeshStatus};
pub use wire::{
    FencedHeader, GoodbyeReason, MeshDatagram, MeshStreamFrame, Profile, RuntimeId, StreamHello,
    StreamRole, ALPN, WIRE_VERSION,
};

/// Mesh configuration, resolved from env by the relay.
#[derive(Clone, Debug)]
pub struct MeshConfig {
    /// `BUZZ_MESH` — `on` (default when replicas can exist) | `off` kill
    /// switch. When off, the relay must behave exactly like single-instance.
    pub enabled: bool,
    /// UDP bind for the iroh endpoint (`BUZZ_MESH_BIND_ADDR`, default
    /// `0.0.0.0:3478`). Excluded from istio sidecar capture in k8s.
    pub bind_addr: std::net::SocketAddr,
    /// Ready-registry heartbeat refresh (default 15s; expiry is 3x).
    pub registry_refresh: std::time::Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    #[error("frame encode: {0}")]
    Encode(#[source] postcard::Error),
    #[error("frame decode: {0}")]
    Decode(#[source] postcard::Error),
    #[error("unknown wire version {0}")]
    UnknownWireVersion(u8),
    #[error("empty frame")]
    EmptyFrame,
    #[error("frame exceeds max size ({size} > {max})")]
    FrameTooLarge { size: usize, max: usize },
    #[error("datagram exceeds connection max_datagram_size ({size} > {max})")]
    DatagramTooLarge { size: usize, max: usize },
    #[error("peer {0} not connected")]
    PeerNotConnected(RuntimeId),
    #[error("peer {0} is draining")]
    PeerDraining(RuntimeId),
    #[error("stale generation for session {session_id}: frame {frame_generation} < known {known_generation}")]
    StaleGeneration {
        session_id: uuid::Uuid,
        frame_generation: u64,
        known_generation: u64,
    },
    // The three variants below complete the fence-rejection taxonomy alongside
    // `StaleGeneration` (Wren's chaos-gate ruling: every fence-visible reject
    // is a typed variant, never a generic `Transport`, so live kill-9 /
    // partition / replay evidence is unambiguous). Counter surface:
    // `mesh_fence_rejections_total{reason=...}` with reasons
    // `stale_generation` | `no_active_lease` | `owner_mismatch` |
    // `future_generation`. None of these are serialized — the wire-level fence
    // signal remains `GoodbyeReason::StaleGeneration`.
    #[error("no active lease for session {session_id}: frame generation {frame_generation}, known generation {known_generation}, claimed owner {frame_owner_runtime_id}")]
    NoActiveLease {
        session_id: uuid::Uuid,
        frame_generation: u64,
        known_generation: u64,
        /// The owner the *frame* claimed — there is no current owner by
        /// definition when no live lease exists.
        frame_owner_runtime_id: RuntimeId,
    },
    #[error("owner mismatch for session {session_id} generation {generation}: frame owner {frame_owner_runtime_id} != current owner {current_owner_runtime_id}")]
    OwnerMismatch {
        session_id: uuid::Uuid,
        generation: u64,
        frame_owner_runtime_id: RuntimeId,
        current_owner_runtime_id: RuntimeId,
    },
    #[error("future generation for session {session_id}: frame {frame_generation} > known {known_generation}")]
    FutureGeneration {
        session_id: uuid::Uuid,
        frame_generation: u64,
        known_generation: u64,
    },
    #[error("mesh is disabled (BUZZ_MESH=off)")]
    Disabled,
    #[error("transport: {0}")]
    Transport(String),
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
}

/// A peer as membership sees it. Everything here is a routing HINT.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub runtime_id: RuntimeId,
    pub draining: bool,
    /// Phi-accrual suspicion; `None` until enough heartbeats observed.
    pub phi: Option<f64>,
    /// Advisory load factor gossiped by the peer (0.0..).
    pub load: f32,
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Seam 1: membership. Answers "who can I route to?" — never "who owns what."
pub trait RelayMeshMembership: Send + Sync + 'static {
    /// Live, non-suspect peers (self excluded).
    fn peers(&self) -> Vec<PeerInfo>;
    /// This runtime's mesh identity.
    fn local_runtime_id(&self) -> RuntimeId;
    /// Begin drain: gossip `draining=true`, stop accepting new sessions.
    fn begin_drain(&self);
}

/// Seam 2: transport. Moves fenced bytes to a specific runtime.
///
/// Implementations perform the datagram-size and wire-version checks; they do
/// NOT perform generation fencing — that belongs to the session layer on both
/// ends (fencing at every hop means every consumer checks, not the pipe).
pub trait RelayPeerTransport: Send + Sync + 'static {
    /// Fire-and-forget realtime datagram (drop-on-full, never blocks on old
    /// audio). Errors only for disconnected peer / oversize frame.
    fn send_datagram(&self, to: RuntimeId, dgram: MeshDatagram) -> Result<(), MeshError>;

    /// Open a reliable bi-stream to a peer for a session (`ReliableStream`
    /// or `HuddleControl` profile). Sends the `Hello` before returning.
    fn open_session_stream(
        &self,
        to: RuntimeId,
        hello: StreamHello,
    ) -> BoxFuture<'_, Result<MeshStream, MeshError>>;

    /// Register the handler invoked for inbound datagrams / session streams.
    /// Called once at relay startup.
    fn set_inbound(&self, handler: Box<dyn InboundHandler>);
}

/// Inbound mesh traffic, delivered after wire decode + Hello validation.
pub trait InboundHandler: Send + Sync + 'static {
    fn on_datagram(&self, from: RuntimeId, dgram: MeshDatagram);
    fn on_session_stream(&self, from: RuntimeId, hello: StreamHello, stream: MeshStream);
}

/// A reliable mesh stream: length-delimited `MeshStreamFrame`s over QUIC.
/// Concrete type (not a trait) so lanes share one framing implementation.
pub struct MeshStream {
    // Mari: wrap iroh SendStream/RecvStream with the u32-LE length framing
    // from `wire`. Placeholder halves keep the seam compilable pre-transport.
    pub(crate) send: Box<dyn StreamSendHalf>,
    pub(crate) recv: Box<dyn StreamRecvHalf>,
}

pub trait StreamSendHalf: Send + 'static {
    fn send_frame(&mut self, frame: MeshStreamFrame) -> BoxFuture<'_, Result<(), MeshError>>;
    fn finish(&mut self) -> Result<(), MeshError>;
}

pub trait StreamRecvHalf: Send + 'static {
    fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>>;
}

impl MeshStream {
    pub fn send_frame(&mut self, frame: MeshStreamFrame) -> BoxFuture<'_, Result<(), MeshError>> {
        self.send.send_frame(frame)
    }
    pub fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>> {
        self.recv.recv_frame()
    }
    pub fn finish(&mut self) -> Result<(), MeshError> {
        self.send.finish()
    }
}

/// Raw bytes helper used by transport internals.
pub fn encode_datagram_checked(
    dgram: &MeshDatagram,
    max_datagram_size: usize,
) -> Result<Bytes, MeshError> {
    let bytes = wire::encode(dgram)?;
    if bytes.len() > max_datagram_size {
        return Err(MeshError::DatagramTooLarge {
            size: bytes.len(),
            max: max_datagram_size,
        });
    }
    Ok(Bytes::from(bytes))
}
