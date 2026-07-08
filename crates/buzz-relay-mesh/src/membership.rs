//! In-memory mesh membership table fed by Redis seed records and gossip.
//!
//! This module implements the relay-facing [`RelayMeshMembership`] seam. It is
//! deliberately incapable of electing session owners: peers here are dial/routing
//! hints only, and liveness disagreement never performs takeover.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use crate::gossip::{system_time_from_millis, GossipRecord, PhiAccrual};
use crate::registry::ReadyRecord;
use crate::status::{ConnectionState, MeshCounters, MeshPeerCounters, MeshPeerStatus, MeshStatus};
use crate::{PeerInfo, RelayMeshMembership, RuntimeId};

pub const DEFAULT_PHI_SUSPECT_THRESHOLD: f64 = 8.0;

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
    phi_suspect_threshold: f64,
}

impl MeshMembership {
    pub fn new(local_record: GossipRecord) -> Self {
        Self {
            local_runtime_id: local_record.runtime_id,
            local_record: Arc::new(RwLock::new(local_record)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            draining: Arc::new(AtomicBool::new(false)),
            stale_generation_rejections: Arc::new(AtomicU64::new(0)),
            phi_suspect_threshold: DEFAULT_PHI_SUSPECT_THRESHOLD,
        }
    }

    pub fn with_phi_suspect_threshold(mut self, threshold: f64) -> Self {
        self.phi_suspect_threshold = threshold;
        self
    }

    pub fn local_record(&self) -> GossipRecord {
        self.local_record
            .read()
            .expect("local record lock poisoned")
            .clone()
    }

    /// Apply Redis bootstrap records. Existing gossip records win when they are
    /// newer; ready-registry records enter as version 1 hints.
    pub fn apply_ready_records(&self, records: impl IntoIterator<Item = ReadyRecord>) {
        for ready in records {
            if ready.runtime_id == self.local_runtime_id {
                continue;
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

    /// Apply a gossiped record if it is newer than the local copy.
    pub fn apply_gossip_record(&self, record: GossipRecord) -> bool {
        if record.runtime_id == self.local_runtime_id {
            return false;
        }

        let heartbeat = system_time_from_millis(record.heartbeat_millis);
        let mut peers = self.peers.write().expect("membership lock poisoned");
        match peers.get_mut(&record.runtime_id) {
            Some(peer) if record.version <= peer.record.version => false,
            Some(peer) => {
                peer.record = record;
                peer.connection_state = ConnectionState::Connected;
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
                        connection_state: ConnectionState::Connected,
                    },
                );
                true
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
            .contains_key(&runtime_id)
    }

    /// All known gossip records (local + peers), for reconcile/dial decisions.
    pub fn records(&self) -> Vec<GossipRecord> {
        let mut records: Vec<GossipRecord> = self
            .peers
            .read()
            .expect("membership lock poisoned")
            .values()
            .map(|peer| peer.record.clone())
            .collect();
        records.push(self.local_record());
        records
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
        self.peers
            .read()
            .expect("membership lock poisoned")
            .values()
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
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn rid(byte: u8) -> RuntimeId {
        RuntimeId([byte; 32])
    }

    fn record(byte: u8, version: u64, heartbeat_secs: u64) -> GossipRecord {
        GossipRecord {
            runtime_id: rid(byte),
            endpoint_addrs: vec![format!("127.0.0.{byte}:3478")],
            proto_version: 1,
            load: 0.25,
            draining: false,
            capabilities: vec!["reliable-stream".to_string()],
            version,
            heartbeat_millis: (UNIX_EPOCH + Duration::from_secs(heartbeat_secs))
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }

    fn relay_keys() -> nostr::Keys {
        nostr::Keys::generate()
    }

    fn ready_record(byte: u8, endpoint_addr: &str) -> ReadyRecord {
        ReadyRecord::new(
            rid(byte),
            &relay_keys(),
            vec![endpoint_addr.into()],
            1,
            vec![],
        )
    }

    #[test]
    fn ready_records_seed_peers_but_skip_self() {
        let membership = MeshMembership::new(record(1, 1, 1));
        membership.apply_ready_records([ready_record(1, "self"), ready_record(2, "peer")]);
        let peers = membership.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].runtime_id, rid(2));
    }

    #[test]
    fn ready_records_must_have_valid_attestation() {
        let membership = MeshMembership::new(record(1, 1, 1));
        let mut tampered = ready_record(2, "peer");
        tampered.runtime_id = rid(3);
        tampered.runtime_pubkey = rid(3).to_hex();

        membership.apply_ready_records([tampered]);
        assert!(membership.peers().is_empty());
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

    #[test]
    fn begin_drain_updates_local_record() {
        let membership = MeshMembership::new(record(1, 1, 1));
        membership.begin_drain();
        assert!(membership.is_draining());
        assert!(membership.local_record().draining);
        assert_eq!(membership.local_record().version, 2);
    }
}
