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

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uuid::Uuid;

use buzz_relay_mesh::{FencedHeader, MeshDatagram, RelayPeerTransport, RuntimeId};

use super::room::AudioRoomManager;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct OwnerIngressKey {
    session_id: Uuid,
    generation: u64,
    sender: RuntimeId,
    peer_index: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct OwnerFanoutKey {
    session_id: Uuid,
    generation: u64,
    owner: RuntimeId,
}

struct OwnerIngressAttachment {
    admission_id: Uuid,
    expires_at: u64,
}

enum MediaAttachmentKind {
    OwnerIngress(OwnerIngressKey),
    OwnerFanout(OwnerFanoutKey),
}

/// Live, compensated media attachments established by the reliable huddle
/// control path. A datagram alone can never create an attachment or advance a
/// generation floor.
#[derive(Default)]
pub struct MediaAttachmentRegistry {
    owner_ingress: dashmap::DashMap<OwnerIngressKey, OwnerIngressAttachment>,
    owner_fanout: dashmap::DashMap<OwnerFanoutKey, HashMap<Uuid, u64>>,
}

/// Drop guard for one live media attachment.
pub(crate) struct MediaAttachmentGuard {
    registry: Arc<MediaAttachmentRegistry>,
    kind: MediaAttachmentKind,
    admission_id: Uuid,
}

impl Drop for MediaAttachmentGuard {
    fn drop(&mut self) {
        match self.kind {
            MediaAttachmentKind::OwnerIngress(key) => {
                if self
                    .registry
                    .owner_ingress
                    .get(&key)
                    .is_some_and(|entry| entry.admission_id == self.admission_id)
                {
                    self.registry.owner_ingress.remove(&key);
                }
            }
            MediaAttachmentKind::OwnerFanout(key) => {
                if let Some(mut admissions) = self.registry.owner_fanout.get_mut(&key) {
                    admissions.remove(&self.admission_id);
                    let empty = admissions.is_empty();
                    drop(admissions);
                    if empty {
                        self.registry
                            .owner_fanout
                            .remove_if(&key, |_, value| value.is_empty());
                    }
                }
            }
        }
    }
}

impl MediaAttachmentRegistry {
    /// Register one protected non-owner participant as an authorized media
    /// author on the owner pod. The owner-assigned index and authenticated
    /// runtime are both part of the key.
    pub(crate) fn register_owner_ingress(
        self: &Arc<Self>,
        fenced: FencedHeader,
        sender: RuntimeId,
        peer_index: u8,
        admission_id: Uuid,
        expires_at: u64,
    ) -> Option<MediaAttachmentGuard> {
        use dashmap::mapref::entry::Entry;

        let key = OwnerIngressKey {
            session_id: fenced.session_id,
            generation: fenced.generation,
            sender,
            peer_index,
        };
        match self.owner_ingress.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(OwnerIngressAttachment {
                    admission_id,
                    expires_at,
                });
            }
            Entry::Occupied(entry) if entry.get().admission_id == admission_id => {}
            Entry::Occupied(_) => return None,
        }
        Some(MediaAttachmentGuard {
            registry: Arc::clone(self),
            kind: MediaAttachmentKind::OwnerIngress(key),
            admission_id,
        })
    }

    /// Register the owner as the only accepted fan-out source for one local
    /// protected participant on a non-owner pod.
    pub(crate) fn register_owner_fanout(
        self: &Arc<Self>,
        fenced: FencedHeader,
        admission_id: Uuid,
        expires_at: u64,
    ) -> MediaAttachmentGuard {
        let key = OwnerFanoutKey {
            session_id: fenced.session_id,
            generation: fenced.generation,
            owner: fenced.owner_runtime_id,
        };
        self.owner_fanout
            .entry(key)
            .or_default()
            .insert(admission_id, expires_at);
        MediaAttachmentGuard {
            registry: Arc::clone(self),
            kind: MediaAttachmentKind::OwnerFanout(key),
            admission_id,
        }
    }

    fn authorizes_owner_ingress(&self, key: OwnerIngressKey) -> Option<Uuid> {
        let (admission_id, expires_at) = self
            .owner_ingress
            .get(&key)
            .map(|entry| (entry.admission_id, entry.expires_at))
            .unwrap_or((Uuid::nil(), 0));
        let current = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs());
        if current.is_none_or(|current| current >= expires_at) {
            self.owner_ingress
                .remove_if(&key, |_, entry| entry.admission_id == admission_id);
            return None;
        }
        self.owner_ingress
            .get(&key)
            .filter(|entry| entry.admission_id == admission_id)
            .map(|entry| entry.admission_id)
    }

    fn authorized_owner_fanout_admissions(
        &self,
        key: OwnerFanoutKey,
    ) -> std::collections::HashSet<Uuid> {
        let current = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs());
        let Some(mut admissions) = self.owner_fanout.get_mut(&key) else {
            return std::collections::HashSet::new();
        };
        admissions.retain(|_, expires_at| current.is_some_and(|current| current < *expires_at));
        admissions.keys().copied().collect()
    }
}

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
    attachments: Arc<MediaAttachmentRegistry>,
}

impl MeshAudioRouter {
    /// Construct a router over this pod's rooms, tagged with the local runtime
    /// identity (used to distinguish owner vs non-owner delivery paths).
    pub fn new(rooms: Arc<AudioRoomManager>, local_runtime_id: RuntimeId) -> Self {
        Self::with_fence(
            rooms,
            local_runtime_id,
            Arc::new(GenerationFloor::new()),
            Arc::new(MediaAttachmentRegistry::default()),
        )
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
        attachments: Arc<MediaAttachmentRegistry>,
    ) -> Self {
        Self {
            rooms,
            fence,
            local_runtime_id,
            attachments,
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
    pub fn on_media_datagram(&self, from: RuntimeId, dgram: &MeshDatagram) -> Option<FenceVerdict> {
        let session_id = dgram.fenced.session_id;
        let Some((&author_index, rest)) = dgram.payload.split_first() else {
            warn!(%session_id, "empty media datagram payload — dropping");
            return None;
        };

        let owner_ingress = dgram.fenced.owner_runtime_id == self.local_runtime_id;
        let mut owner_ingress_admission = None;
        let authorized_fanout = if owner_ingress {
            let Some(admission_id) = self.attachments.authorizes_owner_ingress(OwnerIngressKey {
                session_id,
                generation: dgram.fenced.generation,
                sender: from,
                peer_index: author_index,
            }) else {
                debug!(%session_id, %from, "dropping media without a live control attachment");
                return None;
            };
            owner_ingress_admission = Some(admission_id);
            None
        } else {
            if from != dgram.fenced.owner_runtime_id {
                debug!(%session_id, %from, "dropping media from a non-owner runtime");
                return None;
            }
            let admissions = self
                .attachments
                .authorized_owner_fanout_admissions(OwnerFanoutKey {
                    session_id,
                    generation: dgram.fenced.generation,
                    owner: from,
                });
            if admissions.is_empty() {
                debug!(%session_id, %from, "dropping media without a live control attachment");
                return None;
            }
            Some(admissions)
        };

        let room = self.rooms.get_unambiguous_by_channel(session_id);
        if let Some(admission_id) = owner_ingress_admission {
            let room = room.as_ref()?;
            if !room.is_published_admission(admission_id, author_index) {
                debug!(%session_id, %from, "dropping media from an unpublished admission");
                return None;
            }
        }

        let verdict = self.fence.check(session_id, dgram.fenced.generation);
        if let FenceVerdict::RejectStale { known } = verdict {
            debug!(
                %session_id,
                frame_generation = dgram.fenced.generation,
                known_generation = known,
                "dropping stale-generation media datagram (fence)"
            );
            return Some(verdict);
        }

        let Some(room) = room else {
            // Fan-out may race local room construction. The authenticated
            // owner generation still advances monotonically, but no output is
            // disclosed. Owner-ingress took the stricter path above.
            return Some(verdict);
        };

        // Reconstruct the exact on-wire frame the local fan-out uses:
        // [peer_index][v2 header][Opus]. `rest` is [v2 header][Opus]; the
        // prefix is the author's index. We hand peers the already-prefixed
        // bytes and skip re-broadcasting to the author's own index.
        let mut prefixed = bytes::BytesMut::with_capacity(dgram.payload.len());
        prefixed.extend_from_slice(&[author_index]);
        prefixed.extend_from_slice(rest);
        let prefixed = prefixed.freeze();

        match authorized_fanout {
            Some(admissions) => {
                room.deliver_prefixed_to_admissions(author_index, prefixed, &admissions)
            }
            None => room.deliver_prefixed(author_index, prefixed),
        }
        Some(verdict)
    }
}

/// A sink that forwards a remote peer's fanned-out frames onto the mesh.
///
/// Constructed on the owner pod for each *remote* participant: the owner's
/// `Room` sees the remote peer as an ordinary [`AudioPeer`] whose `audio_tx`
/// feeds this task, which wraps each frame as a [`MeshDatagram`] and sends it to
/// the pod that hosts that participant. Drops on a disconnected/oversize peer —
/// realtime audio never blocks fan-out on one slow remote link.
pub(crate) struct RemotePeerSinkGuard {
    cancel: CancellationToken,
    active: Arc<std::sync::Mutex<bool>>,
}

impl RemotePeerSinkGuard {
    /// Stop the sink without draining frames already queued for this peer.
    ///
    /// The mutex makes `close` the local authoritative boundary: once it
    /// returns, no later transport send can begin. A send already holding the
    /// mutex completes before `close` returns.
    pub(crate) fn close(&self) {
        self.cancel.cancel();
        if let Ok(mut active) = self.active.lock() {
            *active = false;
        }
    }
}

impl Drop for RemotePeerSinkGuard {
    fn drop(&mut self) {
        self.close();
    }
}

/// One-shot start capability for a prepared remote sink. Protected callers
/// install the guard in their exact expiry effect set before consuming this.
pub(crate) struct RemotePeerSinkStart {
    start: Option<oneshot::Sender<()>>,
    cancel: CancellationToken,
    wake_at: Option<tokio::time::Instant>,
}

impl RemotePeerSinkStart {
    pub(crate) fn start(mut self) -> bool {
        !self.cancel.is_cancelled()
            && self
                .wake_at
                .is_none_or(|wake_at| tokio::time::Instant::now() < wake_at)
            && self
                .start
                .take()
                .is_some_and(|start| start.send(()).is_ok())
    }
}

pub(crate) fn prepare_remote_peer_sink(
    transport: Arc<dyn RelayPeerTransport>,
    to: RuntimeId,
    fenced: FencedHeader,
    mut frames: mpsc::Receiver<Bytes>,
    wake_at: Option<tokio::time::Instant>,
) -> (RemotePeerSinkGuard, RemotePeerSinkStart) {
    let cancel = CancellationToken::new();
    let active = Arc::new(std::sync::Mutex::new(true));
    let task_cancel = cancel.clone();
    let task_active = Arc::clone(&active);
    let (start_tx, start_rx) = oneshot::channel();
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = task_cancel.cancelled() => return,
            started = start_rx => if started.is_err() { return },
        }
        let mut seq: u64 = 0;
        loop {
            let frame = tokio::select! {
                biased;
                _ = task_cancel.cancelled() => break,
                frame = frames.recv() => {
                    let Some(frame) = frame else { break };
                    frame
                }
            };
            if wake_at.is_some_and(|wake_at| tokio::time::Instant::now() >= wake_at) {
                task_cancel.cancel();
                break;
            }
            let dgram = MeshDatagram {
                fenced,
                seq,
                payload: frame.to_vec(),
            };
            seq = seq.wrapping_add(1);
            let Ok(active) = task_active.lock() else {
                break;
            };
            if !*active || task_cancel.is_cancelled() {
                break;
            }
            if let Err(e) = transport.send_datagram(to, dgram) {
                debug!(%to, "remote peer datagram send failed: {e}");
            }
        }
        debug!(%to, "remote peer sink closed");
    });
    (
        RemotePeerSinkGuard {
            cancel: cancel.clone(),
            active,
        },
        RemotePeerSinkStart {
            start: Some(start_tx),
            cancel,
            wake_at,
        },
    )
}

pub(crate) fn spawn_remote_peer_sink(
    transport: Arc<dyn RelayPeerTransport>,
    to: RuntimeId,
    fenced: FencedHeader,
    frames: mpsc::Receiver<Bytes>,
) -> RemotePeerSinkGuard {
    let (guard, start) = prepare_remote_peer_sink(transport, to, fenced, frames, None);
    let _ = start.start();
    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingTransport {
        sent: std::sync::Mutex<Vec<MeshDatagram>>,
    }

    impl RelayPeerTransport for RecordingTransport {
        fn send_datagram(
            &self,
            _to: RuntimeId,
            dgram: MeshDatagram,
        ) -> Result<(), buzz_relay_mesh::MeshError> {
            self.sent.lock().expect("recording lock").push(dgram);
            Ok(())
        }

        fn open_session_stream(
            &self,
            _to: RuntimeId,
            _hello: buzz_relay_mesh::StreamHello,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<buzz_relay_mesh::MeshStream, buzz_relay_mesh::MeshError>,
        > {
            Box::pin(async { Err(buzz_relay_mesh::MeshError::Transport("unused".into())) })
        }

        fn set_inbound(&self, _handler: Box<dyn buzz_relay_mesh::InboundHandler>) {}
    }

    fn rt(b: u8) -> RuntimeId {
        RuntimeId([b; 32])
    }

    fn community() -> buzz_core::CommunityId {
        buzz_core::CommunityId::from_uuid(Uuid::from_u128(1))
    }

    fn fenced(session: Uuid, generation: u64) -> FencedHeader {
        FencedHeader {
            session_id: session,
            generation,
            owner_runtime_id: rt(0xAA),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn revoked_remote_sink_discards_buffered_fanout_before_transport_send() {
        let recording = Arc::new(RecordingTransport::default());
        let transport: Arc<dyn RelayPeerTransport> = recording.clone();
        let (tx, rx) = mpsc::channel(8);
        let guard = spawn_remote_peer_sink(transport, rt(2), fenced(Uuid::new_v4(), 1), rx);

        tx.try_send(Bytes::from_static(b"already-authorized"))
            .expect("initial frame queues");
        while recording.sent.lock().expect("recording lock").is_empty() {
            tokio::task::yield_now().await;
        }

        for _ in 0..8 {
            tx.try_send(Bytes::from_static(b"must-be-discarded"))
                .expect("revocation backlog queues");
        }
        guard.close();
        tokio::task::yield_now().await;

        assert_eq!(
            recording.sent.lock().expect("recording lock").len(),
            1,
            "closing a revoked sink must discard its buffered fan-out"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn prepared_remote_sink_cannot_emit_before_exact_effect_installation() {
        let recording = Arc::new(RecordingTransport::default());
        let transport: Arc<dyn RelayPeerTransport> = recording.clone();
        let (tx, rx) = mpsc::channel(1);
        let (guard, start) =
            prepare_remote_peer_sink(transport, rt(2), fenced(Uuid::new_v4(), 1), rx, None);
        tx.try_send(Bytes::from_static(b"not-yet-authorized"))
            .expect("frame queues");
        tokio::task::yield_now().await;
        assert!(recording.sent.lock().expect("recording lock").is_empty());

        guard.close();
        assert!(!start.start(), "closed prepared sink cannot be started");
        tokio::task::yield_now().await;
        assert!(recording.sent.lock().expect("recording lock").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn prepared_remote_sink_cannot_start_at_exact_authority_deadline() {
        let recording = Arc::new(RecordingTransport::default());
        let transport: Arc<dyn RelayPeerTransport> = recording.clone();
        let (tx, rx) = mpsc::channel(1);
        let wake_at = tokio::time::Instant::now() + std::time::Duration::from_millis(250);
        let (_guard, start) = prepare_remote_peer_sink(
            transport,
            rt(2),
            fenced(Uuid::new_v4(), 1),
            rx,
            Some(wake_at),
        );
        tx.try_send(Bytes::from_static(b"must-not-cross-deadline"))
            .expect("frame queues while hidden");

        tokio::time::advance(std::time::Duration::from_millis(250)).await;
        assert!(!start.start(), "deadline equality fails closed");
        tokio::task::yield_now().await;
        assert!(recording.sent.lock().expect("recording lock").is_empty());
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

    fn test_router(
        rooms: Arc<AudioRoomManager>,
        local_runtime_id: RuntimeId,
    ) -> (MeshAudioRouter, Arc<MediaAttachmentRegistry>) {
        let attachments = Arc::new(MediaAttachmentRegistry::default());
        (
            MeshAudioRouter::with_fence(
                rooms,
                local_runtime_id,
                Arc::new(GenerationFloor::new()),
                Arc::clone(&attachments),
            ),
            attachments,
        )
    }

    #[tokio::test]
    async fn router_drops_stale_datagram_without_delivering() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, attachments) = test_router(Arc::clone(&rooms), rt(1));
        let s = Uuid::new_v4();
        let fence = fenced(s, 5);
        let _attached = attachments.register_owner_fanout(fence, Uuid::new_v4(), u64::MAX);
        // Establish a floor at generation 5.
        assert!(matches!(
            router.on_media_datagram(
                rt(0xAA),
                &MeshDatagram {
                    fenced: fence,
                    seq: 0,
                    payload: vec![0, 1, 2],
                },
            ),
            Some(FenceVerdict::Accept { .. })
        ));
        // A stale frame is rejected.
        let stale = fenced(s, 4);
        let _stale_attached = attachments.register_owner_fanout(stale, Uuid::new_v4(), u64::MAX);
        assert_eq!(
            router.on_media_datagram(
                rt(0xAA),
                &MeshDatagram {
                    fenced: stale,
                    seq: 1,
                    payload: vec![0, 1, 2],
                },
            ),
            Some(FenceVerdict::RejectStale { known: 5 })
        );
    }

    #[tokio::test]
    async fn router_tolerates_missing_room_and_empty_payload() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, attachments) = test_router(Arc::clone(&rooms), rt(1));
        let s = Uuid::new_v4();
        let fence = fenced(s, 1);
        let _attached = attachments.register_owner_fanout(fence, Uuid::new_v4(), u64::MAX);
        // No local room for this session: accepted by fence, no panic.
        assert!(matches!(
            router.on_media_datagram(
                rt(0xAA),
                &MeshDatagram {
                    fenced: fence,
                    seq: 0,
                    payload: vec![7, 8],
                },
            ),
            Some(FenceVerdict::Accept { .. })
        ));
        // Empty payload is dropped before it can alter the fence.
        let s2 = Uuid::new_v4();
        assert_eq!(
            router.on_media_datagram(
                rt(0xAA),
                &MeshDatagram {
                    fenced: fenced(s2, 1),
                    seq: 0,
                    payload: vec![],
                },
            ),
            None
        );
    }

    #[tokio::test]
    async fn realtime_media_requires_registered_control_attachment() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, _) = test_router(rooms, rt(1));
        let session = Uuid::new_v4();
        assert_eq!(
            router.on_media_datagram(
                rt(0xAA),
                &MeshDatagram {
                    fenced: fenced(session, 99),
                    seq: 0,
                    payload: vec![3, 1, 2],
                },
            ),
            None
        );
        assert_eq!(
            router.fence().check(session, 1),
            FenceVerdict::Accept { advanced: false }
        );
    }

    #[tokio::test]
    async fn realtime_media_rejects_non_owner_sender_on_ingress() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, attachments) = test_router(rooms, rt(1));
        let fence = fenced(Uuid::new_v4(), 2);
        let _attached = attachments.register_owner_fanout(fence, Uuid::new_v4(), u64::MAX);
        assert_eq!(
            router.on_media_datagram(
                rt(0xBB),
                &MeshDatagram {
                    fenced: fence,
                    seq: 0,
                    payload: vec![4, 1],
                },
            ),
            None
        );
    }

    #[tokio::test]
    async fn late_media_after_abort_is_dropped() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, attachments) = test_router(rooms, rt(1));
        let fence = fenced(Uuid::new_v4(), 3);
        let attached = attachments.register_owner_fanout(fence, Uuid::new_v4(), u64::MAX);
        drop(attached);
        assert_eq!(
            router.on_media_datagram(
                rt(0xAA),
                &MeshDatagram {
                    fenced: fence,
                    seq: 1,
                    payload: vec![5, 1],
                },
            ),
            None
        );
    }

    #[tokio::test]
    async fn expired_local_remote_peer_cannot_receive_through_live_sibling() {
        let rooms = Arc::new(AudioRoomManager::new());
        let local_runtime = rt(1);
        let (router, attachments) = test_router(Arc::clone(&rooms), local_runtime);
        let session = Uuid::new_v4();
        let room = rooms.get_or_create(community(), session);
        let expired_admission = Uuid::new_v4();
        let live_admission = Uuid::new_v4();
        let expired = room
            .reserve_peer(expired_admission, "expired".into(), 2)
            .expect("reserve expired peer")
            .activate()
            .expect("activate expired peer");
        let live = room
            .reserve_peer(live_admission, "live".into(), 2)
            .expect("reserve live peer")
            .activate()
            .expect("activate live peer");
        let fence = fenced(session, 4);
        let expired_attachment =
            attachments.register_owner_fanout(fence, expired_admission, u64::MAX);
        let _live_attachment = attachments.register_owner_fanout(fence, live_admission, u64::MAX);
        drop(expired_attachment);

        assert!(matches!(
            router.on_media_datagram(
                fence.owner_runtime_id,
                &MeshDatagram {
                    fenced: fence,
                    seq: 0,
                    payload: vec![99, 1, 2, 3],
                },
            ),
            Some(FenceVerdict::Accept { .. })
        ));
        let mut expired_rx = expired.2;
        let mut live_rx = live.2;
        assert!(expired_rx.try_recv().is_err());
        assert_eq!(
            live_rx.try_recv().expect("live sibling receives").as_ref(),
            &[99, 1, 2, 3]
        );
    }

    #[test]
    fn expired_owner_ingress_cannot_deliver_or_advance_the_fence() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, attachments) = test_router(rooms, rt(0xAA));
        let fence = fenced(Uuid::new_v4(), 7);
        let sender = rt(1);
        let _attached = attachments
            .register_owner_ingress(fence, sender, 9, Uuid::new_v4(), 0)
            .expect("unique attachment");
        assert_eq!(
            router.on_media_datagram(
                sender,
                &MeshDatagram {
                    fenced: fence,
                    seq: 0,
                    payload: vec![9, 1, 2],
                },
            ),
            None
        );
        assert_eq!(
            router.fence().check(fence.session_id, 1),
            FenceVerdict::Accept { advanced: false }
        );
    }

    #[test]
    fn expired_owner_fanout_cannot_deliver_or_advance_the_fence() {
        let rooms = Arc::new(AudioRoomManager::new());
        let (router, attachments) = test_router(rooms, rt(1));
        let fence = fenced(Uuid::new_v4(), 7);
        let _attached = attachments.register_owner_fanout(fence, Uuid::new_v4(), 0);
        assert_eq!(
            router.on_media_datagram(
                fence.owner_runtime_id,
                &MeshDatagram {
                    fenced: fence,
                    seq: 0,
                    payload: vec![9, 1, 2],
                },
            ),
            None
        );
        assert_eq!(
            router.fence().check(fence.session_id, 1),
            FenceVerdict::Accept { advanced: false }
        );
    }
}
