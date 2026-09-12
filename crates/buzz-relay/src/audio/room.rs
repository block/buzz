//! Audio room: peer registry and frame fan-out.
//!
//! ```text
//! Client A → WS binary frame → Room::broadcast_frame → Client B, C, ...
//!                                                        (versioned peer prefix)
//! ```
//!
//! Frames are opaque Opus bytes — the relay never decodes audio. Protocol v3
//! adds the occupancy epoch after the peer index; v1/v2 keep their released
//! one-byte peer prefix. `try_send` is used throughout: real-time audio
//! tolerates drops, never queues.

use buzz_core::CommunityId;
use bytes::Bytes;
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
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
    /// Per-index reuse generation. Incremented each time this `peer_index` is
    /// (re)assigned to a new occupant, so a frame authored by a departed peer
    /// can be told apart from one authored by the peer that later reused the
    /// same index. Prefixed onto protocol-v3 relayed frames alongside
    /// `peer_index`.
    pub epoch: u8,
    /// Pinned wire version used to shape outbound relay prefixes without
    /// taking the admission mutex on the per-frame audio hot path.
    pub protocol_version: u8,
}

/// Control message for a single peer (separate from audio frames).
pub enum PeerCtrl {
    /// JSON control message (joined/left/speakers).
    Json(String),
    /// Graceful shutdown signal.
    Close,
}

/// Audio channel capacity per peer: 8 frames = 160ms at 20ms/frame.
const AUDIO_CHANNEL_CAPACITY: usize = 8;
/// Control channel capacity per peer: 32 slots — must never drop joined/left
/// messages, which are state-bearing (they maintain the client's peer_index →
/// pubkey map). Sized generously: even 30 simultaneous join/leave events fit.
const CTRL_CHANNEL_CAPACITY: usize = 32;

/// Defense-in-depth cap on peers per room. A room with N peers generates
/// N×(N−1) frame copies per 20ms tick — 25 peers = 600 copies/tick, which
/// is reasonable. Routing identities rotate through a larger 255-value pool.
const MAX_PEERS_PER_ROOM: usize = 25;

/// One authoritative owner-roster entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterPeer {
    /// Nostr pubkey hex.
    pub pubkey: String,
    /// Owner-assigned media routing index.
    pub peer_index: u8,
    /// Per-index reuse generation for `peer_index` (see [`AudioPeer::epoch`]).
    /// Carried in roster snapshots/deltas so receivers can fence media frames
    /// authored by a prior occupant of the same index.
    pub epoch: u8,
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

/// Successful local admission: peer ID, routing index, per-index epoch,
/// audio/control receivers, and the authoritative roster revision assigned to
/// the join.
pub type PeerAdmission = (
    Uuid,
    u8,
    u8,
    mpsc::Receiver<Bytes>,
    mpsc::Receiver<PeerCtrl>,
    u64,
);

/// Successful admission at an owner-assigned index: peer ID, per-index epoch,
/// audio/control receivers, and the roster revision. The routing index is
/// omitted because the caller supplied it.
pub type IndexedPeerAdmission = (
    Uuid,
    u8,
    mpsc::Receiver<Bytes>,
    mpsc::Receiver<PeerCtrl>,
    u64,
);

/// Reason a peer was refused entry to a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    /// The room has been ended (or is shutting down) and no longer admits peers.
    Ended,
    /// The room has hit its participant cap or the requested routing identity
    /// is already active.
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
    /// Next routing identity to probe. Allocation rotates through the complete
    /// 0..=254 space so a recently departed identity is not immediately reused,
    /// while long-running rooms never consume a finite lifetime admission
    /// budget.
    next_candidate: u8,
    /// Routing identities held by currently connected peers. Owner-assigned
    /// mesh identities share this set with locally allocated identities.
    active_indices: HashSet<u8>,
    /// Per-index reuse generation. `next_epoch_for(idx)` returns the epoch to
    /// stamp on the next occupant of `idx` and advances the counter, so every
    /// (re)assignment of an index gets a distinct, monotonically increasing
    /// (mod 256) epoch. A frame carrying a stale epoch for its index was
    /// authored by a departed occupant and is fenced by receivers.
    index_epochs: HashMap<u8, u8>,
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
}

impl AdmissionGuard {
    fn new() -> Self {
        Self {
            next_candidate: 0,
            active_indices: HashSet::new(),
            index_epochs: HashMap::new(),
            ended: false,
            pinned_version: None,
            roster_revision: 0,
        }
    }

    fn alloc(&mut self) -> Option<(u8, u8)> {
        for _ in 0..255 {
            let idx = self.next_candidate;
            self.next_candidate = if idx == 254 { 0 } else { idx + 1 };
            if self.active_indices.insert(idx) {
                return Some((idx, self.next_epoch_for(idx)));
            }
        }
        None
    }

    /// Epoch to stamp on the next occupant of `idx`, advancing the per-index
    /// counter. The first occupant of an index gets epoch 0; each later reuse
    /// increments (wrapping at 256, which is astronomically larger than the
    /// number of in-flight frames a stale occupant could have queued).
    fn next_epoch_for(&mut self, idx: u8) -> u8 {
        let slot = self.index_epochs.entry(idx).or_insert(0);
        let epoch = *slot;
        *slot = slot.wrapping_add(1);
        epoch
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
        if let Ok(mut g) = self.guard.lock() {
            g.ended = true;
            self.peers.is_empty()
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
    ) -> Result<PeerAdmission, AdmissionError> {
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
        let (peer_index, epoch) = g.alloc().ok_or(AdmissionError::Full)?;
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
                epoch,
                protocol_version: requested_version,
            },
        );
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let revision = g.roster_revision;
        let delta = RosterDelta {
            revision,
            joined: Some(RosterPeer {
                pubkey,
                peer_index,
                epoch,
            }),
            left: None,
        };
        let _ = self.roster_tx.send(delta);
        drop(g); // Release lock after ordered roster publication.
        Ok((peer_id, peer_index, epoch, audio_rx, ctrl_rx, revision))
    }

    /// Add a non-owner ingress peer at the index already allocated by the
    /// authoritative owner. No client-visible state is emitted before this
    /// succeeds, so a remote client has exactly one identity end-to-end.
    pub fn add_peer_at_index(
        &self,
        pubkey: String,
        requested_version: u8,
        peer_index: u8,
    ) -> Result<IndexedPeerAdmission, AdmissionError> {
        let mut g = self.guard.lock().map_err(|_| AdmissionError::Ended)?;
        if g.ended {
            return Err(AdmissionError::Ended);
        }
        if self.peers.len() >= MAX_PEERS_PER_ROOM || g.active_indices.contains(&peer_index) {
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
        g.active_indices.insert(peer_index);
        let epoch = g.next_epoch_for(peer_index);
        // Continue local allocation after the newest owner-assigned identity.
        // The cursor wraps, so a high mesh index cannot burn the lower space.
        g.next_candidate = if peer_index == 254 { 0 } else { peer_index + 1 };

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
                epoch,
                protocol_version: requested_version,
            },
        );
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let revision = g.roster_revision;
        let delta = RosterDelta {
            revision,
            joined: Some(RosterPeer {
                pubkey,
                peer_index,
                epoch,
            }),
            left: None,
        };
        let _ = self.roster_tx.send(delta);
        drop(g);
        Ok((peer_id, epoch, audio_rx, ctrl_rx, revision))
    }

    /// Remove a peer and release its routing identity for a later allocator
    /// rotation. Returns the ordered roster delta when the peer existed.
    pub fn remove_peer(&self, peer_id: Uuid) -> Option<RosterDelta> {
        let Ok(mut g) = self.guard.lock() else {
            return None;
        };
        let (_, peer) = self.peers.remove(&peer_id)?;
        g.active_indices.remove(&peer.peer_index);
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let delta = RosterDelta {
            revision: g.roster_revision,
            joined: None,
            left: Some(RosterPeer {
                pubkey: peer.pubkey,
                peer_index: peer.peer_index,
                epoch: peer.epoch,
            }),
        };
        let _ = self.roster_tx.send(delta.clone());
        drop(g);
        Some(delta)
    }

    /// Remove a peer AND atomically check if the room should end.
    /// If the room is now empty, sets `ended = true` under the same lock
    /// acquisition that removes the peer — no window for a concurrent
    /// `add_peer` to sneak in between removal and the ended flag.
    /// Returns `(roster_delta, should_auto_end)`.
    pub fn remove_peer_and_check_ended(&self, peer_id: Uuid) -> Option<(RosterDelta, bool)> {
        let mut g = self.guard.lock().ok()?;
        let (_, peer) = self.peers.remove(&peer_id)?;
        let peer_index = peer.peer_index;
        g.active_indices.remove(&peer_index);
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let delta = RosterDelta {
            revision: g.roster_revision,
            joined: None,
            left: Some(RosterPeer {
                pubkey: peer.pubkey,
                peer_index,
                epoch: peer.epoch,
            }),
        };
        // Only the first task to see empty + !ended wins the auto-end.
        // This prevents duplicate archive/48103 when two peers disconnect
        // simultaneously and both see is_empty() == true.
        let should_end = if !g.ended && self.peers.is_empty() {
            g.ended = true;
            true
        } else {
            false
        };
        let _ = self.roster_tx.send(delta.clone());
        drop(g);
        Some((delta, should_end))
    }

    /// Fan-out a binary frame to all peers except the sender. Protocol v3
    /// prepends the sender's `peer_index` and per-index `epoch`; v1/v2 retain
    /// their released one-byte `peer_index` prefix. Drops on full buffer —
    /// real-time audio never queues.
    pub fn broadcast_frame(&self, sender_id: Uuid, frame: Bytes) {
        let (sender_index, sender_epoch, protocol_version) = match self.peers.get(&sender_id) {
            Some(p) => (p.peer_index, p.epoch, p.protocol_version),
            None => return,
        };

        let prefix_len = if protocol_version >= 3 { 2 } else { 1 };
        let mut prefixed = bytes::BytesMut::with_capacity(prefix_len + frame.len());
        prefixed.extend_from_slice(&[sender_index]);
        if protocol_version >= 3 {
            prefixed.extend_from_slice(&[sender_epoch]);
        }
        prefixed.extend_from_slice(&frame);
        let prefixed = prefixed.freeze();

        for entry in self.peers.iter() {
            if *entry.key() == sender_id {
                continue;
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
        for entry in self.peers.iter() {
            if entry.peer_index == author_index {
                continue;
            }
            let _ = entry.audio_tx.try_send(prefixed.clone());
        }
    }

    /// Send a JSON control message to all peers via the control channel.
    /// Separate from audio so control is never starved by audio backpressure.
    /// Control messages (joined/left) are state-bearing — the client's
    /// peer_index→pubkey map depends on receiving every one. Saturation is
    /// therefore terminal for that receiver: dropping its sender closes the
    /// queue, forcing a reconnect with a fresh authoritative admission snapshot.
    pub fn broadcast_control(&self, json: String) {
        for mut entry in self.peers.iter_mut() {
            if entry
                .ctrl_tx
                .try_send(PeerCtrl::Json(json.clone()))
                .is_err()
            {
                let (replacement_tx, replacement_rx) = mpsc::channel(1);
                drop(replacement_rx);
                let old_tx = std::mem::replace(&mut entry.ctrl_tx, replacement_tx);
                drop(old_tx);
                tracing::warn!(
                    peer_id = %entry.key(),
                    "control channel full — closing receiver for authoritative roster resync"
                );
            }
        }
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
        let g = self.guard.lock().unwrap_or_else(|e| e.into_inner());
        let mut peers = self
            .peers
            .iter()
            .map(|e| RosterPeer {
                pubkey: e.pubkey.clone(),
                peer_index: e.peer_index,
                epoch: e.epoch,
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
        self.peers
            .iter()
            .map(|e| (e.pubkey.clone(), e.peer_index))
            .collect()
    }

    /// True if no peers remain in the room.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

/// Global registry of active audio rooms.
pub struct AudioRoomManager {
    rooms: DashMap<(CommunityId, Uuid), Arc<Room>>,
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

        let (remote_id, _epoch, _audio, _ctrl, _revision) = room
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
    fn active_owner_assigned_index_cannot_be_readmitted() {
        let room = fresh_room();
        let (_remote_id, _epoch, _audio, _ctrl, _revision) = room
            .add_peer_at_index("remote".into(), 2, 7)
            .expect("owner-assigned index admits");

        let result = room.add_peer_at_index("replacement".into(), 2, 7);
        assert!(
            matches!(result, Err(AdmissionError::Full)),
            "an active owner-assigned index must not identify another socket"
        );
    }

    #[test]
    fn owner_assigned_high_index_does_not_exhaust_local_allocation() {
        let room = fresh_room();
        let (remote_id, _epoch, _audio, _ctrl, _revision) = room
            .add_peer_at_index("remote".into(), 2, 254)
            .expect("high owner-assigned index admits");
        room.remove_peer(remote_id).expect("remote peer leaves");

        let (_local_id, local_index, ..) = room
            .add_peer("local".into(), 2)
            .expect("a high owner index must not burn lower routing identities");
        assert_eq!(local_index, 0);
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
                epoch: 0,
            }]
        );
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
        let (peer_id, _, _, _, _, _) = room1
            .add_peer("alice".to_string(), 2)
            .expect("first peer admits");
        // Last peer leaves and ends the room atomically.
        let (_, ended) = room1
            .remove_peer_and_check_ended(peer_id)
            .expect("peer existed");
        assert!(ended, "single-peer room should end on its last departure");
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

    /// Peer indices rotate instead of being immediately reused, which gives
    /// queued media and cleanup work time to drain without imposing a lifetime
    /// admission budget on the room.
    #[test]
    fn peer_indices_are_not_reused_within_a_room_generation() {
        let room = fresh_room();
        let (alice_id, alice_idx, _, _, _, _) =
            room.add_peer("alice".to_string(), 2).expect("alice admits");
        let (_keeper_id, keeper_idx, _, _, _, _) = room
            .add_peer("keeper".to_string(), 2)
            .expect("keeper admits");

        room.remove_peer(alice_id).expect("alice leaves");
        let (_, bob_idx, _, _, _, _) = room
            .add_peer("bob".to_string(), 2)
            .expect("bob admits at v=2");

        assert_eq!(alice_idx, 0);
        assert_eq!(keeper_idx, 1);
        assert_eq!(
            bob_idx, 2,
            "a departed peer index must not be immediately reused",
        );
    }

    /// A reused peer index carries a distinct epoch from its prior occupant,
    /// so receivers can fence media authored before the reassignment. Rotation
    /// still holds (the index is not immediately reused), but even after the
    /// allocator wraps back, the epoch advances.
    #[test]
    fn reused_peer_index_gets_a_distinct_epoch() {
        let room = fresh_room();
        // First occupant of index 0 gets epoch 0.
        let (alice_id, alice_index, alice_epoch, ..) =
            room.add_peer("alice".into(), 2).expect("alice admits");
        assert_eq!(alice_index, 0);
        assert_eq!(alice_epoch, 0);
        room.remove_peer(alice_id).expect("alice leaves");

        // Force the allocator cursor back to 0 so the next admit reuses index 0.
        // A single owner-assigned admit at 254 sets next_candidate to wrap to 0.
        let (_high_id, high_epoch, ..) = room
            .add_peer_at_index("high".into(), 2, 254)
            .expect("high owner index admits");
        assert_eq!(high_epoch, 0, "index 254 is a first occupant");

        let (_bob_id, bob_index, bob_epoch, ..) =
            room.add_peer("bob".into(), 2).expect("bob admits");
        assert_eq!(bob_index, 0, "cursor wrapped to reuse index 0");
        assert_eq!(
            bob_epoch, 1,
            "reused index 0 must advance its epoch past alice's"
        );
    }

    /// The epoch stamped on a fanned-out v3 frame matches the sender's current
    /// per-index epoch. Released v2 retains its one-byte prefix so old v2
    /// clients cannot share a room with v3 clients while decoding different
    /// binary layouts under the same negotiated version.
    #[test]
    fn broadcast_frame_uses_the_prefix_for_the_pinned_version() {
        let v2_room = fresh_room();
        let (v2_sender_id, v2_sender_index, ..) = v2_room
            .add_peer("v2-sender".into(), 2)
            .expect("v2 sender admits");
        let (_v2_listener_id, _, _, mut v2_listener_rx, _, _) = v2_room
            .add_peer("v2-listener".into(), 2)
            .expect("v2 listener admits");
        v2_room.broadcast_frame(v2_sender_id, Bytes::from_static(&[0xAB, 0xCD]));
        let v2_frame = v2_listener_rx
            .try_recv()
            .expect("v2 listener receives frame");
        assert_eq!(&v2_frame[..1], &[v2_sender_index]);
        assert_eq!(&v2_frame[1..], &[0xAB, 0xCD]);

        let v3_room = fresh_room();
        let (v3_sender_id, v3_sender_index, v3_sender_epoch, ..) = v3_room
            .add_peer("v3-sender".into(), 3)
            .expect("v3 sender admits");
        let (_v3_listener_id, _, _, mut v3_listener_rx, _, _) = v3_room
            .add_peer("v3-listener".into(), 3)
            .expect("v3 listener admits");
        v3_room.broadcast_frame(v3_sender_id, Bytes::from_static(&[0xAB, 0xCD]));
        let v3_frame = v3_listener_rx
            .try_recv()
            .expect("v3 listener receives frame");
        assert_eq!(&v3_frame[..2], &[v3_sender_index, v3_sender_epoch]);
        assert_eq!(&v3_frame[2..], &[0xAB, 0xCD]);
    }

    #[test]
    fn one_seated_peer_survives_more_than_index_space_reconnects() {
        let room = fresh_room();
        let (_keeper_id, keeper_index, ..) =
            room.add_peer("keeper".into(), 2).expect("keeper admits");

        for cycle in 0..300 {
            let (peer_id, peer_index, ..) = room
                .add_peer(format!("reconnect-{cycle}"), 2)
                .unwrap_or_else(|error| panic!("cycle {cycle} must admit: {error:?}"));
            assert_ne!(peer_index, keeper_index);
            room.remove_peer(peer_id).expect("reconnecting peer leaves");
        }
    }

    /// Protocol version pinning persists across peer churn even while routing
    /// identities rotate for later reuse.
    #[test]
    fn version_pin_persists_across_peer_churn() {
        let room = fresh_room();
        let (alice_id, _, _, _, _, _) =
            room.add_peer("alice".to_string(), 2).expect("alice admits");
        let (_keeper_id, _, _, _, _, _) = room
            .add_peer("keeper".to_string(), 2)
            .expect("keeper admits");
        room.remove_peer(alice_id);

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
}
