//! Cross-pod huddle audio: owner fan-out over the relay mesh.
//!
//! Today a huddle's audio only fans out within a single pod
//! ([`super::room::Room::broadcast_frame`]). Under horizontal scaling, two
//! participants can land on different pods and never hear each other — which is
//! why [`super::handler`] rejects joins with `huddle_audio_unavailable` when the
//! deployment sets `huddle_audio_available = false`. This module removes that
//! wall by routing audio across the mesh to the pod that *owns* the huddle.
//!
//! ## Owner-authoritative model
//!
//! One pod owns a huddle: the holder of the Redis fenced CAS lease for
//! `session_id == channel_id` (the session directory, Perci's lane, exposed to
//! us through [`HuddleOwnerDirectory`]). That pod hosts the single
//! [`Room`](super::room::Room) —
//! the sole allocator of the 0..=254 `peer_index` space, so indices can never
//! collide across pods. Non-owner pods are thin: they register their local
//! clients as *remote peers* in the owner's room over a reliable
//! [`Profile::HuddleControl`] stream, forward those clients' Opus frames to the
//! owner as datagrams, and deliver the owner's fan-out back verbatim.
//!
//! ## The payload invariant (why this needs no wire change)
//!
//! The client sends `[8B v2 header][opaque Opus]`; the relay parses the header
//! for telemetry only and forwards the frame opaquely, and `broadcast_frame`
//! prepends a 1-byte `peer_index`. That `peer_index` is relay-added *routing*
//! metadata — it never touches ciphertext — so the whole byte string
//! `[peer_index][v2 header][Opus]` is exactly what [`MeshDatagram::payload`] is
//! for: opaque to encryption, owned by the routing plane. **peer_index is
//! always the first byte of a media datagram payload, both directions.** The
//! client's WebSocket wire format is byte-identical to a single-pod huddle.
//!
//! ## Room stays pure
//!
//! `Room` never learns about the mesh. A remote participant is an ordinary
//! [`AudioPeer`] whose `audio_tx` receiver is drained by a task that wraps each
//! frame in a [`MeshDatagram`] and calls [`RelayPeerTransport::send_datagram`].
//! The in-pod fan-out is reused unchanged; only a peer's *sink* differs.
//!
//! ## Fencing (law, not exempt for media)
//!
//! Every datagram carries a [`FencedHeader`]. Both ends reject frames whose
//! generation is stale for the session — a late datagram from a dead generation
//! is dropped, which for lossy audio is indistinguishable from packet loss and
//! is therefore exactly correct. Monotonicity of `generation` across owner death
//! is guaranteed by the directory's companion INCR counter (session-directory
//! lane); this module trusts that and only enforces "reject < known".

use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use buzz_relay_mesh::{FencedHeader, MeshDatagram, RelayPeerTransport, RuntimeId};

use super::room::AudioRoomManager;

/// The slice of the session directory that huddle audio needs.
///
/// Implemented by the session-directory lane (Perci) over the Redis fenced CAS
/// lease. Kept narrow on purpose: audio only asks "who owns this huddle, and at
/// what generation?" — it never acquires, renews, or releases leases (that is
/// the owning pod's session layer). Returning [`None`] means "no live owner"
/// (the caller may then acquire, on the owner path).
pub trait HuddleOwnerDirectory: Send + Sync + 'static {
    /// Current `{owner_runtime_id, generation}` for a huddle session, or `None`
    /// if no live lease exists. Cheap/cached; called on the join path, not per
    /// frame.
    fn owner_of(&self, session_id: Uuid) -> Option<Ownership>;
}

/// A resolved huddle ownership snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ownership {
    /// Boot-unique mesh endpoint key of the pod currently holding the lease.
    pub owner_runtime_id: RuntimeId,
    /// Fenced generation of this ownership epoch; monotonic per session.
    pub generation: u64,
}

/// Tracks the highest generation this pod has observed per session, so stale
/// frames are rejected at every hop (fencing law). Monotonic-only: a frame is
/// accepted iff its generation is `>=` the highest seen; observing a higher
/// generation advances the floor (and signals a takeover the caller may act on).
#[derive(Default)]
pub struct GenerationFloor {
    seen: dashmap::DashMap<Uuid, u64>,
}

impl GenerationFloor {
    /// Create an empty floor (no sessions observed yet).
    pub fn new() -> Self {
        Self {
            seen: dashmap::DashMap::new(),
        }
    }

    /// Check a frame's generation against the floor for its session.
    ///
    /// - `Accept` — generation is current (`== floor`) or advances it (`>
    ///   floor`, a takeover we now pin).
    /// - `RejectStale { known }` — generation is below the floor; drop the
    ///   frame. This is the fence.
    pub fn check(&self, session_id: Uuid, generation: u64) -> FenceVerdict {
        use dashmap::mapref::entry::Entry;
        match self.seen.entry(session_id) {
            Entry::Occupied(mut e) => {
                let known = *e.get();
                if generation < known {
                    FenceVerdict::RejectStale { known }
                } else {
                    if generation > known {
                        *e.get_mut() = generation;
                    }
                    FenceVerdict::Accept {
                        advanced: generation > known,
                    }
                }
            }
            Entry::Vacant(e) => {
                e.insert(generation);
                FenceVerdict::Accept { advanced: false }
            }
        }
    }

    /// Drop all state for a session (room ended / owner teardown).
    pub fn forget(&self, session_id: Uuid) {
        self.seen.remove(&session_id);
    }
}

/// Outcome of a fence check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceVerdict {
    /// Frame is live. `advanced` is true when it bumped the floor (takeover).
    Accept {
        /// True when this frame's generation exceeded the prior floor,
        /// signalling an ownership takeover the caller may act on.
        advanced: bool,
    },
    /// Frame is from a superseded generation; drop it.
    RejectStale {
        /// The highest generation observed for this session — the fence floor.
        known: u64,
    },
}

/// Handles inbound mesh media for huddles on this pod.
///
/// Registered as (part of) the relay's [`buzz_relay_mesh::InboundHandler`].
/// Datagrams are delivered to local room peers; the fence is enforced here so
/// no stale media reaches a client. The `HuddleControl` stream path (remote
/// peer registration) is driven from [`super::handler`] on join/leave and is
/// wired in a following change — this type owns the datagram half and the
/// shared fence state both halves consult.
pub struct MeshAudioRouter {
    rooms: Arc<AudioRoomManager>,
    fence: Arc<GenerationFloor>,
    local_runtime_id: RuntimeId,
}

impl MeshAudioRouter {
    /// Construct a router over this pod's rooms, tagged with the local runtime
    /// identity (used to distinguish owner vs non-owner delivery paths).
    pub fn new(rooms: Arc<AudioRoomManager>, local_runtime_id: RuntimeId) -> Self {
        Self::with_fence(rooms, local_runtime_id, Arc::new(GenerationFloor::new()))
    }

    /// Construct a router that enforces an externally owned generation floor.
    ///
    /// Used by the boot wiring (`mesh_boot::wire_mesh_consumers`) so the
    /// datagram hot path and session teardown (`GenerationFloor::forget`,
    /// reached via `MeshHandle::audio_fence`) consult exactly one floor.
    pub fn with_fence(
        rooms: Arc<AudioRoomManager>,
        local_runtime_id: RuntimeId,
        fence: Arc<GenerationFloor>,
    ) -> Self {
        Self {
            rooms,
            fence,
            local_runtime_id,
        }
    }

    /// Shared fence state, so the `HuddleControl` stream path (join/leave) and
    /// the datagram path enforce one generation floor per session.
    pub fn fence(&self) -> Arc<GenerationFloor> {
        Arc::clone(&self.fence)
    }

    /// This pod's mesh runtime identity.
    pub fn local_runtime_id(&self) -> RuntimeId {
        self.local_runtime_id
    }

    /// Deliver an inbound media datagram to the addressed local huddle.
    ///
    /// The payload is `[peer_index][v2 header][Opus]` — already prefixed by the
    /// sender (the owner, when fanning out to us; or a non-owner client's pod,
    /// when we are the owner). We fence, then push the payload into every
    /// *local* peer's audio sink **except** the peer whose index authored it,
    /// mirroring `broadcast_frame`'s "everyone but the sender" rule so a speaker
    /// never hears themselves.
    ///
    /// Returns the fence verdict for observability/tests. Does not itself
    /// re-fan across the mesh: if we are the owner, cross-pod fan-out happens
    /// through the remote peers' mesh sinks during `broadcast_frame`, so an
    /// owner-side inbound datagram only needs local delivery here.
    pub fn on_media_datagram(&self, dgram: &MeshDatagram) -> FenceVerdict {
        let session_id = dgram.fenced.session_id;
        let verdict = self.fence.check(session_id, dgram.fenced.generation);
        if let FenceVerdict::RejectStale { known } = verdict {
            debug!(
                %session_id,
                frame_generation = dgram.fenced.generation,
                known_generation = known,
                "dropping stale-generation media datagram (fence)"
            );
            return verdict;
        }

        let Some(room) = self.rooms.get_unambiguous_by_channel(session_id) else {
            // No local room for this session: nothing to deliver to. Not an
            // error — membership can race a datagram in flight. An ambiguous
            // same-UUID room collision is also dropped because the current
            // media envelope has no community label.
            return verdict;
        };

        let Some((&author_index, rest)) = dgram.payload.split_first() else {
            warn!(%session_id, "empty media datagram payload — dropping");
            return verdict;
        };
        // Reconstruct the exact on-wire frame the local fan-out uses:
        // [peer_index][v2 header][Opus]. `rest` is [v2 header][Opus]; the
        // prefix is the author's index. We hand peers the already-prefixed
        // bytes and skip re-broadcasting to the author's own index.
        let mut prefixed = bytes::BytesMut::with_capacity(dgram.payload.len());
        prefixed.extend_from_slice(&[author_index]);
        prefixed.extend_from_slice(rest);
        let prefixed = prefixed.freeze();

        room.deliver_prefixed(author_index, prefixed);
        verdict
    }
}

/// A sink that forwards a remote peer's fanned-out frames onto the mesh.
///
/// Constructed on the owner pod for each *remote* participant: the owner's
/// `Room` sees the remote peer as an ordinary [`AudioPeer`] whose `audio_tx`
/// feeds this task, which wraps each frame as a [`MeshDatagram`] and sends it to
/// the pod that hosts that participant. Drops on a disconnected/oversize peer —
/// realtime audio never blocks fan-out on one slow remote link.
pub fn spawn_remote_peer_sink(
    transport: Arc<dyn RelayPeerTransport>,
    to: RuntimeId,
    fenced: FencedHeader,
    mut frames: mpsc::Receiver<Bytes>,
) {
    tokio::spawn(async move {
        let mut seq: u64 = 0;
        while let Some(frame) = frames.recv().await {
            let dgram = MeshDatagram {
                fenced,
                seq,
                payload: frame.to_vec(),
            };
            seq = seq.wrapping_add(1);
            if let Err(e) = transport.send_datagram(to, dgram) {
                // Disconnected peer or oversize frame: drop and keep going.
                // The MTU case is the ship-gate's job to prevent; here we just
                // never let one bad link stall the room.
                debug!(%to, "remote peer datagram send failed: {e}");
            }
        }
        debug!(%to, "remote peer sink closed");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(b: u8) -> RuntimeId {
        RuntimeId([b; 32])
    }

    fn fenced(session: Uuid, generation: u64) -> FencedHeader {
        FencedHeader {
            session_id: session,
            generation,
            owner_runtime_id: rt(0xAA),
        }
    }

    #[test]
    fn fence_accepts_first_and_equal_and_higher() {
        let f = GenerationFloor::new();
        let s = Uuid::new_v4();
        assert_eq!(f.check(s, 5), FenceVerdict::Accept { advanced: false });
        assert_eq!(f.check(s, 5), FenceVerdict::Accept { advanced: false });
        assert_eq!(f.check(s, 6), FenceVerdict::Accept { advanced: true });
    }

    #[test]
    fn fence_rejects_stale_after_advance() {
        let f = GenerationFloor::new();
        let s = Uuid::new_v4();
        assert_eq!(f.check(s, 10), FenceVerdict::Accept { advanced: false });
        // A late frame from the superseded generation is rejected.
        assert_eq!(f.check(s, 9), FenceVerdict::RejectStale { known: 10 });
        // The floor is unchanged by a rejected frame.
        assert_eq!(f.check(s, 10), FenceVerdict::Accept { advanced: false });
    }

    #[test]
    fn fence_is_per_session() {
        let f = GenerationFloor::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert_eq!(f.check(a, 7), FenceVerdict::Accept { advanced: false });
        // A different session starts its own floor.
        assert_eq!(f.check(b, 1), FenceVerdict::Accept { advanced: false });
        assert_eq!(f.check(a, 6), FenceVerdict::RejectStale { known: 7 });
    }

    #[test]
    fn fence_forget_resets_floor() {
        let f = GenerationFloor::new();
        let s = Uuid::new_v4();
        f.check(s, 20);
        f.forget(s);
        // After forget, a lower generation is accepted as a fresh floor —
        // used on room-end/teardown so a rejoin isn't fenced by a dead session.
        assert_eq!(f.check(s, 3), FenceVerdict::Accept { advanced: false });
    }

    #[test]
    fn router_drops_stale_datagram_without_delivering() {
        let rooms = Arc::new(AudioRoomManager::new());
        let router = MeshAudioRouter::new(Arc::clone(&rooms), rt(1));
        let s = Uuid::new_v4();
        // Establish a floor at generation 5.
        assert!(matches!(
            router.on_media_datagram(&MeshDatagram {
                fenced: fenced(s, 5),
                seq: 0,
                payload: vec![0, 1, 2],
            }),
            FenceVerdict::Accept { .. }
        ));
        // A stale frame is rejected.
        assert_eq!(
            router.on_media_datagram(&MeshDatagram {
                fenced: fenced(s, 4),
                seq: 1,
                payload: vec![0, 1, 2],
            }),
            FenceVerdict::RejectStale { known: 5 }
        );
    }

    #[test]
    fn router_tolerates_missing_room_and_empty_payload() {
        let rooms = Arc::new(AudioRoomManager::new());
        let router = MeshAudioRouter::new(Arc::clone(&rooms), rt(1));
        let s = Uuid::new_v4();
        // No local room for this session: accepted by fence, no panic.
        assert!(matches!(
            router.on_media_datagram(&MeshDatagram {
                fenced: fenced(s, 1),
                seq: 0,
                payload: vec![7, 8],
            }),
            FenceVerdict::Accept { .. }
        ));
        // Empty payload after a valid fence: dropped, no panic.
        let s2 = Uuid::new_v4();
        assert!(matches!(
            router.on_media_datagram(&MeshDatagram {
                fenced: fenced(s2, 1),
                seq: 0,
                payload: vec![],
            }),
            FenceVerdict::Accept { .. }
        ));
    }
}
