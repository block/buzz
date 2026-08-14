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
use bytes::Bytes;
use dashmap::DashMap;
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
}

impl AdmissionGuard {
    fn new() -> Self {
        Self {
            next_fresh: 0,
            free: Vec::new(),
            ended: false,
            pinned_version: None,
            roster_revision: 0,
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
    ) -> Result<(Uuid, u8, mpsc::Receiver<Bytes>, mpsc::Receiver<PeerCtrl>), AdmissionError> {
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

    /// Remove a peer and recycle its index.
    pub fn remove_peer(&self, peer_id: Uuid) {
        let Ok(mut g) = self.guard.lock() else {
            return;
        };
        if let Some((_, peer)) = self.peers.remove(&peer_id) {
            g.release(peer.peer_index);
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
            drop(g);
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
        let peer_index = peer.peer_index;
        g.release(peer_index);
        g.roster_revision = g.roster_revision.wrapping_add(1);
        let delta = RosterDelta {
            revision: g.roster_revision,
            joined: None,
            left: Some(RosterPeer {
                pubkey: peer.pubkey,
                peer_index,
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
        let _ = self.roster_tx.send(delta);
        drop(g);
        Some((peer_index, should_end))
    }

    /// Fan-out a binary frame to all peers except the sender.
    /// Prepends the sender's `peer_index` as a 1-byte prefix.
    /// Drops on full buffer — real-time audio never queues.
    pub fn broadcast_frame(&self, sender_id: Uuid, frame: Bytes) {
        let sender_index = match self.peers.get(&sender_id) {
            Some(p) => p.peer_index,
            None => return,
        };

        // Prepend peer_index as 1-byte header.
        let mut prefixed = bytes::BytesMut::with_capacity(1 + frame.len());
        prefixed.extend_from_slice(&[sender_index]);
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
    /// peer_index→pubkey map depends on receiving every one. The channel is
    /// sized generously (32 slots) so drops should never happen in practice;
    /// if they do, we log a warning so the issue is visible.
    pub fn broadcast_control(&self, json: String) {
        for entry in self.peers.iter() {
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
}
