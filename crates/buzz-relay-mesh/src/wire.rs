//! The mesh wire contract — FROZEN surface.
//!
//! Every byte that crosses the mesh is one of the frames in this module,
//! postcard-encoded behind a one-byte protocol version. This file is the
//! contract between all mesh lanes: transport (endpoint/peer), membership
//! (gossip/registry), the session directory, and the media fan-out all build
//! against these types. **Changes here require a post in the mesh thread
//! before the edit** — two lanes compiling against different frame layouts is
//! the failure mode this file exists to prevent.
//!
//! ## The fencing law (non-negotiable)
//!
//! Every session-bearing frame carries the fenced tuple
//! [`FencedHeader`] `{session_id, generation, owner_runtime_id}`. Receivers
//! MUST reject frames whose generation is stale for that session, at every
//! hop. Mesh membership is a hint; the fenced generation (Redis CAS lease)
//! is the arbiter. The mesh may say "don't dial" — it may never say "take
//! over."
//!
//! ## Framing
//!
//! - **Datagrams** (realtime-media): one [`MeshDatagram`] per QUIC datagram,
//!   postcard-encoded, no length prefix (the datagram boundary is the frame
//!   boundary). Senders MUST check the encoded size against the connection's
//!   `max_datagram_size()` and fail loud, never truncate.
//! - **Bi-streams** (reliable-stream + gossip control): length-delimited
//!   postcard. Each frame is a u32-LE length followed by that many bytes of
//!   postcard-encoded [`MeshStreamFrame`]. Max frame size: [`MAX_STREAM_FRAME`].
//!   The first frame on any stream MUST be `Hello`; a non-`Hello` first frame
//!   is a protocol error and the stream is reset.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// ALPN for the mesh QUIC endpoint. Version bumps get a new ALPN so old and
/// new pods never half-speak to each other during a rolling deploy.
pub const ALPN: &[u8] = b"buzz/mesh/1";

/// Wire protocol version, first byte of every encoded frame (datagram or
/// stream frame). Receivers MUST reject unknown versions loudly (count it,
/// log it) rather than guessing.
pub const WIRE_VERSION: u8 = 1;

/// Hard cap on a single length-delimited stream frame (16 MiB). Anything
/// larger is a protocol error, not a bigger buffer.
pub const MAX_STREAM_FRAME: u32 = 16 * 1024 * 1024;

/// A relay runtime's mesh identity: the ed25519 public key of the **mesh
/// endpoint keypair generated fresh at process start**. This is both the
/// iroh endpoint id and the boot-unique runtime id used in the ready
/// registry and ownership leases — one value, boot-unique by construction.
///
/// It is deliberately NOT the deployment's Nostr relay key: that key is
/// secp256k1, and the helm chart shares one `BUZZ_RELAY_PRIVATE_KEY` Secret
/// across all pods of a release — using it here would give every pod the
/// same runtime id and collapse the ownership plane (Wren's contract-review
/// blocker). Binding to the deployment identity is done out-of-band: the
/// ready-registry record carries a relay-key-signed attestation of the
/// runtime pubkey (membership lane), and peers accept mesh connections only
/// from endpoint ids present in attested registry/gossip records.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeId(pub [u8; 32]);

impl RuntimeId {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Debug for RuntimeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RuntimeId({}…)", &self.to_hex()[..8])
    }
}

impl std::fmt::Display for RuntimeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// The fenced tuple. Present on every session-bearing frame; checked at
/// every hop against the Redis lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FencedHeader {
    pub session_id: Uuid,
    /// Monotonic lease generation from the Redis CAS. A receiver that has
    /// observed generation G for a session rejects any frame with < G.
    pub generation: u64,
    /// The runtime the sender believes owns the session. Advisory for
    /// routing/diagnostics; the generation is what fences.
    pub owner_runtime_id: RuntimeId,
}

/// Tunnel profile, fixed at session establishment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// Ordered, reliable, backpressured (goose/berd). Rides `open_bi()`.
    ReliableStream,
    /// Lossy-by-design realtime media (huddle Opus). Rides QUIC datagrams.
    RealtimeMedia,
    /// Huddle roster/join/leave control. State-bearing — a dropped roster
    /// delta is an unrecoverable peer-index desync, so this rides a reliable
    /// stream like `ReliableStream`, never datagrams. Separate variant so
    /// routing intent and `/_mesh` counters stay legible.
    HuddleControl,
}

/// One QUIC datagram: realtime media only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshDatagram {
    pub fenced: FencedHeader,
    /// Sender-scoped monotonic sequence for loss/reorder observability.
    /// Receivers tolerate gaps and reordering; they never wait.
    pub seq: u64,
    /// Opaque at this layer: the profile owner defines the internal layout.
    /// For realtime media it is `[peer_index: u8][client frame]` — the
    /// peer_index is relay routing metadata (owner pod is sole allocator);
    /// the client frame's encrypted content is NIP-44 between client
    /// endpoints, so server-side plaintext of the media itself never exists.
    pub payload: Vec<u8>,
}

/// One length-delimited frame on a mesh bi-stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeshStreamFrame {
    /// MUST be the first frame on every stream, in both directions.
    Hello(StreamHello),
    /// Opaque tunnel bytes for a reliable-stream session.
    Data {
        fenced: FencedHeader,
        payload: Vec<u8>,
    },
    /// Clean close: the sender will send no more `Data` for this session.
    /// Distinct from a QUIC reset — receivers treat reset as abnormal.
    Goodbye {
        fenced: FencedHeader,
        reason: GoodbyeReason,
    },
    /// Membership gossip on the control stream (one per peer connection).
    /// Payload is the gossip lane's postcard-encoded digest/delta exchange —
    /// opaque at this layer so gossip can evolve without a wire bump here.
    Gossip { payload: Vec<u8> },
}

/// Stream role, declared in the Hello.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamRole {
    /// The per-connection control stream (gossip + liveness). Exactly one
    /// per peer connection, opened by the dialer immediately after connect.
    Control,
    /// A reliable-stream tunnel session.
    Session {
        fenced: FencedHeader,
        profile: Profile,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamHello {
    pub sender: RuntimeId,
    pub role: StreamRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoodbyeReason {
    /// Client closed / session ended normally.
    SessionEnded,
    /// This runtime is draining (SIGTERM) — re-establish elsewhere.
    Draining,
    /// The sender observed a newer generation and is fencing itself out.
    StaleGeneration,
}

/// Encode a frame: version byte + postcard.
pub fn encode<T: Serialize>(frame: &T) -> Result<Vec<u8>, crate::MeshError> {
    let buf = vec![WIRE_VERSION];
    postcard::to_extend(frame, buf).map_err(crate::MeshError::Encode)
}

/// Decode a frame: check version byte, then postcard.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, crate::MeshError> {
    match bytes.split_first() {
        Some((&WIRE_VERSION, rest)) => postcard::from_bytes(rest).map_err(crate::MeshError::Decode),
        Some((&v, _)) => Err(crate::MeshError::UnknownWireVersion(v)),
        None => Err(crate::MeshError::EmptyFrame),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fenced() -> FencedHeader {
        FencedHeader {
            session_id: Uuid::from_u128(0xDEAD_BEEF),
            generation: 42,
            owner_runtime_id: RuntimeId([7u8; 32]),
        }
    }

    #[test]
    fn datagram_roundtrip() {
        let d = MeshDatagram {
            fenced: fenced(),
            seq: 9001,
            payload: vec![1, 2, 3],
        };
        let bytes = encode(&d).unwrap();
        assert_eq!(bytes[0], WIRE_VERSION);
        let back: MeshDatagram = decode(&bytes).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn stream_frame_roundtrip() {
        for f in [
            MeshStreamFrame::Hello(StreamHello {
                sender: RuntimeId([1u8; 32]),
                role: StreamRole::Session {
                    fenced: fenced(),
                    profile: Profile::ReliableStream,
                },
            }),
            MeshStreamFrame::Data {
                fenced: fenced(),
                payload: b"opaque".to_vec(),
            },
            MeshStreamFrame::Goodbye {
                fenced: fenced(),
                reason: GoodbyeReason::Draining,
            },
            MeshStreamFrame::Gossip {
                payload: vec![0xAA; 16],
            },
        ] {
            let back: MeshStreamFrame = decode(&encode(&f).unwrap()).unwrap();
            assert_eq!(back, f);
        }
    }

    #[test]
    fn unknown_version_rejected() {
        let d = MeshDatagram {
            fenced: fenced(),
            seq: 1,
            payload: vec![],
        };
        let mut bytes = encode(&d).unwrap();
        bytes[0] = 99;
        assert!(matches!(
            decode::<MeshDatagram>(&bytes),
            Err(crate::MeshError::UnknownWireVersion(99))
        ));
    }

    /// Opus @ 20ms worst case (~160B) + header must clear the conservative
    /// QUIC datagram floor (~1200B path MTU minus QUIC overhead). This pins
    /// the header overhead so it can't silently grow past the budget.
    #[test]
    fn datagram_header_overhead_within_budget() {
        let payload = vec![0u8; 160];
        let d = MeshDatagram {
            fenced: fenced(),
            seq: u64::MAX,
            payload: payload.clone(),
        };
        let overhead = encode(&d).unwrap().len() - payload.len();
        assert!(
            overhead <= 64,
            "datagram header overhead {overhead}B exceeds 64B budget"
        );
    }
}
