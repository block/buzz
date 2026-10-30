//! In-memory mesh membership table fed by Redis seed records and gossip.
//!
//! This module implements the relay-facing [`RelayMeshMembership`] seam. It is
//! deliberately incapable of electing session owners: peers here are dial/routing
//! hints only, and liveness disagreement never performs takeover.
//!
//! **Record lifetime.** Every boot gets a fresh runtime id, so a restarted pod
//! leaves its old record behind. A live runtime refreshes `heartbeat_millis`
//! every gossip tick; a record whose heartbeat is older than the peer-record
//! TTL belongs to a dead process. Such records are rejected on arrival, left
//! out of digests, deltas, and dial candidates, and pruned from the table, so
//! scuttlebutt can no longer keep a dead runtime alive fleet-wide. The TTL
//! never prunes or hides a peer we hold a live connection to: the transport
//! itself proves the runtime is alive. (Phi suspicion still filters it from
//! [`RelayMeshMembership::peers`] once its gossip stalls.) The runtime keeps
//! `Connected` in step with its transport map, writing it under the map's lock
//! and re-syncing it every reconcile pass, so a closed connection cannot leave
//! a dead runtime exempt from expiry.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use crate::gossip::{system_time_from_millis, GossipRecord, PhiAccrual};
use crate::registry::ReadyRecord;
use crate::status::{ConnectionState, MeshCounters, MeshPeerCounters, MeshPeerStatus, MeshStatus};
use crate::telemetry::PeerTableStats;
use crate::{PeerInfo, RelayMeshMembership, RuntimeId};

pub const DEFAULT_PHI_SUSPECT_THRESHOLD: f64 = 8.0;

/// How long a peer record survives without a fresher heartbeat.
///
/// Live runtimes refresh every [`crate::runtime::DEFAULT_GOSSIP_INTERVAL`]
/// (2s), and the ready registry expires a silent runtime after 45s. Five
/// minutes sits well past both, so multi-hop gossip delay and cross-pod clock
/// skew *under the TTL* never expire a live runtime. A pod whose clock is
/// behind by more than the TTL has all of its gossip rejected fleet-wide; it
/// stays reachable through its Redis ready record and any live connection.
/// `buzz_mesh_peer_records_expired_total{reason="rejected"}` that keeps
/// climbing outside a rollout is the signal for that skew.
pub const DEFAULT_PEER_RECORD_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
struct PeerState {
    record: GossipRecord,
    phi: PhiAccrual,
    connection_state: ConnectionState,
    counters: MeshPeerCounters,
}

/// Thread-safe membership view consumed by the relay.
#[derive(Clone, Debug)]
pub struct MeshMembership {
    local_runtime_id: RuntimeId,
    local_record: Arc<RwLock<GossipRecord>>,
    peers: Arc<RwLock<HashMap<RuntimeId, PeerState>>>,
    draining: Arc<AtomicBool>,
    stale_generation_rejections: Arc<AtomicU64>,
    foreign_relay_rejections: Arc<AtomicU64>,
    /// The relay identity ready records must be attested by. All pods in one
    /// deployment share the relay signing key, so a valid seed is one signed
    /// by *our* key — "signed by some relay key" is possession, not
    /// authorization. `None` (never set) rejects every ready record: the
    /// unanchored state is fail-closed, not accept-any.
    expected_relay_pubkey: Option<String>,
    phi_suspect_threshold: f64,
    /// Records whose heartbeat is older than this are dead; see the module docs.
    peer_record_ttl: Duration,
}

impl MeshMembership {
    pub fn new(local_record: GossipRecord) -> Self {
        Self {
            local_runtime_id: local_record.runtime_id,
            local_record: Arc::new(RwLock::new(local_record)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            draining: Arc::new(AtomicBool::new(false)),
            stale_generation_rejections: Arc::new(AtomicU64::new(0)),
            foreign_relay_rejections: Arc::new(AtomicU64::new(0)),
            expected_relay_pubkey: None,
            phi_suspect_threshold: DEFAULT_PHI_SUSPECT_THRESHOLD,
            peer_record_ttl: DEFAULT_PEER_RECORD_TTL,
        }
    }

    /// Anchor ready-record acceptance to this relay identity (hex pubkey).
    /// Without an anchor, [`Self::apply_ready_records`] admits nothing.
    pub fn with_expected_relay_pubkey(mut self, pubkey_hex: String) -> Self {
        self.expected_relay_pubkey = Some(pubkey_hex);
        self
    }

    pub fn with_phi_suspect_threshold(mut self, threshold: f64) -> Self {
        self.phi_suspect_threshold = threshold;
        self
    }

    /// Override [`DEFAULT_PEER_RECORD_TTL`].
    pub fn with_peer_record_ttl(mut self, ttl: Duration) -> Self {
        self.peer_record_ttl = ttl;
        self
    }

    /// Whether `record`'s heartbeat is older than the peer-record TTL at
    /// `now_millis`. A heartbeat ahead of our clock (skew) is fresh.
    fn is_expired(&self, record: &GossipRecord, now_millis: u64) -> bool {
        let age = Duration::from_millis(now_millis.saturating_sub(record.heartbeat_millis));
        age > self.peer_record_ttl
    }

    /// Whether `peer` still belongs in the table: its heartbeat is fresh, or
    /// we hold a live connection to it. Governs prune, admission, and
    /// routing. Digests, deltas, and dials use [`Self::is_expired`] alone so a
    /// stale heartbeat is never re-gossiped.
    fn is_live(&self, peer: &PeerState, now_millis: u64) -> bool {
        peer.connection_state == ConnectionState::Connected
            || !self.is_expired(&peer.record, now_millis)
    }

    pub fn local_record(&self) -> GossipRecord {
        self.local_record
            .read()
            .expect("local record lock poisoned")
            .clone()
    }

    /// Apply Redis bootstrap records. Existing gossip records win when they are
    /// newer; ready-registry records enter as version 1 hints.
    ///
    /// A record is admitted only when its `relay_pubkey` matches the expected
    /// relay identity AND its attestation signature verifies. Matching first
    /// makes the authorization question explicit: a record signed by a key we
    /// don't recognize is foreign no matter how valid its signature is.
    pub fn apply_ready_records(&self, records: impl IntoIterator<Item = ReadyRecord>) {
        for ready in records {
            if ready.runtime_id == self.local_runtime_id {
                continue;
            }
            match self.expected_relay_pubkey.as_deref() {
                Some(expected) if ready.relay_pubkey == expected => {}
                anchor => {
                    self.foreign_relay_rejections
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        runtime_id = %ready.runtime_id,
                        record_relay_pubkey = %ready.relay_pubkey,
                        anchored = anchor.is_some(),
                        "mesh membership rejected ready seed not attested by expected relay identity"
                    );
                    continue;
                }
            }
            if let Err(err) = ready.verify_attestation() {
                tracing::warn!(
                    runtime_id = %ready.runtime_id,
                    %err,
                    "mesh membership rejected unauthenticated ready seed"
                );
                continue;
            }
            let mut record =
                GossipRecord::new(ready.runtime_id, ready.endpoint_addrs, ready.proto_version);
            record.capabilities = ready.capabilities;
            self.apply_gossip_record(record);
        }
    }

    /// Apply a gossiped record if it is newer than the local copy and its
    /// heartbeat has not expired.
    ///
    /// A record learned here is a dial hint, not a connection: a new peer
    /// enters `Disconnected`, and an update never changes connection state.
    /// Only the runtime's real connect/disconnect paths do that.
    ///
    /// An existing entry that is no longer live (expired, not yet pruned, and
    /// not connected) counts as absent: any fresh record replaces it whatever
    /// its version, so a live peer's v1 Redis hint can re-admit it before the
    /// next prune.
    pub fn apply_gossip_record(&self, record: GossipRecord) -> bool {
        if record.runtime_id == self.local_runtime_id {
            return false;
        }
        let now_millis = crate::gossip::now_millis();
        if self.is_expired(&record, now_millis) {
            crate::telemetry::record_expired("rejected", 1);
            return false;
        }

        let heartbeat = system_time_from_millis(record.heartbeat_millis);
        let mut peers = self.peers.write().expect("membership lock poisoned");
        match peers.get_mut(&record.runtime_id) {
            Some(peer)
                if self.is_live(peer, now_millis) && record.version <= peer.record.version =>
            {
                false
            }
            Some(peer) => {
                if !self.is_live(peer, now_millis) {
                    // The old heartbeat history belongs to the dead stretch.
                    peer.phi = PhiAccrual::default();
                }
                peer.record = record;
                peer.phi.observe(heartbeat);
                true
            }
            None => {
                let mut phi = PhiAccrual::default();
                phi.observe(heartbeat);
                peers.insert(
                    record.runtime_id,
                    PeerState {
                        counters: MeshPeerCounters {
                            runtime_id: record.runtime_id.to_string(),
                            ..MeshPeerCounters::default()
                        },
                        record,
                        phi,
                        connection_state: ConnectionState::Disconnected,
                    },
                );
                true
            }
        }
    }

    /// The current record for `runtime_id` if it is still worth dialing:
    /// present, unexpired, and not draining. Dials re-read this before every
    /// attempt, because a reconcile pass's candidate snapshot goes stale while
    /// earlier dials are pending.
    pub fn dial_record(&self, runtime_id: RuntimeId) -> Option<GossipRecord> {
        let now_millis = crate::gossip::now_millis();
        self.peers
            .read()
            .expect("membership lock poisoned")
            .get(&runtime_id)
            .filter(|peer| !peer.record.draining && !self.is_expired(&peer.record, now_millis))
            .map(|peer| peer.record.clone())
    }

    /// Re-derive `Connected` from the transport: a peer in `connected` is
    /// `Connected`; a `Connected` peer missing from it is `Disconnected`.
    /// `Connecting` is left alone. Heals any write the event-driven updates
    /// missed (for example a connection installed after its record was pruned
    /// and later re-learned from gossip).
    pub(crate) fn sync_connection_states(&self, connected: impl Fn(RuntimeId) -> bool) {
        let mut peers = self.peers.write().expect("membership lock poisoned");
        for (runtime_id, peer) in peers.iter_mut() {
            if connected(*runtime_id) {
                peer.connection_state = ConnectionState::Connected;
            } else if peer.connection_state == ConnectionState::Connected {
                peer.connection_state = ConnectionState::Disconnected;
            }
        }
    }

    pub fn mark_connection_state(&self, runtime_id: RuntimeId, state: ConnectionState) {
        if let Some(peer) = self
            .peers
            .write()
            .expect("membership lock poisoned")
            .get_mut(&runtime_id)
        {
            peer.connection_state = state;
        }
    }

    pub fn update_local<F>(&self, update: F) -> GossipRecord
    where
        F: FnOnce(&mut GossipRecord),
    {
        let mut local = self
            .local_record
            .write()
            .expect("local record lock poisoned");
        update(&mut local);
        local.version = local.version.saturating_add(1);
        local.heartbeat_millis = crate::gossip::now_millis();
        local.clone()
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Relaxed)
    }

    /// Whether `runtime_id` is present in the (attested) peer table. Used by
    /// the runtime's accept loop to gate inbound connections — a dialability
    /// hint, never an ownership statement.
    pub fn has_peer(&self, runtime_id: RuntimeId) -> bool {
        self.peers
            .read()
            .expect("membership lock poisoned")
            .get(&runtime_id)
            .is_some_and(|peer| self.is_live(peer, crate::gossip::now_millis()))
    }

    /// All live gossip records (local + unexpired peers), for reconcile/dial
    /// decisions and gossip digests/deltas. Expired records are left out even
    /// before [`Self::prune_expired`] removes them, so they are never dialed
    /// or re-gossiped.
    pub fn records(&self) -> Vec<GossipRecord> {
        self.records_at(crate::gossip::now_millis())
    }

    fn records_at(&self, now_millis: u64) -> Vec<GossipRecord> {
        let mut records: Vec<GossipRecord> = self
            .peers
            .read()
            .expect("membership lock poisoned")
            .values()
            .filter(|peer| !self.is_expired(&peer.record, now_millis))
            .map(|peer| peer.record.clone())
            .collect();
        records.push(self.local_record());
        records
    }

    /// Drop every peer whose heartbeat has expired, except peers we are still
    /// connected to. Returns how many went.
    pub fn prune_expired(&self) -> usize {
        self.prune_expired_at(crate::gossip::now_millis())
    }

    fn prune_expired_at(&self, now_millis: u64) -> usize {
        let mut peers = self.peers.write().expect("membership lock poisoned");
        let before = peers.len();
        peers.retain(|_, peer| self.is_live(peer, now_millis));
        let pruned = before - peers.len();
        drop(peers);
        crate::telemetry::record_expired("pruned", pruned as u64);
        pruned
    }

    /// Publish peer-table size and staleness gauges.
    pub(crate) fn publish_table_metrics(&self) {
        self.table_stats(SystemTime::now()).publish();
    }

    fn table_stats(&self, now: SystemTime) -> PeerTableStats {
        let mut stats = PeerTableStats::default();
        for status in self.peer_statuses(now) {
            let heartbeat = system_time_from_millis(status.last_heartbeat_millis);
            let age = now.duration_since(heartbeat).unwrap_or_default();
            stats.record(status.connection_state, status.draining, age);
        }
        stats
    }

    /// Scuttlebutt digest over every record this runtime knows (local + peers).
    pub fn digest(&self) -> crate::gossip::GossipMessage {
        let mut entries: Vec<_> = self
            .records()
            .into_iter()
            .map(|record| crate::gossip::GossipDigestEntry {
                runtime_id: record.runtime_id,
                version: record.version,
            })
            .collect();
        entries.sort_by_key(|entry| entry.runtime_id.to_hex());
        crate::gossip::GossipMessage::Digest {
            version: crate::gossip::GOSSIP_PAYLOAD_VERSION,
            entries,
        }
    }

    /// Records the remote digest is missing or behind on.
    pub fn delta_for(
        &self,
        digest: &[crate::gossip::GossipDigestEntry],
    ) -> crate::gossip::GossipMessage {
        let remote: std::collections::HashMap<_, _> = digest
            .iter()
            .map(|entry| (entry.runtime_id, entry.version))
            .collect();
        let mut records: Vec<_> = self
            .records()
            .into_iter()
            .filter(|record| {
                remote
                    .get(&record.runtime_id)
                    .is_none_or(|version| *version < record.version)
            })
            .collect();
        records.sort_by_key(|record| record.runtime_id.to_hex());
        crate::gossip::GossipMessage::Delta {
            version: crate::gossip::GOSSIP_PAYLOAD_VERSION,
            records,
        }
    }

    pub fn record_stream_opened(&self, runtime_id: RuntimeId) {
        self.update_peer_counters(runtime_id, |c| {
            c.streams_opened = c.streams_opened.saturating_add(1)
        });
    }

    pub fn record_stream_received(&self, runtime_id: RuntimeId) {
        self.update_peer_counters(runtime_id, |c| {
            c.streams_received = c.streams_received.saturating_add(1)
        });
    }

    pub fn record_datagram_sent(&self, runtime_id: RuntimeId) {
        self.update_peer_counters(runtime_id, |c| {
            c.datagrams_sent = c.datagrams_sent.saturating_add(1)
        });
    }

    pub fn record_datagram_received(&self, runtime_id: RuntimeId) {
        self.update_peer_counters(runtime_id, |c| {
            c.datagrams_received = c.datagrams_received.saturating_add(1)
        });
    }

    pub fn record_gossip_frame_sent(&self, runtime_id: RuntimeId) {
        self.update_peer_counters(runtime_id, |c| {
            c.gossip_frames_sent = c.gossip_frames_sent.saturating_add(1)
        });
    }

    pub fn record_gossip_frame_received(&self, runtime_id: RuntimeId) {
        self.update_peer_counters(runtime_id, |c| {
            c.gossip_frames_received = c.gossip_frames_received.saturating_add(1)
        });
    }

    pub fn record_stale_generation_rejection(&self, runtime_id: Option<RuntimeId>) {
        self.stale_generation_rejections
            .fetch_add(1, Ordering::Relaxed);
        if let Some(runtime_id) = runtime_id {
            self.update_peer_counters(runtime_id, |c| {
                c.stale_generation_rejections = c.stale_generation_rejections.saturating_add(1)
            });
        }
    }

    pub fn status(&self) -> MeshStatus {
        let now = SystemTime::now();
        let local = self.local_record();
        let mut peers = self.peer_statuses(now);
        peers.sort_by(|a, b| a.runtime_id.cmp(&b.runtime_id));
        let counters = MeshCounters {
            stale_generation_rejections: self.stale_generation_rejections.load(Ordering::Relaxed),
            foreign_relay_rejections: self.foreign_relay_rejections.load(Ordering::Relaxed),
            peers: peers.iter().map(|peer| peer.counters.clone()).collect(),
        };
        MeshStatus {
            enabled: true,
            local_runtime_id: local.runtime_id.to_string(),
            draining: self.is_draining(),
            peer_count: peers.len(),
            peers,
            counters,
        }
    }

    fn update_peer_counters<F>(&self, runtime_id: RuntimeId, update: F)
    where
        F: FnOnce(&mut MeshPeerCounters),
    {
        if let Some(peer) = self
            .peers
            .write()
            .expect("membership lock poisoned")
            .get_mut(&runtime_id)
        {
            update(&mut peer.counters);
        }
    }

    fn peer_statuses(&self, now: SystemTime) -> Vec<MeshPeerStatus> {
        self.peers
            .read()
            .expect("membership lock poisoned")
            .values()
            .map(|peer| {
                let phi = peer.phi.phi_at(now);
                let connection_state = if phi.is_some_and(|p| p >= self.phi_suspect_threshold) {
                    ConnectionState::Suspect
                } else {
                    peer.connection_state
                };
                MeshPeerStatus {
                    runtime_id: peer.record.runtime_id.to_string(),
                    endpoint_addrs: peer.record.endpoint_addrs.clone(),
                    proto_version: peer.record.proto_version,
                    draining: peer.record.draining,
                    connection_state,
                    phi,
                    load: peer.record.load,
                    record_version: peer.record.version,
                    last_heartbeat_millis: peer.record.heartbeat_millis,
                    counters: peer.counters.clone(),
                }
            })
            .collect()
    }
}

impl RelayMeshMembership for MeshMembership {
    fn peers(&self) -> Vec<PeerInfo> {
        let now = SystemTime::now();
        let now_millis = crate::gossip::now_millis();
        self.peers
            .read()
            .expect("membership lock poisoned")
            .values()
            .filter(|peer| self.is_live(peer, now_millis))
            .filter_map(|peer| {
                let phi = peer.phi.phi_at(now);
                if phi.is_some_and(|p| p >= self.phi_suspect_threshold) {
                    return None;
                }
                Some(PeerInfo {
                    runtime_id: peer.record.runtime_id,
                    draining: peer.record.draining,
                    phi,
                    load: peer.record.load,
                })
            })
            .collect()
    }

    fn local_runtime_id(&self) -> RuntimeId {
        self.local_runtime_id
    }

    fn begin_drain(&self) {
        self.draining.store(true, Ordering::Relaxed);
        self.update_local(|record| record.draining = true);
    }

    fn peer_has_capability(&self, runtime_id: RuntimeId, capability: &str) -> bool {
        self.peers
            .read()
            .expect("membership lock poisoned")
            .get(&runtime_id)
            .filter(|peer| self.is_live(peer, crate::gossip::now_millis()))
            .is_some_and(|peer| peer.record.capabilities.iter().any(|c| c == capability))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gossip::now_millis;

    fn rid(byte: u8) -> RuntimeId {
        RuntimeId([byte; 32])
    }

    /// A peer record whose heartbeat is `age_secs` old.
    fn record(byte: u8, version: u64, age_secs: u64) -> GossipRecord {
        GossipRecord {
            runtime_id: rid(byte),
            endpoint_addrs: vec![format!("127.0.0.{byte}:3478")],
            proto_version: 1,
            load: 0.25,
            draining: false,
            capabilities: vec!["reliable-stream".to_string()],
            version,
            heartbeat_millis: now_millis() - age_secs * 1_000,
        }
    }

    fn relay_keys() -> nostr::Keys {
        nostr::Keys::generate()
    }

    fn ready_record_signed(byte: u8, endpoint_addr: &str, keys: &nostr::Keys) -> ReadyRecord {
        ReadyRecord::new(rid(byte), keys, vec![endpoint_addr.into()], 1, vec![])
    }

    #[test]
    fn ready_records_seed_peers_but_skip_self() {
        let keys = relay_keys();
        let membership = MeshMembership::new(record(1, 1, 1))
            .with_expected_relay_pubkey(keys.public_key().to_hex());
        membership.apply_ready_records([
            ready_record_signed(1, "self", &keys),
            ready_record_signed(2, "peer", &keys),
        ]);
        let peers = membership.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].runtime_id, rid(2));
    }

    #[test]
    fn ready_records_must_have_valid_attestation() {
        let keys = relay_keys();
        let membership = MeshMembership::new(record(1, 1, 1))
            .with_expected_relay_pubkey(keys.public_key().to_hex());
        let mut tampered = ready_record_signed(2, "peer", &keys);
        tampered.runtime_id = rid(3);
        tampered.runtime_pubkey = rid(3).to_hex();

        membership.apply_ready_records([tampered]);
        assert!(membership.peers().is_empty());
    }

    #[test]
    fn ready_records_from_foreign_relay_identity_are_rejected() {
        let ours = relay_keys();
        let theirs = relay_keys();
        let membership = MeshMembership::new(record(1, 1, 1))
            .with_expected_relay_pubkey(ours.public_key().to_hex());

        // Validly signed, but by a key that isn't our deployment's identity.
        membership.apply_ready_records([ready_record_signed(2, "peer", &theirs)]);
        assert!(membership.peers().is_empty());
        assert_eq!(membership.status().counters.foreign_relay_rejections, 1);
    }

    #[test]
    fn unanchored_membership_rejects_all_ready_records() {
        let keys = relay_keys();
        let membership = MeshMembership::new(record(1, 1, 1));
        membership.apply_ready_records([ready_record_signed(2, "peer", &keys)]);
        assert!(membership.peers().is_empty());
        assert_eq!(membership.status().counters.foreign_relay_rejections, 1);
    }

    #[test]
    fn stale_gossip_record_is_ignored() {
        let membership = MeshMembership::new(record(1, 1, 1));
        assert!(membership.apply_gossip_record(record(2, 5, 1)));
        assert!(!membership.apply_gossip_record(record(2, 4, 2)));
        assert_eq!(membership.status().peers[0].record_version, 5);
    }

    #[test]
    fn counters_are_reflected_in_status() {
        let membership = MeshMembership::new(record(1, 1, 1));
        membership.apply_gossip_record(record(2, 1, 1));
        membership.record_datagram_sent(rid(2));
        membership.record_stale_generation_rejection(Some(rid(2)));
        let status = membership.status();
        assert_eq!(status.counters.stale_generation_rejections, 1);
        assert_eq!(status.peers[0].counters.datagrams_sent, 1);
        assert_eq!(status.peers[0].counters.stale_generation_rejections, 1);
    }

    const TTL_SECS: u64 = DEFAULT_PEER_RECORD_TTL.as_secs();

    #[test]
    fn expired_gossip_record_is_rejected() {
        let membership = MeshMembership::new(record(1, 1, 0));
        assert!(!membership.apply_gossip_record(record(2, 1, TTL_SECS + 1)));
        assert!(membership.status().peers.is_empty());
        assert!(!membership.has_peer(rid(2)));
        // A fresh record for the same runtime is still admitted.
        assert!(membership.apply_gossip_record(record(2, 2, 0)));
        assert!(membership.has_peer(rid(2)));
    }

    #[test]
    fn prune_drops_only_expired_records() {
        let membership = MeshMembership::new(record(1, 1, 0));
        membership.apply_gossip_record(record(2, 1, 0));
        membership.apply_gossip_record(record(3, 1, 200));
        // 150s later: rid(3) is 350s old (expired), rid(2) is 150s (live).
        let later = now_millis() + 150_000;
        assert_eq!(membership.prune_expired_at(later), 1);
        assert!(membership.has_peer(rid(2)));
        assert!(!membership.has_peer(rid(3)));
        assert_eq!(membership.prune_expired_at(later), 0);
    }

    #[test]
    fn prune_keeps_a_connected_peer_with_a_stale_heartbeat() {
        // A live connection whose gossip stopped arriving (control stream
        // died, or the peer's clock is behind) must not be pruned: a Redis
        // rescan would re-add it as `Disconnected` and nothing would restore
        // `Connected`. Its stale record still stays out of digests and dials.
        let membership = MeshMembership::new(record(1, 1, 0));
        membership.apply_gossip_record(record(2, 1, 0));
        membership.mark_connection_state(rid(2), ConnectionState::Connected);
        let later = now_millis() + (TTL_SECS + 1) * 1_000;
        assert_eq!(membership.prune_expired_at(later), 0);
        assert!(membership.has_peer(rid(2)));
        let status = membership.status();
        assert_eq!(status.peers[0].connection_state, ConnectionState::Connected);
        assert!(!membership
            .records_at(later)
            .iter()
            .any(|r| r.runtime_id == rid(2)));
        // Once the connection really closes, the next prune drops it.
        membership.mark_connection_state(rid(2), ConnectionState::Disconnected);
        assert_eq!(membership.prune_expired_at(later), 1);
        assert!(!membership.has_peer(rid(2)));
    }

    #[test]
    fn expired_unconnected_records_are_not_admitted_or_routed_before_prune() {
        // Prune runs once per reconcile pass; a slow pass must not leave
        // ghosts admissible or routable in between.
        let ttl = Duration::from_millis(50);
        let membership = MeshMembership::new(record(1, 1, 0)).with_peer_record_ttl(ttl);
        membership.apply_gossip_record(record(2, 1, 0));
        membership.apply_gossip_record(record(3, 1, 0));
        membership.mark_connection_state(rid(3), ConnectionState::Connected);
        std::thread::sleep(ttl * 3);
        assert!(!membership.has_peer(rid(2)));
        assert!(!membership.peer_has_capability(rid(2), "reliable-stream"));
        let routed: Vec<_> = membership
            .peers()
            .into_iter()
            .map(|p| p.runtime_id)
            .collect();
        assert!(!routed.contains(&rid(2)));
        // The connected peer stays admitted and routable.
        assert!(membership.has_peer(rid(3)));
        assert!(membership.peer_has_capability(rid(3), "reliable-stream"));
        assert!(routed.contains(&rid(3)));
    }

    #[test]
    fn a_fresh_record_replaces_an_expired_unpruned_entry_whatever_its_version() {
        // A live peer whose stale entry outlived the TTL but not yet a prune
        // must be re-admitted by its v1 Redis hint, or inbound from it is
        // rejected until the next pass.
        let ttl = Duration::from_millis(50);
        let membership = MeshMembership::new(record(1, 1, 0)).with_peer_record_ttl(ttl);
        assert!(membership.apply_gossip_record(record(2, 5, 0)));
        std::thread::sleep(ttl * 3);
        assert!(!membership.has_peer(rid(2)));
        assert!(membership.apply_gossip_record(record(2, 1, 0)));
        assert!(membership.has_peer(rid(2)));
        // The dead gap is not kept as a phi sample: one fresh observation
        // and no intervals reads as no suspicion yet.
        assert_eq!(membership.status().peers[0].phi, None);
        // A live entry still keeps the version guard.
        assert!(!membership.apply_gossip_record(record(2, 1, 0)));
    }

    #[test]
    fn expired_records_are_not_dialed_or_gossiped_before_prune() {
        let membership = MeshMembership::new(record(1, 1, 0));
        membership.apply_gossip_record(record(2, 1, 0));
        membership.apply_gossip_record(record(3, 1, 200));
        let later = now_millis() + 150_000;
        let ids: Vec<_> = membership
            .records_at(later)
            .into_iter()
            .map(|r| r.runtime_id)
            .collect();
        assert!(ids.contains(&rid(1)), "local record is always present");
        assert!(ids.contains(&rid(2)));
        assert!(
            !ids.contains(&rid(3)),
            "expired record must not be a candidate"
        );
        // Still in the table until a prune runs.
        assert!(membership.has_peer(rid(3)));
    }

    #[test]
    fn gossip_never_sets_connection_state() {
        let membership = MeshMembership::new(record(1, 1, 0));
        let state = |m: &MeshMembership| m.status().peers[0].connection_state;

        membership.apply_gossip_record(record(2, 1, 0));
        assert_eq!(state(&membership), ConnectionState::Disconnected);

        membership.mark_connection_state(rid(2), ConnectionState::Connected);
        membership.apply_gossip_record(record(2, 2, 0));
        assert_eq!(state(&membership), ConnectionState::Connected);

        membership.mark_connection_state(rid(2), ConnectionState::Disconnected);
        membership.apply_gossip_record(record(2, 3, 0));
        assert_eq!(state(&membership), ConnectionState::Disconnected);
    }

    #[test]
    fn table_stats_count_state_and_draining_and_track_oldest_heartbeat() {
        use crate::telemetry::CONNECTION_STATES;

        let membership = MeshMembership::new(record(1, 1, 0));
        membership.apply_gossip_record(record(2, 1, 10));
        membership.apply_gossip_record(record(3, 1, 120));
        membership.mark_connection_state(rid(3), ConnectionState::Connected);
        let mut draining = record(4, 1, 30);
        draining.draining = true;
        membership.apply_gossip_record(draining);

        let stats = membership.table_stats(SystemTime::now());
        let count = |state, draining: bool| {
            let index = CONNECTION_STATES.iter().position(|s| *s == state).unwrap();
            stats.counts[index][usize::from(draining)]
        };
        assert_eq!(count(ConnectionState::Disconnected, false), 1);
        assert_eq!(count(ConnectionState::Disconnected, true), 1);
        assert_eq!(count(ConnectionState::Connected, false), 1);
        assert_eq!(stats.counts.iter().flatten().sum::<u64>(), 3);
        assert!(stats.max_age >= Duration::from_secs(120));
        assert!(stats.max_age < Duration::from_secs(130));
    }

    #[test]
    fn begin_drain_updates_local_record() {
        let membership = MeshMembership::new(record(1, 1, 1));
        membership.begin_drain();
        assert!(membership.is_draining());
        assert!(membership.local_record().draining);
        assert_eq!(membership.local_record().version, 2);
    }
}
