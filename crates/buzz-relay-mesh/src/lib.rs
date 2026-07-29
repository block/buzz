#![warn(missing_docs)]
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

/// QUIC endpoint wrapper and connection lifecycle.
pub mod endpoint;
/// Scuttlebutt membership gossip state machine and digest exchange.
pub mod gossip;
/// Phi-accrual membership tracker and live peer view.
pub mod membership;
/// Per-peer connection state and framed transport halves.
pub mod peer;
/// Ready registry: Redis-backed attestation/heartbeat directory of runtimes.
pub mod registry;
/// [`MeshRuntime`] — the trait iroh-backed runtimes implement.
pub mod runtime;
/// `/_mesh` status snapshot types surfaced to operators.
pub mod status;
/// The fenced wire contract: framing, version, and headers.
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

/// Errors raised by the mesh transport and wire codec.
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    /// Failed to encode a wire frame.
    #[error("frame encode: {0}")]
    Encode(#[source] postcard::Error),
    /// Failed to decode a wire frame.
    #[error("frame decode: {0}")]
    Decode(#[source] postcard::Error),
    /// Encountered an unrecognized wire protocol version.
    #[error("unknown wire version {0}")]
    UnknownWireVersion(u8),
    /// Received an empty (zero-length) frame.
    #[error("empty frame")]
    EmptyFrame,
    /// A frame exceeded the configured maximum size.
    #[error("frame exceeds max size ({size} > {max})")]
    FrameTooLarge {
        /// Actual frame size in bytes.
        size: usize,
        /// Configured maximum frame size in bytes.
        max: usize,
    },
    /// A datagram exceeded the connection's `max_datagram_size`.
    #[error("datagram exceeds connection max_datagram_size ({size} > {max})")]
    DatagramTooLarge {
        /// Actual datagram size in bytes.
        size: usize,
        /// Connection maximum datagram size in bytes.
        max: usize,
    },
    /// The target peer has no live mesh connection.
    #[error("peer {0} not connected")]
    PeerNotConnected(RuntimeId),
    /// The target peer is draining and refuses new traffic.
    #[error("peer {0} is draining")]
    PeerDraining(RuntimeId),
    /// The frame's generation is older than the known generation for the session.
    #[error("stale generation for session {session_id}: frame {frame_generation} < known {known_generation}")]
    StaleGeneration {
        /// The session the stale frame targeted.
        session_id: uuid::Uuid,
        /// Generation claimed by the rejected frame.
        frame_generation: u64,
        /// Highest generation known for the session.
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
    /// A frame arrived for a session with no live lease on this runtime.
    #[error("no active lease for session {session_id}: frame generation {frame_generation}, known generation {known_generation}, claimed owner {frame_owner_runtime_id}")]
    NoActiveLease {
        /// The session the frame targeted.
        session_id: uuid::Uuid,
        /// Generation claimed by the frame.
        frame_generation: u64,
        /// Highest generation known for the session.
        known_generation: u64,
        /// The owner the *frame* claimed — there is no current owner by
        /// definition when no live lease exists.
        frame_owner_runtime_id: RuntimeId,
    },
    /// The frame's claimed owner does not match the session's current owner.
    #[error("owner mismatch for session {session_id} generation {generation}: frame owner {frame_owner_runtime_id} != current owner {current_owner_runtime_id}")]
    OwnerMismatch {
        /// The session whose owner was contested.
        session_id: uuid::Uuid,
        /// Generation at which the mismatch was detected.
        generation: u64,
        /// Owner claimed by the frame.
        frame_owner_runtime_id: RuntimeId,
        /// The runtime that actually owns the session.
        current_owner_runtime_id: RuntimeId,
    },
    /// The frame carried a generation ahead of any known lease (replay/forge signal).
    #[error("future generation for session {session_id}: frame {frame_generation} > known {known_generation}")]
    FutureGeneration {
        /// The session the frame targeted.
        session_id: uuid::Uuid,
        /// Generation claimed by the frame.
        frame_generation: u64,
        /// Highest generation known for the session.
        known_generation: u64,
    },
    /// The mesh is disabled via `BUZZ_MESH=off`.
    #[error("mesh is disabled (BUZZ_MESH=off)")]
    Disabled,
    /// Lower-level transport failure not covered by a more specific variant.
    #[error("transport: {0}")]
    Transport(String),
    /// A Redis operation failed.
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
}

/// A peer as membership sees it. Everything here is a routing HINT.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// The peer's mesh identity.
    pub runtime_id: RuntimeId,
    /// Whether the peer has begun draining and should shed load.
    pub draining: bool,
    /// Phi-accrual suspicion; `None` until enough heartbeats observed.
    pub phi: Option<f64>,
    /// Advisory load factor gossiped by the peer (0.0..).
    pub load: f32,
}

/// Boxed future used across the seam traits. Public because implementors of
/// [`StreamSendHalf`]/[`StreamRecvHalf`]/[`RelayPeerTransport`] outside this
/// crate must name it.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
    /// Called for each inbound realtime datagram after wire decode.
    fn on_datagram(&self, from: RuntimeId, dgram: MeshDatagram);
    /// Called for each inbound session stream after `Hello` validation.
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

/// Write half of a framed mesh stream.
pub trait StreamSendHalf: Send + 'static {
    /// Send a single length-delimited frame.
    fn send_frame(&mut self, frame: MeshStreamFrame) -> BoxFuture<'_, Result<(), MeshError>>;
    /// Finish the send half, signalling clean EOF to the peer.
    fn finish(&mut self) -> Result<(), MeshError>;
}

/// Read half of a framed mesh stream.
pub trait StreamRecvHalf: Send + 'static {
    /// Receive the next frame, or `Ok(None)` at clean EOF.
    fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>>;
}

impl MeshStream {
    /// Send a single length-delimited frame on the underlying stream.
    pub fn send_frame(&mut self, frame: MeshStreamFrame) -> BoxFuture<'_, Result<(), MeshError>> {
        self.send.send_frame(frame)
    }
    /// Receive the next frame, or `Ok(None)` at clean EOF.
    pub fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>> {
        self.recv.recv_frame()
    }
    /// Finish the send half, signalling clean EOF to the peer.
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
