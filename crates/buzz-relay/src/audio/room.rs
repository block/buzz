//! Audio room: peer registry and frame fan-out.
//!
//! ```text
//! Client A → WS binary frame → Room::broadcast_frame → Client B, C, ...
//!                                                        (1-byte peer_index prefix)
//! ```
//!
//! Frames are opaque Opus bytes — the relay never decodes audio.
//! `try_send` is used throughout: real-time audio tolerates drops, never queues.

use buzz_core::CommunityId;
use buzz_relay_mesh::RuntimeId;
use bytes::Bytes;
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A connected audio peer.
pub struct AudioPeer {
    /// Nostr pubkey hex.
    pub pubkey: String,
    /// Audio frames (binary Opus with peer_index prefix). Drops on full — real-time.
    pub audio_tx: mpsc::Sender<Bytes>,
    /// Control messages (joined/left/close JSON). Separate queue so control
    /// is never starved by audio backpressure.
    pub ctrl_tx: mpsc::Sender<PeerCtrl>,
    /// Stable 0-254 index assigned at join; prefixed onto relayed frames.
    pub peer_index: u8,
    /// Durable protected admission attempt, when this peer has one.
    admission_id: Option<Uuid>,
    /// Absolute protected-authority deadline. Legacy peers have no deadline.
    authority_expires_at: Option<u64>,
    /// Exact monotonic instant at which this protected activation expires.
    authority_wake_at: Option<tokio::time::Instant>,
    /// Room-local generation for this exact protected activation.
    authority_generation: Option<u64>,
    /// Effects revoked synchronously before protected roster withdrawal.
    protected_effects: Option<ProtectedPeerEffects>,
    /// Whether this exact protected activation published its client join.
    protected_join_published: bool,
    /// Owner-side mesh destination. Peers on the same remote runtime share
    /// one fan-out copy; the destination pod performs per-admission delivery.
    fanout_group: Option<[u8; 32]>,
}

impl AudioPeer {
    fn protected_schedule(&self) -> Option<ProtectedDeadlineSchedule> {
        Some(ProtectedDeadlineSchedule {
            deadline: self.authority_expires_at?,
            wake_at: self.authority_wake_at?,
        })
    }

    fn authority_is_current(&self) -> bool {
        if self.authority_expires_at.is_none()
            && self.authority_wake_at.is_none()
            && self.protected_effects.is_none()
        {
            return true;
        }
        self.protected_schedule()
            .is_some_and(ProtectedDeadlineSchedule::is_current)
            && self
                .protected_effects
                .as_ref()
                .is_some_and(ProtectedPeerEffects::is_live)
    }

    fn is_visible(&self) -> bool {
        self.protected_join_published && self.authority_is_current()
    }

    fn matches_epoch(&self, epoch: ProtectedPeerEpoch) -> bool {
        self.admission_id == Some(epoch.admission_id)
            && self.authority_generation == Some(epoch.generation)
            && self.authority_expires_at == Some(epoch.deadline)
            && self.authority_wake_at == Some(epoch.wake_at)
    }
}

/// Control message for a single peer (separate from audio frames).
pub enum PeerCtrl {
    /// JSON control message (joined/left/speakers).
    Json(String),
    /// Graceful shutdown signal.
    Close,
}

type EffectRevoker = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct ProtectedPeerEffectsState {
    closed: bool,
    revokers: Vec<EffectRevoker>,
}

/// Close-aware effects owned by one protected audio admission.
#[derive(Clone)]
pub(crate) struct ProtectedPeerEffects {
    state: Arc<std::sync::Mutex<ProtectedPeerEffectsState>>,
    cancel: CancellationToken,
}

impl ProtectedPeerEffects {
    pub(crate) fn new(cancel: CancellationToken) -> Self {
        Self {
            state: Arc::new(std::sync::Mutex::new(ProtectedPeerEffectsState::default())),
            cancel,
        }
    }

    /// Install an exact effect revoker. If expiry already won, execute it now.
    pub(crate) fn install_revoker<F>(&self, revoker: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        let mut revoker = Some(Box::new(revoker) as EffectRevoker);
        let installed = match self.state.lock() {
            Ok(mut state) if !state.closed => {
                state
                    .revokers
                    .push(revoker.take().expect("revoker present"));
                true
            }
            _ => false,
        };
        if let Some(revoker) = revoker {
            revoker();
        }
        installed
    }

    pub(crate) fn revoke(&self) {
        let revokers = match self.state.lock() {
            Ok(mut state) => {
                if state.closed {
                    return;
                }
                state.closed = true;
                std::mem::take(&mut state.revokers)
            }
            Err(_) => {
                self.cancel.cancel();
                return;
            }
        };
        for revoker in revokers {
            revoker();
        }
        self.cancel.cancel();
    }

    pub(crate) fn is_live(&self) -> bool {
        !self.cancel.is_cancelled() && self.state.lock().is_ok_and(|state| !state.closed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProtectedPeerEpoch {
    community_id: CommunityId,
    channel_id: Uuid,
    peer_id: Uuid,
    admission_id: Uuid,
    generation: u64,
    deadline: u64,
    wake_at: tokio::time::Instant,
}

struct ActivationCommit {
    committed: bool,
    protected_epoch: Option<ProtectedPeerEpoch>,
}

/// One immutable, conservative schedule for a protected admission deadline.
///
/// Authority expiries are encoded as whole Unix seconds and denote the start
/// of that second. The monotonic wake instant is therefore derived from a
/// high-resolution wall-clock sample and is never rounded up.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProtectedDeadlineSchedule {
    deadline: u64,
    wake_at: tokio::time::Instant,
}

/// Exact mesh owner incarnation authorized to use one protected room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoomOwnerEpoch {
    pub(crate) owner_runtime_id: RuntimeId,
    pub(crate) generation: u64,
}

impl RoomOwnerEpoch {
    pub(crate) const fn new(owner_runtime_id: RuntimeId, generation: u64) -> Self {
        Self {
            owner_runtime_id,
            generation,
        }
    }
}

impl ProtectedDeadlineSchedule {
    /// Build a production schedule. `coarse_delay` comes from an injected
    /// whole-second authorization clock; subtracting one second makes that
    /// input conservative before combining it with the high-resolution wall
    /// clock. Either source may move closure earlier, never later.
    pub(crate) fn new(
        deadline: u64,
        coarse_delay: Option<std::time::Duration>,
    ) -> Result<Self, AdmissionError> {
        let monotonic_now = tokio::time::Instant::now();
        Self::new_anchored(deadline, monotonic_now, coarse_delay)
    }

    /// Finish a deadline capture after the caller sampled the monotonic
    /// anchor and then consulted an injected/coarse authority clock.
    ///
    /// Sampling the anchor first is essential: time spent consulting the
    /// injected clock must consume authority rather than extend it.
    pub(crate) fn new_anchored(
        deadline: u64,
        monotonic_now: tokio::time::Instant,
        coarse_delay: Option<std::time::Duration>,
    ) -> Result<Self, AdmissionError> {
        let wall_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| AdmissionError::Ended)?;
        Self::from_samples(deadline, monotonic_now, wall_now, coarse_delay)
            .ok_or(AdmissionError::Ended)
    }

    fn from_samples(
        deadline: u64,
        monotonic_now: tokio::time::Instant,
        wall_now: std::time::Duration,
        coarse_delay: Option<std::time::Duration>,
    ) -> Option<Self> {
        let wall_remaining = std::time::Duration::from_secs(deadline).checked_sub(wall_now)?;
        if wall_remaining.is_zero() {
            return None;
        }
        let remaining = coarse_delay.map_or(wall_remaining, |delay| {
            wall_remaining.min(delay.saturating_sub(std::time::Duration::from_secs(1)))
        });
        Some(Self {
            deadline,
            wake_at: monotonic_now.checked_add(remaining)?,
        })
    }

    pub(crate) fn deadline(self) -> u64 {
        self.deadline
    }

    pub(crate) fn wake_at(self) -> tokio::time::Instant {
        self.wake_at
    }

    fn is_current(self) -> bool {
        let monotonic_now = tokio::time::Instant::now();
        if monotonic_now >= self.wake_at {
            return false;
        }
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .is_ok_and(|wall_now| wall_now < std::time::Duration::from_secs(self.deadline))
    }
}

/// A successfully activated peer and its private audio/control receivers.
pub type ActivatedAudioPeer = (Uuid, u8, mpsc::Receiver<Bytes>, mpsc::Receiver<PeerCtrl>);

/// Audio channel capacity per peer: 8 frames = 160ms at 20ms/frame.
const AUDIO_CHANNEL_CAPACITY: usize = 8;
/// Control channel capacity per peer: 32 slots — must never drop joined/left
/// messages, which are state-bearing (they maintain the client's peer_index →
/// pubkey map). Sized generously: even 30 simultaneous join/leave events fit.
const CTRL_CHANNEL_CAPACITY: usize = 32;

/// Defense-in-depth cap on peers per room. A room with N peers generates
/// N×(N−1) frame copies per 20ms tick — 25 peers = 600 copies/tick, which
/// is reasonable. The 255 index space is the hard limit; this is the soft one.
const MAX_PEERS_PER_ROOM: usize = 25;

/// One authoritative owner-roster entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterPeer {
    /// Nostr pubkey hex.
    pub pubkey: String,
    /// Owner-assigned media routing index.
    pub peer_index: u8,
}

/// A complete owner-roster snapshot at one monotonic revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterSnapshot {
    /// Owner-monotonic roster revision.
    pub revision: u64,
    /// Complete participants at this revision.
    pub peers: Vec<RosterPeer>,
}

/// One ordered owner-roster mutation. Receivers that miss a revision must
/// replace local state from a fresh [`RosterSnapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterDelta {
    /// Owner-monotonic roster revision.
    pub revision: u64,
    /// Newly admitted peer, when this is a join.
    pub joined: Option<RosterPeer>,
    /// Removed peer, when this is a leave.
    pub left: Option<RosterPeer>,
}

/// Reason a peer was refused entry to a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    /// The room has been ended (or is shutting down) and no longer admits peers.
    Ended,
    /// The room has hit the soft peer cap or exhausted the 255-index space.
    Full,
    /// The room is pinned to a different protocol version than the requested one.
    /// The caller should reply to the WS client with an `upgrade_required` error
    /// and the room's actual `pinned` version, then close the socket.
    VersionMismatch {
        /// Version the room is currently pinned to.
        pinned: u8,
        /// Version the joining client requested.
        requested: u8,
    },
}

/// Peer index allocator + room lifecycle gate.
///
/// The `ended` flag and peer admission are synchronized under the same mutex.
/// `add_peer` holds this lock across the ended check, index allocation, and
/// peer insert — so `mark_ended` (which also acquires this lock) is mutually
/// exclusive with peer admission. This closes the race between the last
/// peer's cleanup path and a concurrent joiner.
struct AdmissionGuard {
    next_fresh: u8,
    free: Vec<u8>,
    ended: bool,
    /// Pinned huddle audio protocol version for this room.
    ///
    /// `None` until the first peer admits. The first admission pins the room
    /// to that version; subsequent peers MUST present the same version or
    /// they're rejected with `AdmissionError::VersionMismatch`. The relay
    /// forwards binary frames opaquely either way, so allowing a v1 client
    /// into a v2 room would silently corrupt v2 peers' decode (they'd see
    /// no header where one is expected, and vice versa).
    ///
    /// Pin is per-`Room`-instance and clears when the manager evicts the
    /// Room via [`AudioRoomManager::cleanup_if_empty`] — the next
    /// `get_or_create` for the same community-local channel then constructs a
    /// fresh `Room` with a fresh `AdmissionGuard` (and therefore `None` pin),
    /// so a new generation of joiners can negotiate a new version.
    /// A momentarily-empty-but-not-yet-cleaned-up Room keeps its pin so
    /// reconnecting peers don't accidentally renegotiate mid-call. See
    /// `version_pin_persists_across_peer_churn` for the test that pins
    /// this behavior.
    pinned_version: Option<u8>,
    roster_revision: u64,
    /// Non-visible reservations keyed by the durable admission attempt id.
    pending: HashMap<Uuid, PendingPeerRecord>,
    /// Activated protected attempts. A retry cannot create a second presence.
    active: HashMap<Uuid, Uuid>,
    /// Monotonic room-local generation for protected activations.
    next_authority_generation: u64,
    /// Immutable mesh-owner incarnation for protected use of this room.
    owner_epoch: Option<RoomOwnerEpoch>,
    /// Unique pre-reservation claims. A claim spans every asynchronous step
    /// between selecting an owner generation and creating a pending peer.
    owner_claims: HashSet<Uuid>,
}

struct PendingPeerRecord {
    peer_id: Uuid,
    pubkey: String,
    peer_index: u8,
    audio_tx: mpsc::Sender<Bytes>,
    ctrl_tx: mpsc::Sender<PeerCtrl>,
    fanout_group: Option<[u8; 32]>,
}

/// A non-visible audio attachment reservation.
///
/// Dropping this value before activation idempotently releases its room index.
pub struct PendingAudioPeer {
    room: std::sync::Weak<Room>,
    admission_id: Uuid,
    peer_id: Uuid,
    peer_index: u8,
    audio_rx: Option<mpsc::Receiver<Bytes>>,
    ctrl_rx: Option<mpsc::Receiver<PeerCtrl>>,
    activated: bool,
}

impl PendingAudioPeer {
    /// Owner-assigned peer index reserved for this attempt.
    pub fn peer_index(&self) -> u8 {
        self.peer_index
    }

    /// Atomically make the reserved peer visible and emit its first roster delta.
    pub fn activate(self) -> Result<ActivatedAudioPeer, AdmissionError> {
        self.activate_if(|| true)?.ok_or(AdmissionError::Ended)
    }

    /// Make the reservation visible only if the synchronous commit predicate
    /// is still true while the room admission lock is held.
    ///
    /// Protected callers use this for the absolute lease deadline and owner
    /// epoch after their final asynchronous authority revalidation. A false
    /// predicate leaves the reservation pending; `Drop` then releases it
    /// without ever inserting a peer or publishing a roster delta.
    pub fn activate_if<F>(
        mut self,
        predicate: F,
    ) -> Result<Option<ActivatedAudioPeer>, AdmissionError>
    where
        F: FnOnce() -> bool,
    {
        let room = self.room.upgrade().ok_or(AdmissionError::Ended)?;
        if !room
            .activate_pending_if(self.admission_id, self.peer_id, None, None, predicate)?
            .committed
        {
            return Ok(None);
        }
        self.activated = true;
        Ok(Some((
            self.peer_id,
            self.peer_index,
            self.audio_rx.take().ok_or(AdmissionError::Ended)?,
            self.ctrl_rx.take().ok_or(AdmissionError::Ended)?,
        )))
    }

    /// Make a protected reservation visible only while its immutable
    /// authority deadline is still in the future.
    #[cfg(test)]
    pub fn activate_protected_if<F>(
        self,
        expires_at: u64,
        predicate: F,
    ) -> Result<Option<ActivatedAudioPeer>, AdmissionError>
    where
        F: FnOnce() -> bool,
    {
        let Ok(schedule) = ProtectedDeadlineSchedule::new(expires_at, None) else {
            return Ok(None);
        };
        let room = self.room.upgrade().ok_or(AdmissionError::Ended)?;
        let result = self.activate_protected_with_effects_if(
            schedule,
            ProtectedPeerEffects::new(CancellationToken::new()),
            predicate,
        )?;
        let Some((activated, epoch)) = result else {
            return Ok(None);
        };
        let pubkey = room
            .peers
            .get(&activated.0)
            .map(|peer| peer.pubkey.clone())
            .ok_or(AdmissionError::Ended)?;
        if room
            .broadcast_protected_join_if_current(epoch, &pubkey, activated.1)
            .is_none()
        {
            room.remove_protected_epoch(epoch);
            return Ok(None);
        }
        Ok(Some(activated))
    }

    /// Activate with the exact effects that deadline expiry must revoke before
    /// the peer is withdrawn from the roster. The V1 deadline is immutable:
    /// renewed authority must create a fresh admission/rejoin, so ordinary
    /// revalidation cannot silently extend an existing timer.
    pub(crate) fn activate_protected_with_effects_if<F>(
        mut self,
        schedule: ProtectedDeadlineSchedule,
        effects: ProtectedPeerEffects,
        predicate: F,
    ) -> Result<Option<(ActivatedAudioPeer, ProtectedPeerEpoch)>, AdmissionError>
    where
        F: FnOnce() -> bool,
    {
        let room = self.room.upgrade().ok_or(AdmissionError::Ended)?;
        let commit = room.activate_pending_if(
            self.admission_id,
            self.peer_id,
            Some(schedule),
            Some(effects),
            predicate,
        )?;
        if !commit.committed {
            return Ok(None);
        }
        self.activated = true;
        let activated = (
            self.peer_id,
            self.peer_index,
            self.audio_rx.take().ok_or(AdmissionError::Ended)?,
            self.ctrl_rx.take().ok_or(AdmissionError::Ended)?,
        );
        Ok(Some((
            activated,
            commit.protected_epoch.ok_or(AdmissionError::Ended)?,
        )))
    }
}

impl Drop for PendingAudioPeer {
    fn drop(&mut self) {
        if !self.activated {
            if let Some(room) = self.room.upgrade() {
                room.abort_pending(self.admission_id, self.peer_id);
            }
        }
    }
}

impl AdmissionGuard {
    fn new() -> Self {
        Self {
            next_fresh: 0,
            free: Vec::new(),
            ended: false,
            pinned_version: None,
            roster_revision: 0,
            pending: HashMap::new(),
            active: HashMap::new(),
            next_authority_generation: 1,
            owner_epoch: None,
            owner_claims: HashSet::new(),
        }
    }

    fn alloc(&mut self) -> Option<u8> {
        if let Some(idx) = self.free.pop() {
            return Some(idx);
        }
        if self.next_fresh == 255 {
            return None;
        }
        let idx = self.next_fresh;
        self.next_fresh += 1;
        Some(idx)
    }

    fn release(&mut self, idx: u8) {
        self.free.push(idx);
    }
}

/// A single audio room for one channel.
pub struct Room {
    /// Community this room belongs to.
    pub community_id: CommunityId,
    /// Channel UUID this room belongs to.
    pub channel_id: Uuid,
    /// Connected peers keyed by peer UUID.
    pub peers: DashMap<Uuid, AudioPeer>,
    /// Admission gate: index allocator + ended flag under one lock.
    guard: std::sync::Mutex<AdmissionGuard>,
    /// Ordered authoritative roster mutations. Lag is recoverable from
    /// [`Self::roster_snapshot`], so the owner never blocks admission.
    roster_tx: broadcast::Sender<RosterDelta>,
}

impl Room {
    /// Create an empty room for the given community-local channel.
    pub fn new(community_id: CommunityId, channel_id: Uuid) -> Self {
        let (roster_tx, _) = broadcast::channel(64);
        Self {
            community_id,
            channel_id,
            peers: DashMap::new(),
            guard: std::sync::Mutex::new(AdmissionGuard::new()),
            roster_tx,
        }
    }

    /// Mark the room as ended. After this returns, no new `add_peer` can
    /// succeed — they'll see `ended == true` under the same lock.
    /// Returns `true` if the room is empty (safe to archive + emit 48103).
    /// Returns `false` if a peer snuck in before we acquired the lock.
    pub fn mark_ended(&self) -> bool {
        self.prune_expired_authority();
        if let Ok(mut g) = self.guard.lock() {
            g.ended = true;
            self.peers.is_empty() && g.pending.is_empty() && g.owner_claims.is_empty()
        } else {
            false
        }
    }

    /// Undo `mark_ended` — used when archive needs to be rolled back.
    pub fn clear_ended(&self) {
        if let Ok(mut g) = self.guard.lock() {
            g.ended = false;
        }
    }

    fn mark_ended_if_empty(&self) -> bool {
        let Ok(mut guard) = self.guard.lock() else {
            return false;
        };
        if !self.peers.is_empty() || !guard.pending.is_empty() || !guard.owner_claims.is_empty() {
            return false;
        }
        guard.ended = true;
        true
    }

    fn owner_epoch(&self) -> Option<RoomOwnerEpoch> {
        self.guard.lock().ok().and_then(|guard| guard.owner_epoch)
    }

    pub(crate) fn matches_owner_epoch(&self, epoch: RoomOwnerEpoch) -> bool {
        self.owner_epoch() == Some(epoch)
    }

    fn claim_owner_epoch(&self, epoch: RoomOwnerEpoch) -> Result<Uuid, AdmissionError> {
        let mut guard = self.guard.lock().map_err(|_| AdmissionError::Ended)?;
        if guard.ended {
            return Err(AdmissionError::Ended);
        }
        match guard.owner_epoch {
            Some(current) if current == epoch => {
                let token = Uuid::new_v4();
                guard.owner_claims.insert(token);
                Ok(token)
            }
            Some(_) => Err(AdmissionError::Ended),
            None if self.peers.is_empty() && guard.pending.is_empty() => {
                guard.owner_epoch = Some(epoch);
                let token = Uuid::new_v4();
                guard.owner_claims.insert(token);
                Ok(token)
            }
            None => Err(AdmissionError::Ended),
        }
    }

    fn release_owner_claim(&self, epoch: RoomOwnerEpoch, token: Uuid) {
        if let Ok(mut guard) = self.guard.lock() {
            if guard.owner_epoch == Some(epoch) {
                guard.owner_claims.remove(&token);
            }
        }
    }

    /// Add a peer. Returns `(peer_id, peer_index, audio_rx, ctrl_rx)` on
    /// success, or an [`AdmissionError`] explaining why the peer was rejected.
    ///
    /// `requested_version` is the huddle audio protocol version the peer
    /// negotiated in its WS auth message. The first successful admission
    /// pins the room to that version; later admits must match the pin or
    /// they receive [`AdmissionError::VersionMismatch`].
    ///
    /// The cap check, ended check, version pin, index allocation, and peer
    /// insert all happen under the admission guard lock — mutually exclusive
    /// with `mark_ended` and with any concurrent `add_peer` that might race
    /// the version pin.
    ///
    /// Error precedence is deliberate: `Ended` > `Full` > `VersionMismatch`.
    /// A "no seat available" error wins over version mismatch because a
    /// client that couldn't join either way shouldn't learn the room's
    /// pinned protocol version — that's a (mild) information leak. The cap
    /// check lives inside the lock so two concurrent joiners can't both
    /// pass it; the per-room index space (255) plus the soft cap
    /// (`MAX_PEERS_PER_ROOM`) is then a single, race-free invariant.
    pub fn add_peer(
        &self,
        pubkey: String,
        requested_version: u8,
    ) -> Result<(Uuid, u8, mpsc::Receiver<Bytes>, mpsc::Receiver<PeerCtrl>), AdmissionError> {
        self.add_peer_inner(pubkey, requested_version, None)
    }

    /// Add an owner-side representation of a participant hosted by another
    /// runtime. Fan-out is grouped by runtime so one source frame produces one
    /// mesh datagram per destination pod, regardless of participant count.
    pub(crate) fn add_remote_peer(
        &self,
        pubkey: String,
        requested_version: u8,
        fanout_group: [u8; 32],
    ) -> Result<(Uuid, u8, mpsc::Receiver<Bytes>, mpsc::Receiver<PeerCtrl>), AdmissionError> {
        self.add_peer_inner(pubkey, requested_version, Some(fanout_group))
    }

    fn add_peer_inner(
        &self,
        pubkey: String,
        requested_version: u8,
        fanout_group: Option<[u8; 32]>,
    ) -> Result<(Uuid, u8, mpsc::Receiver<Bytes>, mpsc::Receiver<PeerCtrl>), AdmissionError> {
        self.prune_expired_authority();
        let mut g = self.guard.lock().map_err(
            |_| AdmissionError::Ended, /* poisoned ≈ shutting down */
        )?;
        if g.ended {
            return Err(AdmissionError::Ended);
        }
        if self.peers.len() >= MAX_PEERS_PER_ROOM {
            return Err(AdmissionError::Full);
        }
        if let Some(pinned) = g.pinned_version {
            if pinned != requested_version {
                return Err(AdmissionError::VersionMismatch {
                    pinned,
                    requested: requested_version,
                });
            }
        }
        let peer_index = g.alloc().ok_or(AdmissionError::Full)?;
        // Pin the room version on the first successful index allocation. We
        // pin *after* alloc so a Full error doesn't accidentally set the
        // version for a peer that didn't actually join.
        g.pinned_version.get_or_insert(requested_version);
        let peer_id = Uuid::new_v4();
        let (audio_tx, audio_rx) = mpsc::channel(AUDIO_CHANNEL_CAPACITY);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(CTRL_CHANNEL_CAPACITY);
        self.peers.insert(
            peer_id,
            AudioPeer {
                pubkey: pubkey.clone(),
                audio_tx,
                ctrl_tx,
                peer_index,
                admission_id: None,
                authority_expires_at: None,
                authority_wake_at: None,
                authority_generation: None,
                protected_effects: None,
                protected_join_published: true,
                fanout_group,
            },
        );
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let delta = RosterDelta {
            revision: g.roster_revision,
            joined: Some(RosterPeer { pubkey, peer_index }),
            left: None,
        };
        let _ = self.roster_tx.send(delta);
        drop(g); // Release lock after ordered roster publication.
        Ok((peer_id, peer_index, audio_rx, ctrl_rx))
    }

    /// Add a non-owner ingress peer at the index already allocated by the
    /// authoritative owner. No client-visible state is emitted before this
    /// succeeds, so a remote client has exactly one identity end-to-end.
    pub fn add_peer_at_index(
        &self,
        pubkey: String,
        requested_version: u8,
        peer_index: u8,
    ) -> Result<(Uuid, mpsc::Receiver<Bytes>, mpsc::Receiver<PeerCtrl>), AdmissionError> {
        self.prune_expired_authority();
        let mut g = self.guard.lock().map_err(|_| AdmissionError::Ended)?;
        if g.ended {
            return Err(AdmissionError::Ended);
        }
        if self.peers.len() >= MAX_PEERS_PER_ROOM
            || self.peers.iter().any(|peer| peer.peer_index == peer_index)
        {
            return Err(AdmissionError::Full);
        }
        if let Some(pinned) = g.pinned_version {
            if pinned != requested_version {
                return Err(AdmissionError::VersionMismatch {
                    pinned,
                    requested: requested_version,
                });
            }
        }
        g.pinned_version.get_or_insert(requested_version);
        // Keep a later local allocation from colliding if ownership changes
        // while this room is still winding down. Skipped lower indices are a
        // bounded handoff cost; a fresh room resets the allocator.
        g.free.retain(|idx| *idx != peer_index);
        if peer_index >= g.next_fresh {
            g.next_fresh = peer_index.saturating_add(1);
        }

        let peer_id = Uuid::new_v4();
        let (audio_tx, audio_rx) = mpsc::channel(AUDIO_CHANNEL_CAPACITY);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(CTRL_CHANNEL_CAPACITY);
        self.peers.insert(
            peer_id,
            AudioPeer {
                pubkey: pubkey.clone(),
                audio_tx,
                ctrl_tx,
                peer_index,
                admission_id: None,
                authority_expires_at: None,
                authority_wake_at: None,
                authority_generation: None,
                protected_effects: None,
                protected_join_published: true,
                fanout_group: None,
            },
        );
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let delta = RosterDelta {
            revision: g.roster_revision,
            joined: Some(RosterPeer { pubkey, peer_index }),
            left: None,
        };
        let _ = self.roster_tx.send(delta);
        drop(g);
        Ok((peer_id, audio_rx, ctrl_rx))
    }

    /// Reserve a local peer without exposing it to roster or media fan-out.
    pub fn reserve_peer(
        self: &Arc<Self>,
        admission_id: Uuid,
        pubkey: String,
        requested_version: u8,
    ) -> Result<PendingAudioPeer, AdmissionError> {
        self.reserve_peer_inner(admission_id, pubkey, requested_version, None, None)
    }

    /// Reserve an owner-side remote participant without making it visible.
    pub(crate) fn reserve_remote_peer(
        self: &Arc<Self>,
        admission_id: Uuid,
        pubkey: String,
        requested_version: u8,
        fanout_group: [u8; 32],
    ) -> Result<PendingAudioPeer, AdmissionError> {
        self.reserve_peer_inner(
            admission_id,
            pubkey,
            requested_version,
            None,
            Some(fanout_group),
        )
    }

    /// Reserve an ingress peer at the index already chosen by the room owner.
    pub fn reserve_peer_at_index(
        self: &Arc<Self>,
        admission_id: Uuid,
        pubkey: String,
        requested_version: u8,
        peer_index: u8,
    ) -> Result<PendingAudioPeer, AdmissionError> {
        self.reserve_peer_inner(
            admission_id,
            pubkey,
            requested_version,
            Some(peer_index),
            None,
        )
    }

    fn reserve_peer_inner(
        self: &Arc<Self>,
        admission_id: Uuid,
        pubkey: String,
        requested_version: u8,
        requested_index: Option<u8>,
        fanout_group: Option<[u8; 32]>,
    ) -> Result<PendingAudioPeer, AdmissionError> {
        self.prune_expired_authority();
        let mut guard = self.guard.lock().map_err(|_| AdmissionError::Ended)?;
        if guard.ended {
            return Err(AdmissionError::Ended);
        }
        if self.peers.len() + guard.pending.len() >= MAX_PEERS_PER_ROOM
            || guard.pending.contains_key(&admission_id)
            || guard.active.contains_key(&admission_id)
        {
            return Err(AdmissionError::Full);
        }
        if let Some(pinned) = guard.pinned_version {
            if pinned != requested_version {
                return Err(AdmissionError::VersionMismatch {
                    pinned,
                    requested: requested_version,
                });
            }
        }
        let peer_index = match requested_index {
            Some(index)
                if !self.peers.iter().any(|peer| peer.peer_index == index)
                    && !guard
                        .pending
                        .values()
                        .any(|pending| pending.peer_index == index) =>
            {
                guard.free.retain(|candidate| *candidate != index);
                if index >= guard.next_fresh {
                    guard.next_fresh = index.saturating_add(1);
                }
                index
            }
            Some(_) => return Err(AdmissionError::Full),
            None => guard.alloc().ok_or(AdmissionError::Full)?,
        };
        guard.pinned_version.get_or_insert(requested_version);
        let peer_id = Uuid::new_v4();
        let (audio_tx, audio_rx) = mpsc::channel(AUDIO_CHANNEL_CAPACITY);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(CTRL_CHANNEL_CAPACITY);
        guard.pending.insert(
            admission_id,
            PendingPeerRecord {
                peer_id,
                pubkey,
                peer_index,
                audio_tx,
                ctrl_tx,
                fanout_group,
            },
        );
        Ok(PendingAudioPeer {
            room: Arc::downgrade(self),
            admission_id,
            peer_id,
            peer_index,
            audio_rx: Some(audio_rx),
            ctrl_rx: Some(ctrl_rx),
            activated: false,
        })
    }

    fn activate_pending_if<F>(
        self: &Arc<Self>,
        admission_id: Uuid,
        peer_id: Uuid,
        authority_schedule: Option<ProtectedDeadlineSchedule>,
        protected_effects: Option<ProtectedPeerEffects>,
        predicate: F,
    ) -> Result<ActivationCommit, AdmissionError>
    where
        F: FnOnce() -> bool,
    {
        let authority_expires_at = authority_schedule.map(ProtectedDeadlineSchedule::deadline);
        let authority_wake_at = authority_schedule.map(ProtectedDeadlineSchedule::wake_at);
        let timer = authority_schedule
            .map(|schedule| {
                let handle =
                    tokio::runtime::Handle::try_current().map_err(|_| AdmissionError::Ended)?;
                Ok::<_, AdmissionError>((handle, schedule.wake_at()))
            })
            .transpose()?;
        let mut guard = self.guard.lock().map_err(|_| AdmissionError::Ended)?;
        if guard.ended {
            return Err(AdmissionError::Ended);
        }
        if !predicate() || authority_schedule.is_some_and(|schedule| !schedule.is_current()) {
            return Ok(ActivationCommit {
                committed: false,
                protected_epoch: None,
            });
        }
        let pending = guard
            .pending
            .remove(&admission_id)
            .filter(|pending| pending.peer_id == peer_id)
            .ok_or(AdmissionError::Ended)?;
        let authority_generation = authority_expires_at.map(|_| {
            let generation = guard.next_authority_generation;
            guard.next_authority_generation =
                guard.next_authority_generation.wrapping_add(1).max(1);
            generation
        });
        self.peers.insert(
            peer_id,
            AudioPeer {
                pubkey: pending.pubkey.clone(),
                audio_tx: pending.audio_tx,
                ctrl_tx: pending.ctrl_tx,
                peer_index: pending.peer_index,
                admission_id: Some(admission_id),
                authority_expires_at,
                authority_wake_at,
                authority_generation,
                protected_effects,
                protected_join_published: authority_schedule.is_none(),
                fanout_group: pending.fanout_group,
            },
        );
        guard.active.insert(admission_id, peer_id);
        let protected_epoch = if let (Some(deadline), Some(generation)) =
            (authority_expires_at, authority_generation)
        {
            let epoch = ProtectedPeerEpoch {
                community_id: self.community_id,
                channel_id: self.channel_id,
                peer_id,
                admission_id,
                generation,
                deadline,
                wake_at: authority_wake_at.ok_or(AdmissionError::Ended)?,
            };
            if let Some((handle, wake_at)) = timer {
                let room = Arc::downgrade(self);
                handle.spawn(async move {
                    tokio::time::sleep_until(wake_at).await;
                    if let Some(room) = room.upgrade() {
                        room.expire_protected_epoch(epoch);
                    }
                });
            }
            Some(epoch)
        } else {
            None
        };
        // Protected activation is deliberately hidden. Publication performs
        // the final synchronous authority check and is the sole linearization
        // point for roster, snapshot, media, and control visibility.
        if authority_schedule.is_none() {
            guard.roster_revision = guard.roster_revision.wrapping_add(1);
            let _ = self.roster_tx.send(RosterDelta {
                revision: guard.roster_revision,
                joined: Some(RosterPeer {
                    pubkey: pending.pubkey,
                    peer_index: pending.peer_index,
                }),
                left: None,
            });
        }
        Ok(ActivationCommit {
            committed: true,
            protected_epoch,
        })
    }

    fn abort_pending(&self, admission_id: Uuid, peer_id: Uuid) {
        let Ok(mut guard) = self.guard.lock() else {
            return;
        };
        if let Some(pending) = guard
            .pending
            .remove(&admission_id)
            .filter(|pending| pending.peer_id == peer_id)
        {
            guard.release(pending.peer_index);
            if self.peers.is_empty() && guard.pending.is_empty() && guard.owner_claims.is_empty() {
                guard.pinned_version = None;
            }
        }
    }

    /// Expire only the exact activation that scheduled this timer. Reused
    /// peer ids, admission ids, and later deadlines cannot be evicted by a
    /// stale task because all epoch fields must still match under the room
    /// admission lock.
    fn expire_protected_epoch(&self, epoch: ProtectedPeerEpoch) -> bool {
        if self.community_id != epoch.community_id || self.channel_id != epoch.channel_id {
            return false;
        }
        let Ok(mut guard) = self.guard.lock() else {
            return false;
        };
        let matches = self
            .peers
            .get(&epoch.peer_id)
            .is_some_and(|peer| peer.matches_epoch(epoch));
        if !matches {
            return false;
        }
        let Some((_, peer)) = self.peers.remove(&epoch.peer_id) else {
            return false;
        };

        // Revoke every exact media/control effect before publishing that this
        // peer is gone. Late effect registrations observe closed and compensate
        // synchronously instead of resurrecting the admission.
        let _ = peer.ctrl_tx.try_send(PeerCtrl::Close);
        if let Some(effects) = peer.protected_effects.as_ref() {
            effects.revoke();
        }
        guard.active.remove(&epoch.admission_id);
        guard.release(peer.peer_index);
        if peer.protected_join_published {
            guard.roster_revision = guard.roster_revision.wrapping_add(1);
            let left = RosterPeer {
                pubkey: peer.pubkey,
                peer_index: peer.peer_index,
            };
            let _ = self.roster_tx.send(RosterDelta {
                revision: guard.roster_revision,
                joined: None,
                left: Some(left.clone()),
            });
            let message = serde_json::json!({
                "type": "left",
                "pubkey": left.pubkey,
                "peer_index": left.peer_index,
            })
            .to_string();
            for remaining in self.peers.iter().filter(|peer| peer.is_visible()) {
                let _ = remaining.ctrl_tx.try_send(PeerCtrl::Json(message.clone()));
            }
        }
        if self.peers.is_empty() && guard.pending.is_empty() && guard.owner_claims.is_empty() {
            guard.pinned_version = None;
        }
        true
    }

    /// Retire exactly one protected admission generation. Stale cleanup can
    /// never remove a renewed or rejoined peer that reused an admission id.
    pub(crate) fn remove_protected_epoch(&self, epoch: ProtectedPeerEpoch) -> bool {
        self.expire_protected_epoch(epoch)
    }

    /// Remove a peer and recycle its index.
    pub fn remove_peer(&self, peer_id: Uuid) -> bool {
        let Ok(mut g) = self.guard.lock() else {
            return false;
        };
        if let Some((_, peer)) = self.peers.remove(&peer_id) {
            let _ = peer.ctrl_tx.try_send(PeerCtrl::Close);
            if let Some(effects) = peer.protected_effects.as_ref() {
                effects.revoke();
            }
            if let Some(admission_id) = peer.admission_id {
                g.active.remove(&admission_id);
            }
            g.release(peer.peer_index);
            if peer.protected_join_published {
                g.roster_revision = g.roster_revision.wrapping_add(1);
                let delta = RosterDelta {
                    revision: g.roster_revision,
                    joined: None,
                    left: Some(RosterPeer {
                        pubkey: peer.pubkey,
                        peer_index: peer.peer_index,
                    }),
                };
                let _ = self.roster_tx.send(delta);
            }
            drop(g);
            true
        } else {
            false
        }
    }

    /// Remove a peer AND atomically check if the room should end.
    /// If the room is now empty, sets `ended = true` under the same lock
    /// acquisition that recycles the index — no window for a concurrent
    /// `add_peer` to sneak in between removal and the ended flag.
    /// Returns `(peer_index, should_auto_end)`.
    pub fn remove_peer_and_check_ended(&self, peer_id: Uuid) -> Option<(u8, bool)> {
        let mut g = self.guard.lock().ok()?;
        let (_, peer) = self.peers.remove(&peer_id)?;
        let _ = peer.ctrl_tx.try_send(PeerCtrl::Close);
        if let Some(effects) = peer.protected_effects.as_ref() {
            effects.revoke();
        }
        if let Some(admission_id) = peer.admission_id {
            g.active.remove(&admission_id);
        }
        let peer_index = peer.peer_index;
        g.release(peer_index);
        let delta = peer.protected_join_published.then(|| {
            g.roster_revision = g.roster_revision.wrapping_add(1);
            RosterDelta {
                revision: g.roster_revision,
                joined: None,
                left: Some(RosterPeer {
                    pubkey: peer.pubkey,
                    peer_index,
                }),
            }
        });
        // Only the first task to see empty + !ended wins the auto-end.
        // This prevents duplicate archive/48103 when two peers disconnect
        // simultaneously and both see is_empty() == true.
        let should_end = if !g.ended
            && self.peers.is_empty()
            && g.pending.is_empty()
            && g.owner_claims.is_empty()
        {
            g.ended = true;
            true
        } else {
            false
        };
        if let Some(delta) = delta {
            let _ = self.roster_tx.send(delta);
        }
        drop(g);
        Some((peer_index, should_end))
    }

    /// Fan-out a binary frame to all peers except the sender.
    /// Prepends the sender's `peer_index` as a 1-byte prefix.
    /// Drops on full buffer — real-time audio never queues.
    pub fn broadcast_frame(&self, sender_id: Uuid, frame: Bytes) {
        self.prune_expired_authority();
        let sender_index = match self.peers.get(&sender_id) {
            Some(p) if p.is_visible() => p.peer_index,
            None => return,
            Some(_) => return,
        };

        // Prepend peer_index as 1-byte header.
        let mut prefixed = bytes::BytesMut::with_capacity(1 + frame.len());
        prefixed.extend_from_slice(&[sender_index]);
        prefixed.extend_from_slice(&frame);
        let prefixed = prefixed.freeze();

        let mut delivered_groups = std::collections::HashSet::new();
        for entry in self.peers.iter() {
            if *entry.key() == sender_id || !entry.is_visible() {
                continue;
            }
            if let Some(group) = entry.fanout_group {
                if !delivered_groups.insert(group) {
                    continue;
                }
            }
            let _ = entry.audio_tx.try_send(prefixed.clone());
        }
    }

    /// Deliver an already-`[peer_index]`-prefixed frame that arrived over the
    /// mesh to every local peer except the one whose `peer_index` authored it.
    ///
    /// Used by the cross-pod media path ([`super::mesh`]): the frame is
    /// byte-identical to what `broadcast_frame` produces, but the author is a
    /// *remote* participant identified only by its (owner-assigned) index, not
    /// a local peer UUID. Skipping by index keeps a participant whose own frame
    /// round-tripped owner→back-to-their-pod from hearing themselves. Drops on
    /// full — real-time audio never queues.
    pub fn deliver_prefixed(&self, author_index: u8, prefixed: Bytes) {
        self.prune_expired_authority();
        for entry in self.peers.iter() {
            if entry.peer_index == author_index || !entry.is_visible() {
                continue;
            }
            let _ = entry.audio_tx.try_send(prefixed.clone());
        }
    }

    /// Deliver owner fan-out only to local peers whose exact protected
    /// admission (or legacy peer identity) still has a live media attachment.
    pub fn deliver_prefixed_to_admissions(
        &self,
        author_index: u8,
        prefixed: Bytes,
        admissions: &std::collections::HashSet<Uuid>,
    ) {
        self.prune_expired_authority();
        for entry in self.peers.iter() {
            if entry.peer_index == author_index || !entry.is_visible() {
                continue;
            }
            let recipient = entry.admission_id.unwrap_or(*entry.key());
            if admissions.contains(&recipient) {
                let _ = entry.audio_tx.try_send(prefixed.clone());
            }
        }
    }

    /// Send a JSON control message to all peers via the control channel.
    /// Separate from audio so control is never starved by audio backpressure.
    /// Control messages (joined/left) are state-bearing — the client's
    /// peer_index→pubkey map depends on receiving every one. The channel is
    /// sized generously (32 slots) so drops should never happen in practice;
    /// if they do, we log a warning so the issue is visible.
    pub fn broadcast_control(&self, json: String) {
        self.prune_expired_authority();
        for entry in self.peers.iter().filter(|peer| peer.is_visible()) {
            if entry
                .ctrl_tx
                .try_send(PeerCtrl::Json(json.clone()))
                .is_err()
            {
                tracing::warn!(
                    peer_id = %entry.key(),
                    "control channel full — dropped state-bearing message (peer map may desync)"
                );
            }
        }
    }

    /// Publish one protected join only if that exact peer is still present
    /// and its absolute authority deadline remains current at emission.
    ///
    /// The returned roster is captured under the same admission lock as the
    /// control fan-out, so a caller cannot acknowledge a peer that pruning
    /// removed between a stale prebuilt `joined` message and its reply.
    pub(crate) fn publish_protected_join_if_current<T, F>(
        &self,
        epoch: ProtectedPeerEpoch,
        expected_pubkey: &str,
        expected_index: u8,
        publish: F,
    ) -> Option<(RosterSnapshot, T)>
    where
        F: FnOnce(&str, u8, &RosterSnapshot) -> Option<T>,
    {
        self.prune_expired_authority();
        let mut guard = self.guard.lock().ok()?;
        let peer = self.peers.get(&epoch.peer_id)?;
        if peer.pubkey != expected_pubkey
            || peer.peer_index != expected_index
            || !peer.matches_epoch(epoch)
            || peer.protected_join_published
            || !(ProtectedDeadlineSchedule {
                deadline: epoch.deadline,
                wake_at: epoch.wake_at,
            })
            .is_current()
            || !peer
                .protected_effects
                .as_ref()
                .is_some_and(ProtectedPeerEffects::is_live)
        {
            return None;
        }
        let pubkey = peer.pubkey.clone();
        let peer_index = peer.peer_index;
        drop(peer);
        let next_revision = guard.roster_revision.wrapping_add(1);
        let mut peers = self
            .peers
            .iter()
            .filter(|entry| entry.is_visible())
            .map(|entry| RosterPeer {
                pubkey: entry.pubkey.clone(),
                peer_index: entry.peer_index,
            })
            .collect::<Vec<_>>();
        peers.push(RosterPeer {
            pubkey: pubkey.clone(),
            peer_index,
        });
        peers.sort_by_key(|entry| entry.peer_index);
        let snapshot = RosterSnapshot {
            revision: next_revision,
            peers,
        };
        // The candidate snapshot may take time to construct. Recheck the exact
        // schedule immediately before the non-awaiting sink publication.
        let peer = self.peers.get(&epoch.peer_id)?;
        if !peer.matches_epoch(epoch)
            || peer.protected_join_published
            || !peer.authority_is_current()
        {
            return None;
        }
        drop(peer);
        let output = publish(&pubkey, peer_index, &snapshot)?;
        let mut peer = self.peers.get_mut(&epoch.peer_id)?;
        if !peer.matches_epoch(epoch) || peer.protected_join_published {
            return None;
        }
        peer.protected_join_published = true;
        drop(peer);
        guard.roster_revision = next_revision;
        let _ = self.roster_tx.send(RosterDelta {
            revision: guard.roster_revision,
            joined: Some(RosterPeer {
                pubkey: pubkey.clone(),
                peer_index,
            }),
            left: None,
        });
        drop(guard);
        Some((snapshot, output))
    }

    /// Publish one exact protected join to room control queues.
    pub(crate) fn broadcast_protected_join_if_current(
        &self,
        epoch: ProtectedPeerEpoch,
        expected_pubkey: &str,
        expected_index: u8,
    ) -> Option<RosterSnapshot> {
        self.publish_protected_join_if_current(
            epoch,
            expected_pubkey,
            expected_index,
            |pubkey, peer_index, _snapshot| {
                let joined = serde_json::json!({
                    "type": "joined",
                    "pubkey": pubkey,
                    "peer_index": peer_index,
                    "peers": [{"pubkey": expected_pubkey, "peer_index": expected_index}],
                })
                .to_string();
                for entry in self
                    .peers
                    .iter()
                    .filter(|peer| peer.is_visible() || *peer.key() == epoch.peer_id)
                {
                    if entry
                        .ctrl_tx
                        .try_send(PeerCtrl::Json(joined.clone()))
                        .is_err()
                    {
                        tracing::warn!(
                        peer_id = %entry.key(),
                        "control channel full — dropped state-bearing message (peer map may desync)"
                            );
                    }
                }
                Some(())
            },
        )
        .map(|(snapshot, ())| snapshot)
    }

    /// Whether one exact protected admission generation is currently visible
    /// and authorized for media/control effects.
    pub(crate) fn is_protected_epoch_current(&self, epoch: ProtectedPeerEpoch) -> bool {
        self.prune_expired_authority();
        self.guard.lock().is_ok_and(|_| {
            self.peers
                .get(&epoch.peer_id)
                .is_some_and(|peer| peer.matches_epoch(epoch) && peer.is_visible())
        })
    }

    /// Bind owner-side ingress to the exact published admission/index pair.
    pub(crate) fn is_published_admission(&self, admission_id: Uuid, peer_index: u8) -> bool {
        self.prune_expired_authority();
        self.peers.iter().any(|peer| {
            peer.admission_id == Some(admission_id)
                && peer.peer_index == peer_index
                && peer.is_visible()
        })
    }

    /// Subscribe to ordered roster mutations. A lagged receiver must call
    /// [`Self::roster_snapshot`] and continue from that snapshot's revision.
    pub fn subscribe_roster(&self) -> broadcast::Receiver<RosterDelta> {
        self.roster_tx.subscribe()
    }

    /// Capture a complete roster and its revision atomically with respect to
    /// admission/removal. Subscribe before calling this to close the
    /// snapshot-to-delta race; stale deltas at or below `revision` are ignored.
    pub fn roster_snapshot(&self) -> RosterSnapshot {
        self.prune_expired_authority();
        let g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        let mut peers = self
            .peers
            .iter()
            .filter(|entry| entry.is_visible())
            .map(|e| RosterPeer {
                pubkey: e.pubkey.clone(),
                peer_index: e.peer_index,
            })
            .collect::<Vec<_>>();
        peers.sort_by_key(|peer| peer.peer_index);
        RosterSnapshot {
            revision: g.roster_revision,
            peers,
        }
    }

    /// All `(pubkey, peer_index)` pairs in the room.
    pub fn peer_pubkeys(&self) -> Vec<(String, u8)> {
        self.prune_expired_authority();
        self.peers
            .iter()
            .filter(|entry| entry.is_visible())
            .map(|e| (e.pubkey.clone(), e.peer_index))
            .collect()
    }

    /// True if no peers remain in the room.
    pub fn is_empty(&self) -> bool {
        self.prune_expired_authority();
        self.peers.is_empty()
            && self
                .guard
                .lock()
                .map(|guard| guard.pending.is_empty() && guard.owner_claims.is_empty())
                .unwrap_or(false)
    }

    fn prune_expired_authority(&self) {
        self.prune_expired_authority_at(unix_time_seconds());
    }

    fn prune_expired_authority_at(&self, now: u64) {
        let expired = self
            .peers
            .iter()
            .filter_map(|peer| {
                let deadline = peer.authority_expires_at?;
                let wake_at = peer.authority_wake_at?;
                (deadline <= now || !peer.authority_is_current()).then_some(ProtectedPeerEpoch {
                    community_id: self.community_id,
                    channel_id: self.channel_id,
                    peer_id: *peer.key(),
                    admission_id: peer.admission_id?,
                    generation: peer.authority_generation?,
                    deadline,
                    wake_at,
                })
            })
            .collect::<Vec<_>>();
        for epoch in expired {
            self.expire_protected_epoch(epoch);
        }
    }
}

fn unix_time_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Global registry of active audio rooms.
pub struct AudioRoomManager {
    rooms: DashMap<(CommunityId, Uuid), Arc<Room>>,
}

/// RAII claim for the asynchronous gap before a protected reservation exists.
/// The unique token prevents room retirement and owner-generation reuse until
/// the caller either creates a pending peer or exits.
pub(crate) struct ProtectedRoomClaim {
    room: Arc<Room>,
    epoch: RoomOwnerEpoch,
    token: Uuid,
}

impl ProtectedRoomClaim {
    pub(crate) fn room(&self) -> Arc<Room> {
        Arc::clone(&self.room)
    }
}

impl std::ops::Deref for ProtectedRoomClaim {
    type Target = Arc<Room>;

    fn deref(&self) -> &Self::Target {
        &self.room
    }
}

impl Drop for ProtectedRoomClaim {
    fn drop(&mut self) {
        self.room.release_owner_claim(self.epoch, self.token);
    }
}

impl AudioRoomManager {
    /// Create an empty room manager.
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
        }
    }

    /// Get an existing room or create a new one.
    ///
    /// Channel UUIDs are only unique inside a community. The room key must
    /// carry both labels so two tenants that legitimately reuse the same UUID
    /// never share peer lists, protocol pins, or audio frames.
    pub fn get_or_create(&self, community_id: CommunityId, channel_id: Uuid) -> Arc<Room> {
        self.rooms
            .entry((community_id, channel_id))
            .or_insert_with(|| Arc::new(Room::new(community_id, channel_id)))
            .clone()
    }

    /// Claim a room for one exact mesh owner incarnation. A different epoch
    /// may replace only a quiescent room, and the old owner is released while
    /// that exact incarnation is still fenced from new admission.
    pub(crate) fn get_or_create_for_owner<F>(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        epoch: RoomOwnerEpoch,
        release_old_owner: F,
    ) -> Result<ProtectedRoomClaim, AdmissionError>
    where
        F: Fn(RoomOwnerEpoch),
    {
        loop {
            let room = self.get_or_create(community_id, channel_id);
            if let Ok(token) = room.claim_owner_epoch(epoch) {
                return Ok(ProtectedRoomClaim { room, epoch, token });
            }
            let Some(old_epoch) = room.owner_epoch() else {
                return Err(AdmissionError::Ended);
            };
            if old_epoch == epoch
                || !self.retire_exact_owner_if_empty(
                    community_id,
                    channel_id,
                    &room,
                    old_epoch,
                    || release_old_owner(old_epoch),
                )
            {
                return Err(AdmissionError::Ended);
            }
        }
    }

    /// Look up an existing community-local room without creating one.
    pub fn get(&self, community_id: CommunityId, channel_id: Uuid) -> Option<Arc<Room>> {
        self.rooms
            .get(&(community_id, channel_id))
            .map(|room| room.clone())
    }

    /// Look up a room for a mesh datagram that carries only a channel UUID.
    ///
    /// The current mesh media envelope does not carry a community identifier.
    /// If two active communities use the same channel UUID, routing would be
    /// ambiguous, so fail closed instead of delivering one community's audio
    /// to the other. Control-path lookups always use [`Self::get`].
    pub fn get_unambiguous_by_channel(&self, channel_id: Uuid) -> Option<Arc<Room>> {
        let mut matches = self
            .rooms
            .iter()
            .filter(|entry| entry.key().1 == channel_id)
            .map(|entry| Arc::clone(entry.value()));
        let room = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(room)
    }

    /// Remove the room if it has no peers. Returns `true` if the room was removed.
    pub fn cleanup_if_empty(&self, community_id: CommunityId, channel_id: Uuid) -> bool {
        self.rooms
            .remove_if(&(community_id, channel_id), |_, room| room.is_empty())
            .is_some()
    }

    /// Retire one exact room incarnation and run its owner-release fence while
    /// the map key is still exclusively held. A concurrent rejoin therefore
    /// either lands in the old room before it is found empty, or creates a new
    /// room only after the old generation has been released.
    #[cfg(test)]
    pub(crate) fn retire_exact_if_empty<F>(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        expected: &Arc<Room>,
        release_owner: F,
    ) -> bool
    where
        F: FnOnce(),
    {
        let mut release_owner = Some(release_owner);
        self.rooms
            .remove_if(&(community_id, channel_id), |_, current| {
                if !Arc::ptr_eq(current, expected) || !current.mark_ended_if_empty() {
                    return false;
                }
                release_owner.take().expect("release called once")();
                true
            })
            .is_some()
    }

    /// Retire only the exact room pointer and exact owner incarnation.
    pub(crate) fn retire_exact_owner_if_empty<F>(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        expected: &Arc<Room>,
        expected_epoch: RoomOwnerEpoch,
        release_owner: F,
    ) -> bool
    where
        F: FnOnce(),
    {
        let mut release_owner = Some(release_owner);
        self.rooms
            .remove_if(&(community_id, channel_id), |_, current| {
                if !Arc::ptr_eq(current, expected)
                    || current.owner_epoch() != Some(expected_epoch)
                    || !current.mark_ended_if_empty()
                {
                    return false;
                }
                release_owner.take().expect("release called once")();
                true
            })
            .is_some()
    }
}

impl Default for AudioRoomManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_room() -> Room {
        Room::new(CommunityId::from_uuid(Uuid::new_v4()), Uuid::new_v4())
    }

    #[test]
    fn owner_assigned_index_is_preserved_and_reserved() {
        let room = fresh_room();
        let (_local_id, local_index, ..) = room.add_peer("owner-local".into(), 2).unwrap();
        assert_eq!(local_index, 0);

        let (remote_id, _audio, _ctrl) = room
            .add_peer_at_index("remote".into(), 2, 7)
            .expect("owner-assigned index admits");
        assert_eq!(room.peers.get(&remote_id).unwrap().peer_index, 7);

        let (_next_id, next_index, ..) = room.add_peer("next-local".into(), 2).unwrap();
        assert_eq!(
            next_index, 8,
            "local allocation cannot collide with owner index"
        );
    }

    #[test]
    fn roster_revisions_are_ordered_and_snapshot_is_authoritative() {
        let room = fresh_room();
        let mut deltas = room.subscribe_roster();
        let (alice, alice_index, ..) = room.add_peer("alice".into(), 2).unwrap();
        let (_bob, bob_index, ..) = room.add_peer("bob".into(), 2).unwrap();
        room.remove_peer(alice);

        assert_eq!(deltas.try_recv().unwrap().revision, 1);
        assert_eq!(deltas.try_recv().unwrap().revision, 2);
        let leave = deltas.try_recv().unwrap();
        assert_eq!(leave.revision, 3);
        assert_eq!(
            leave.left.as_ref().map(|peer| peer.peer_index),
            Some(alice_index)
        );

        let snapshot = room.roster_snapshot();
        assert_eq!(snapshot.revision, 3);
        assert_eq!(
            snapshot.peers,
            vec![RosterPeer {
                pubkey: "bob".into(),
                peer_index: bob_index,
            }]
        );
    }

    #[tokio::test]
    async fn expired_protected_peer_is_withdrawn_before_roster_or_media_emission() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        let (peer_id, peer_index, _audio, mut control) = pending
            .activate_protected_if(unix_time_seconds() + 3_600, || true)
            .expect("activation result")
            .expect("protected peer activates before deadline");
        let (_legacy_id, ..) = room.add_peer("legacy".into(), 2).expect("legacy peer");
        let mut deltas = room.subscribe_roster();

        room.prune_expired_authority_at(u64::MAX);

        assert!(!room.peers.contains_key(&peer_id));
        assert!(room
            .roster_snapshot()
            .peers
            .iter()
            .all(|peer| peer.peer_index != peer_index));
        let left = deltas.try_recv().expect("expiry publishes a leave delta");
        assert_eq!(left.left.map(|peer| peer.peer_index), Some(peer_index));
        assert!(
            std::iter::from_fn(|| control.try_recv().ok())
                .any(|message| matches!(message, PeerCtrl::Close)),
            "expiry queues exact close after any prior join control"
        );
        room.prune_expired_authority_at(u64::MAX);
        assert!(deltas.try_recv().is_err(), "retry is idempotent");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_protected_peer_closes_at_exact_deadline_without_room_activity() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let deadline = unix_time_seconds() + 10;
        let mut deltas = room.subscribe_roster();
        let pending = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        let (peer_id, peer_index, _audio, mut control) = pending
            .activate_protected_if(deadline, || true)
            .expect("activation result")
            .expect("protected peer activates before deadline");
        let joined = deltas.try_recv().expect("activation publishes join");
        assert_eq!(joined.joined.map(|peer| peer.peer_index), Some(peer_index));

        tokio::time::advance(std::time::Duration::from_secs(10)).await;
        tokio::task::yield_now().await;

        assert!(
            !room.peers.contains_key(&peer_id),
            "idle protected peer must be removed without a later room call"
        );
        assert!(
            !room
                .guard
                .lock()
                .expect("room guard")
                .active
                .contains_key(&admission_id),
            "expiry must remove the exact active admission"
        );
        let left = deltas.try_recv().expect("expiry publishes one leave");
        assert_eq!(left.left.map(|peer| peer.peer_index), Some(peer_index));
        assert!(
            std::iter::from_fn(|| control.try_recv().ok())
                .any(|message| matches!(message, PeerCtrl::Close)),
            "expiry closes the exact control channel after prior join control"
        );
        assert!(deltas.try_recv().is_err(), "expiry is idempotent");
    }

    #[tokio::test(start_paused = true)]
    async fn expiry_revokes_effects_before_publishing_roster_withdrawal() {
        let room = Arc::new(fresh_room());
        let effects = ProtectedPeerEffects::new(CancellationToken::new());
        let revoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = Arc::clone(&revoked);
        assert!(effects.install_revoker(move || {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        let mut deltas = room.subscribe_roster();
        let pending = room
            .reserve_peer(Uuid::new_v4(), "protected".into(), 2)
            .expect("reserve protected peer");
        let schedule = ProtectedDeadlineSchedule::new(unix_time_seconds() + 5, None)
            .expect("future protected deadline");
        let ((_peer_id, peer_index, _audio, _control), epoch) = pending
            .activate_protected_with_effects_if(schedule, effects, || true)
            .expect("activation")
            .expect("visible before expiry");
        assert!(room
            .broadcast_protected_join_if_current(epoch, "protected", peer_index)
            .is_some());
        let _ = deltas.try_recv().expect("join");

        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        tokio::task::yield_now().await;

        assert!(revoked.load(std::sync::atomic::Ordering::SeqCst));
        assert!(deltas.try_recv().expect("leave").left.is_some());
    }

    #[test]
    fn effect_registration_after_expiry_compensates_immediately() {
        let effects = ProtectedPeerEffects::new(CancellationToken::new());
        let first = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let first_observed = Arc::clone(&first);
        assert!(effects.install_revoker(move || {
            first_observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        effects.revoke();
        effects.revoke();
        assert_eq!(first.load(std::sync::atomic::Ordering::SeqCst), 1);
        let compensated = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = Arc::clone(&compensated);

        assert!(!effects.install_revoker(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        assert_eq!(compensated.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn stale_timer_cannot_evict_a_rejoined_admission() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let first = room
            .reserve_peer(admission_id, "first".into(), 2)
            .expect("reserve first")
            .activate_protected_if(unix_time_seconds() + 5, || true)
            .expect("activate first")
            .expect("first visible");
        room.remove_peer(first.0);
        let second = room
            .reserve_peer(admission_id, "second".into(), 2)
            .expect("reserve rejoin")
            .activate_protected_if(unix_time_seconds() + 20, || true)
            .expect("activate rejoin")
            .expect("rejoin visible");

        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        tokio::task::yield_now().await;

        assert!(room.peers.contains_key(&second.0));
        assert_eq!(room.roster_snapshot().peers[0].pubkey, "second");
    }

    #[test]
    fn subsecond_deadline_delay_must_not_round_up() {
        let wall_now = std::time::Duration::new(100, 900_000_000);
        let deadline = 101_u64;

        let monotonic_now = tokio::time::Instant::now();
        let schedule =
            ProtectedDeadlineSchedule::from_samples(deadline, monotonic_now, wall_now, None)
                .expect("future deadline");
        let delay = schedule.wake_at().duration_since(monotonic_now);

        assert_eq!(delay, std::time::Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn elapsed_monotonic_deadline_rejects_activation_without_timer_poll() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        let monotonic_now = tokio::time::Instant::now();
        let wall_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock after epoch");
        let schedule = ProtectedDeadlineSchedule::from_samples(
            unix_time_seconds() + 3_600,
            monotonic_now,
            wall_now,
            Some(std::time::Duration::ZERO),
        )
        .expect("absolute deadline remains in the future");

        let activated = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result");

        assert!(
            activated.is_none(),
            "wake_at equality is expired even when the timer has not polled"
        );
        assert!(room.peers.is_empty());
        assert_eq!(room.roster_snapshot().revision, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn elapsed_monotonic_deadline_rejects_publication_without_timer_poll() {
        let room = Arc::new(fresh_room());
        let pending = room
            .reserve_peer(Uuid::new_v4(), "protected".into(), 2)
            .expect("reserve protected peer");
        let monotonic_now = tokio::time::Instant::now();
        let wall_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock after epoch");
        let schedule = ProtectedDeadlineSchedule::from_samples(
            unix_time_seconds() + 3_600,
            monotonic_now,
            wall_now,
            Some(std::time::Duration::from_secs(2)),
        )
        .expect("future schedule");
        let ((_peer_id, peer_index, _audio, _control), epoch) = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result")
            .expect("activation precedes wake_at");

        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        // Deliberately do not yield: the asynchronous expiry task must not be
        // the authority fence for publication.
        assert!(room
            .broadcast_protected_join_if_current(epoch, "protected", peer_index)
            .is_none());
        assert_eq!(room.roster_snapshot().revision, 0);
    }

    #[tokio::test]
    async fn protected_activation_is_hidden_from_every_room_projection() {
        let room = Arc::new(fresh_room());
        let (observer_id, _observer_index, mut observer_audio, _observer_control) =
            room.add_peer("observer".into(), 2).expect("observer joins");
        let mut roster = room.subscribe_roster();
        let pending = room
            .reserve_peer(Uuid::new_v4(), "protected".into(), 2)
            .expect("reserve protected peer");
        let schedule = ProtectedDeadlineSchedule::new(unix_time_seconds() + 60, None)
            .expect("future deadline");
        let ((hidden_id, _hidden_index, mut hidden_audio, mut hidden_control), _epoch) = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result")
            .expect("hidden activation succeeds");

        assert_eq!(
            room.roster_snapshot().peers,
            vec![RosterPeer {
                pubkey: "observer".into(),
                peer_index: 0,
            }]
        );
        assert_eq!(room.roster_snapshot().revision, 1);
        assert_eq!(room.peer_pubkeys(), vec![("observer".into(), 0)]);
        assert!(
            roster.try_recv().is_err(),
            "hidden activation emits no delta"
        );

        room.broadcast_frame(observer_id, Bytes::from_static(b"observer-frame"));
        assert!(
            hidden_audio.try_recv().is_err(),
            "hidden peer receives no media"
        );
        room.broadcast_frame(hidden_id, Bytes::from_static(b"hidden-frame"));
        assert!(
            observer_audio.try_recv().is_err(),
            "hidden peer cannot author visible media"
        );
        room.broadcast_control("control-probe".into());
        assert!(
            hidden_control.try_recv().is_err(),
            "hidden peer receives no control fan-out"
        );
    }

    #[tokio::test]
    async fn failed_publication_sink_leaves_protected_peer_hidden() {
        let room = Arc::new(fresh_room());
        let mut deltas = room.subscribe_roster();
        let pending = room
            .reserve_peer(Uuid::new_v4(), "protected".into(), 2)
            .expect("reserve protected peer");
        let schedule = ProtectedDeadlineSchedule::new(unix_time_seconds() + 60, None)
            .expect("future deadline");
        let ((_peer_id, peer_index, _audio, _control), epoch) = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result")
            .expect("hidden activation succeeds");

        assert!(room
            .publish_protected_join_if_current(
                epoch,
                "protected",
                peer_index,
                |_pubkey, _peer_index, _snapshot| None::<()>,
            )
            .is_none());
        assert!(room.peer_pubkeys().is_empty());
        assert_eq!(room.roster_snapshot().revision, 0);
        assert!(deltas.try_recv().is_err());
        assert!(room.remove_protected_epoch(epoch));
        assert!(deltas.try_recv().is_err(), "hidden cleanup emits no left");
    }

    #[test]
    fn in_flight_owner_claim_blocks_retirement_and_generation_reuse() {
        let manager = AudioRoomManager::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let first = RoomOwnerEpoch::new(RuntimeId([1; 32]), 7);
        let second = RoomOwnerEpoch::new(RuntimeId([1; 32]), 8);
        let claimed = manager
            .get_or_create_for_owner(community, channel, first, |_| {})
            .expect("first owner claim");

        assert!(matches!(
            manager.get_or_create_for_owner(community, channel, second, |_| {}),
            Err(AdmissionError::Ended)
        ));
        assert!(
            !manager.retire_exact_owner_if_empty(community, channel, &claimed, first, || {}),
            "the exact room cannot retire while its pre-reservation claim is live"
        );
        assert_eq!(
            manager.get(community, channel).unwrap().owner_epoch(),
            Some(first)
        );
    }

    #[tokio::test]
    async fn unpublished_expiry_emits_neither_left_nor_stale_join() {
        let room = Arc::new(fresh_room());
        let (_observer_id, _observer_index, _observer_audio, mut observer_control) =
            room.add_peer("observer".into(), 2).expect("observer joins");
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        let schedule = ProtectedDeadlineSchedule::new(unix_time_seconds() + 60, None)
            .expect("future deadline");
        let ((peer_id, peer_index, _audio, _control), epoch) = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result")
            .expect("protected peer activates");

        assert!(room.expire_protected_epoch(epoch));
        assert!(room
            .broadcast_protected_join_if_current(epoch, "protected", peer_index)
            .is_none());
        assert!(!room.peers.contains_key(&peer_id));

        let messages = std::iter::from_fn(|| observer_control.try_recv().ok())
            .filter_map(|message| match message {
                PeerCtrl::Json(json) => Some(json),
                PeerCtrl::Close => None,
            })
            .collect::<Vec<_>>();
        assert!(
            messages.is_empty(),
            "a hidden peer has no visible lifecycle"
        );
    }

    #[tokio::test]
    async fn protected_join_and_expiry_linearize_in_join_then_left_order() {
        let room = Arc::new(fresh_room());
        let (_observer_id, _observer_index, _observer_audio, mut observer_control) =
            room.add_peer("observer".into(), 2).expect("observer joins");
        let pending = room
            .reserve_peer(Uuid::new_v4(), "protected".into(), 2)
            .expect("reserve protected peer");
        let schedule = ProtectedDeadlineSchedule::new(unix_time_seconds() + 60, None)
            .expect("future deadline");
        let ((_peer_id, peer_index, _audio, _control), epoch) = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result")
            .expect("protected peer activates");

        assert!(room
            .broadcast_protected_join_if_current(epoch, "protected", peer_index)
            .is_some());
        assert!(room.expire_protected_epoch(epoch));

        let messages = std::iter::from_fn(|| observer_control.try_recv().ok())
            .filter_map(|message| match message {
                PeerCtrl::Json(json) => Some(json),
                PeerCtrl::Close => None,
            })
            .collect::<Vec<_>>();
        let joined = messages
            .iter()
            .position(|message| message.contains("\"joined\""))
            .expect("join publishes first");
        let left = messages
            .iter()
            .position(|message| message.contains("\"left\""))
            .expect("expiry publishes second");
        assert!(joined < left);
    }

    #[test]
    fn stale_owner_retirement_cannot_orphan_a_new_room_generation() {
        let manager = AudioRoomManager::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let first = RoomOwnerEpoch::new(RuntimeId([1; 32]), 7);
        let second = RoomOwnerEpoch::new(RuntimeId([1; 32]), 8);

        let old_claim = manager
            .get_or_create_for_owner(community, channel, first, |_| {})
            .expect("first owner claim");
        let old = old_claim.room();
        drop(old_claim);
        let replacement_claim = manager
            .get_or_create_for_owner(community, channel, second, |_| {})
            .expect("quiescent generation replacement");
        let replacement = replacement_claim.room();
        assert!(!Arc::ptr_eq(&old, &replacement));

        assert!(
            !manager.retire_exact_owner_if_empty(community, channel, &old, first, || panic!(
                "stale owner must not be released"
            ),)
        );
        assert!(replacement.add_peer("rejoin".into(), 2).is_ok());
    }

    #[test]
    fn owner_generation_cannot_change_while_room_has_a_live_claim() {
        let manager = AudioRoomManager::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let first = RoomOwnerEpoch::new(RuntimeId([1; 32]), 7);
        let second = RoomOwnerEpoch::new(RuntimeId([1; 32]), 8);
        let room = manager
            .get_or_create_for_owner(community, channel, first, |_| {})
            .expect("first owner claim");
        let _peer = room.add_peer("active".into(), 2).expect("live peer");

        assert!(matches!(
            manager.get_or_create_for_owner(community, channel, second, |_| {}),
            Err(AdmissionError::Ended)
        ));
        assert_eq!(
            manager.get(community, channel).unwrap().owner_epoch(),
            Some(first)
        );
    }

    #[test]
    fn exact_empty_retirement_releases_before_new_room_incarnation() {
        let manager = AudioRoomManager::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let old = manager.get_or_create(community, channel);
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = Arc::clone(&released);

        assert!(manager.retire_exact_if_empty(community, channel, &old, || {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        assert!(released.load(std::sync::atomic::Ordering::SeqCst));
        assert!(matches!(
            old.add_peer("stale".into(), 2),
            Err(AdmissionError::Ended)
        ));

        let new = manager.get_or_create(community, channel);
        assert!(!Arc::ptr_eq(&old, &new));
        assert!(new.add_peer("rejoin".into(), 2).is_ok());
    }

    #[test]
    fn concurrent_rejoin_waits_for_generation_fenced_owner_release() {
        let manager = Arc::new(AudioRoomManager::new());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let old = manager.get_or_create(community, channel);
        let (release_entered_tx, release_entered_rx) = std::sync::mpsc::channel();
        let (allow_release_tx, allow_release_rx) = std::sync::mpsc::channel();
        let retire_manager = Arc::clone(&manager);
        let retired_room = Arc::clone(&old);
        let retire = std::thread::spawn(move || {
            retire_manager.retire_exact_if_empty(community, channel, &retired_room, || {
                release_entered_tx.send(()).expect("report release fence");
                allow_release_rx.recv().expect("allow owner release");
            })
        });
        release_entered_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("retirement reaches owner release");

        let (rejoined_tx, rejoined_rx) = std::sync::mpsc::channel();
        let rejoin_manager = Arc::clone(&manager);
        let rejoin = std::thread::spawn(move || {
            let room = rejoin_manager.get_or_create(community, channel);
            rejoined_tx.send(room).expect("return new room");
        });
        assert!(
            rejoined_rx
                .recv_timeout(std::time::Duration::from_millis(25))
                .is_err(),
            "new room cannot publish before the old owner generation is released"
        );

        allow_release_tx.send(()).expect("finish owner release");
        assert!(retire.join().expect("retirement thread"));
        let new = rejoined_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("rejoin completes after release");
        rejoin.join().expect("rejoin thread");
        assert!(!Arc::ptr_eq(&old, &new));
    }

    #[test]
    fn expired_pending_authority_never_becomes_visible() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        assert!(pending
            .activate_protected_if(unix_time_seconds(), || true)
            .expect("activation result")
            .is_none());
        assert!(room.roster_snapshot().peers.is_empty());
        assert_eq!(room.roster_snapshot().revision, 0);
    }

    #[tokio::test]
    async fn expired_protected_peer_cannot_publish_joined_or_be_acknowledged() {
        let room = Arc::new(fresh_room());
        let (_legacy_id, _legacy_index, _legacy_audio, mut legacy_control) =
            room.add_peer("legacy".into(), 2).expect("legacy peer");
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        let schedule = ProtectedDeadlineSchedule::new(unix_time_seconds() + 3_600, None)
            .expect("future schedule");
        let ((peer_id, peer_index, _audio, _control), epoch) = pending
            .activate_protected_with_effects_if(
                schedule,
                ProtectedPeerEffects::new(CancellationToken::new()),
                || true,
            )
            .expect("activation result")
            .expect("protected peer activates before deadline");
        let mut peer = room.peers.get_mut(&peer_id).expect("protected peer");
        peer.authority_expires_at = Some(0);
        drop(peer);

        assert!(room
            .broadcast_protected_join_if_current(epoch, "protected", peer_index)
            .is_none());
        assert!(!room.peers.contains_key(&peer_id));
        let messages = std::iter::from_fn(|| legacy_control.try_recv().ok())
            .filter_map(|message| match message {
                PeerCtrl::Json(json) => Some(json),
                PeerCtrl::Close => None,
            })
            .collect::<Vec<_>>();
        assert!(
            messages.is_empty(),
            "an unpublished peer has no client history"
        );
    }

    #[test]
    fn failed_durable_visibility_gate_leaves_reservation_unpublished() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let reservation = room
            .reserve_peer(admission_id, "protected".into(), 2)
            .expect("reserve protected peer");
        let mut roster = room.subscribe_roster();

        let injected_visibility_result: Result<(), &'static str> = Err("injected failure");
        if injected_visibility_result.is_ok() {
            let _ = reservation.activate_protected_if(unix_time_seconds() + 3_600, || true);
        } else {
            drop(reservation);
        }

        assert!(room.peers.is_empty());
        assert!(room.roster_snapshot().peers.is_empty());
        assert_eq!(room.roster_snapshot().revision, 0);
        assert!(roster.try_recv().is_err());
    }

    /// First peer's `requested_version` becomes the room's pin; later peers
    /// requesting the same version are admitted normally.
    #[test]
    fn first_admit_pins_version_and_matching_admits_succeed() {
        let room = fresh_room();

        let first = room
            .add_peer("alice".to_string(), 2)
            .expect("first peer admits");
        let second = room
            .add_peer("bob".to_string(), 2)
            .expect("matching version admits");

        assert_eq!(room.peers.len(), 2);
        // peer_index allocation is monotonic from 0 inside a fresh room.
        assert_eq!(first.1, 0);
        assert_eq!(second.1, 1);
    }

    /// A peer requesting a different protocol version than the pinned one
    /// is refused with `VersionMismatch` and never appears in the peer map.
    #[test]
    fn admit_rejects_mismatched_version() {
        let room = fresh_room();
        let _ = room
            .add_peer("alice".to_string(), 2)
            .expect("first peer admits");

        let err = room
            .add_peer("bob".to_string(), 1)
            .expect_err("mismatched version must be rejected");
        match err {
            AdmissionError::VersionMismatch { pinned, requested } => {
                assert_eq!(pinned, 2);
                assert_eq!(requested, 1);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }

        // The rejected peer must not appear in the peer map, and the index
        // space must not have been consumed by the failed admit.
        assert_eq!(room.peers.len(), 1);
    }

    /// `add_peer` rejects requests after the room is marked ended even if the
    /// version matches.
    #[test]
    fn admit_after_mark_ended_returns_ended() {
        let room = fresh_room();
        assert!(room.mark_ended());
        let err = room
            .add_peer("alice".to_string(), 1)
            .expect_err("ended room must refuse");
        assert!(matches!(err, AdmissionError::Ended));
    }

    /// Per Max's review checklist: an empty-and-cleaned-up room (via the
    /// manager's `cleanup_if_empty`) becomes a fresh room on the next
    /// `get_or_create`, with no pin carried over.
    #[test]
    fn manager_cleanup_resets_version_pin() {
        let manager = AudioRoomManager::new();
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        let channel_id = Uuid::new_v4();

        let room1 = manager.get_or_create(community_id, channel_id);
        let (peer_id, _, _, _) = room1
            .add_peer("alice".to_string(), 2)
            .expect("first peer admits");
        // Last peer leaves and ends the room atomically.
        let (_, ended) = room1
            .remove_peer_and_check_ended(peer_id)
            .expect("peer existed");
        assert!(ended, "single-peer room should end on its last departure");
        let err = room1
            .add_peer("late-peer".to_string(), 2)
            .expect_err("a stale room handle must not admit after empty cleanup seals it");
        assert!(matches!(err, AdmissionError::Ended));
        assert!(manager.cleanup_if_empty(community_id, channel_id));

        // Next joiner with a different version on the same channel id gets a
        // brand-new room (no v=2 pin carried over from the prior generation).
        let room2 = manager.get_or_create(community_id, channel_id);
        let _ = room2
            .add_peer("bob".to_string(), 1)
            .expect("fresh room must accept any version");
    }

    #[test]
    fn manager_isolates_same_channel_uuid_across_communities() {
        let manager = AudioRoomManager::new();
        let channel_id = Uuid::new_v4();
        let community_a = CommunityId::from_uuid(Uuid::new_v4());
        let community_b = CommunityId::from_uuid(Uuid::new_v4());

        let room_a = manager.get_or_create(community_a, channel_id);
        assert!(Arc::ptr_eq(
            &manager
                .get_unambiguous_by_channel(channel_id)
                .expect("one matching room is unambiguous"),
            &room_a
        ));
        let room_b = manager.get_or_create(community_b, channel_id);

        assert!(
            !Arc::ptr_eq(&room_a, &room_b),
            "same channel UUID in two communities must create distinct rooms"
        );
        assert_eq!(room_a.community_id, community_a);
        assert_eq!(room_b.community_id, community_b);
        assert!(
            manager.get_unambiguous_by_channel(channel_id).is_none(),
            "community-free mesh lookup must fail closed on a UUID collision"
        );

        room_a
            .add_peer("alice".to_string(), 1)
            .expect("A peer admits");
        assert_eq!(room_a.peer_pubkeys(), vec![("alice".to_string(), 0)]);
        assert!(
            room_b.peer_pubkeys().is_empty(),
            "A room peers must not appear in B's same-UUID room"
        );
    }

    /// Peer-index reuse: after a peer leaves, their index is released; a new
    /// peer joining the same (still-pinned) room reuses the freed index.
    /// Version pin must persist across this reuse — the room generation
    /// hasn't ended.
    #[test]
    fn version_pin_persists_across_peer_churn() {
        let room = fresh_room();
        let (alice_id, alice_idx, _, _) =
            room.add_peer("alice".to_string(), 2).expect("alice admits");
        room.remove_peer(alice_id);
        // Room is non-empty thanks to nothing yet — wait, alice left and
        // nobody else is here. Add bob with the same version: should work.
        // Then add carol with a different version: should fail with the
        // *original* pin, even though alice already left.
        let (_, bob_idx, _, _) = room
            .add_peer("bob".to_string(), 2)
            .expect("bob admits at v=2");
        assert_eq!(
            bob_idx, alice_idx,
            "freed peer index should be recycled by the next admit",
        );
        let err = room
            .add_peer("carol".to_string(), 1)
            .expect_err("v=1 must still be refused — room is pinned v=2");
        assert!(matches!(
            err,
            AdmissionError::VersionMismatch {
                pinned: 2,
                requested: 1
            }
        ));
    }

    /// Per Sami/Perci's review: when a room is both at-capacity AND the
    /// joiner's protocol version doesn't match the pin, the error must be
    /// `Full` — not `VersionMismatch`. A client that couldn't get a seat
    /// either way shouldn't learn the room's pinned protocol version.
    /// This also pins the in-lock cap check (the old code's outside-lock
    /// cap check meant the version error could win in a race).
    #[test]
    fn admit_full_wins_over_version_mismatch() {
        let room = fresh_room();
        // Fill the room with v=2 peers right up to the soft cap.
        for i in 0..MAX_PEERS_PER_ROOM {
            room.add_peer(format!("peer-{i}"), 2)
                .expect("seed admit must succeed");
        }
        // Next joiner is BOTH over the cap AND requests the wrong version.
        let err = room
            .add_peer("over-cap-and-wrong-version".to_string(), 1)
            .expect_err("over-cap + wrong-version joiner must be rejected");
        assert!(
            matches!(err, AdmissionError::Full),
            "expected Full to win over VersionMismatch, got {err:?}",
        );
        // And the room state must be unchanged.
        assert_eq!(room.peers.len(), MAX_PEERS_PER_ROOM);
    }

    #[test]
    fn protected_reservation_is_invisible_until_activation() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let mut deltas = room.subscribe_roster();

        let pending = room
            .reserve_peer(admission_id, "alice".into(), 2)
            .expect("reservation succeeds");
        assert!(room.peer_pubkeys().is_empty());
        assert!(deltas.try_recv().is_err());
        assert!(
            !room.is_empty(),
            "pending attachment prevents room eviction"
        );

        let (peer_id, peer_index, ..) = pending.activate().expect("activation succeeds");
        assert_eq!(room.peer_pubkeys(), vec![("alice".into(), peer_index)]);
        let delta = deltas.try_recv().expect("activation publishes one delta");
        assert_eq!(delta.joined.unwrap().peer_index, peer_index);
        room.remove_peer(peer_id);
    }

    #[test]
    fn failed_commit_predicate_never_publishes_protected_presence() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let mut deltas = room.subscribe_roster();
        let pending = room
            .reserve_peer(admission_id, "alice".into(), 2)
            .expect("reservation succeeds");
        let reserved_index = pending.peer_index();

        assert!(pending
            .activate_if(|| false)
            .expect("predicate denial is not a room failure")
            .is_none());
        assert!(room.peer_pubkeys().is_empty());
        assert!(deltas.try_recv().is_err());

        let retry = room
            .reserve_peer(admission_id, "alice".into(), 2)
            .expect("denied activation releases the attempt");
        assert_eq!(retry.peer_index(), reserved_index);
    }

    #[test]
    fn dropped_reservation_is_compensated_and_retryable() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let first = room
            .reserve_peer(admission_id, "alice".into(), 2)
            .expect("first reservation succeeds");
        let first_index = first.peer_index();

        assert!(
            room.reserve_peer(admission_id, "alice".into(), 2).is_err(),
            "a live attempt cannot reserve twice"
        );
        drop(first);
        let retry = room
            .reserve_peer(admission_id, "alice".into(), 2)
            .expect("aborted attempt can retry");
        assert_eq!(retry.peer_index(), first_index);
    }

    #[test]
    fn active_attempt_cannot_create_duplicate_presence() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer(admission_id, "alice".into(), 2)
            .expect("reservation succeeds");
        let (peer_id, ..) = pending.activate().expect("activation succeeds");

        assert!(
            room.reserve_peer(admission_id, "alice".into(), 2).is_err(),
            "active attempt cannot attach twice"
        );
        room.remove_peer(peer_id);
        assert!(
            room.reserve_peer(admission_id, "alice".into(), 2).is_ok(),
            "disconnect removes the ephemeral attempt marker"
        );
    }

    #[test]
    fn owner_assigned_pending_index_is_reserved_and_released() {
        let room = Arc::new(fresh_room());
        let admission_id = Uuid::new_v4();
        let pending = room
            .reserve_peer_at_index(admission_id, "remote".into(), 2, 7)
            .expect("owner-assigned reservation succeeds");
        assert!(
            room.reserve_peer_at_index(Uuid::new_v4(), "other".into(), 2, 7)
                .is_err(),
            "pending indices cannot collide"
        );
        drop(pending);
        assert_eq!(
            room.reserve_peer_at_index(Uuid::new_v4(), "retry".into(), 2, 7)
                .expect("aborted index can be reused")
                .peer_index(),
            7
        );
    }

    #[test]
    fn owner_fanout_emits_once_per_remote_runtime() {
        let room = fresh_room();
        let (sender, ..) = room.add_peer("owner-local".into(), 2).unwrap();
        let remote_runtime = [0x42; 32];
        let (first_id, _, mut first_rx, _) = room
            .add_remote_peer("remote-a".into(), 2, remote_runtime)
            .unwrap();
        let (_, _, mut second_rx, _) = room
            .add_remote_peer("remote-b".into(), 2, remote_runtime)
            .unwrap();

        room.broadcast_frame(sender, Bytes::from_static(b"frame-one"));
        let first_count = usize::from(first_rx.try_recv().is_ok());
        let second_count = usize::from(second_rx.try_recv().is_ok());
        assert_eq!(first_count + second_count, 1);

        room.broadcast_frame(first_id, Bytes::from_static(b"remote-frame"));
        assert!(
            first_rx.try_recv().is_err(),
            "the author never hears itself"
        );
        assert!(
            second_rx.try_recv().is_ok(),
            "a same-runtime sibling still causes one pod fan-out"
        );

        room.remove_peer(first_id);
        room.broadcast_frame(sender, Bytes::from_static(b"frame-two"));
        assert!(second_rx.try_recv().is_ok());
    }
}
