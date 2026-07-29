//! Scuttlebutt-style membership gossip over the mesh control stream.
//!
//! Gossip answers liveness/dialability questions only. It never elects owners,
//! never transfers sessions, and never carries tunnel data bytes.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{MeshError, RuntimeId};

/// Wire version of the gossip payload exchanged over the control stream.
pub const GOSSIP_PAYLOAD_VERSION: u8 = 1;

/// A single runtime's gossiped state: identity, dialability, load, and version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GossipRecord {
    /// The runtime this record describes.
    pub runtime_id: RuntimeId,
    /// Dialable endpoint address strings for the runtime.
    pub endpoint_addrs: Vec<String>,
    /// Protocol version the runtime speaks.
    pub proto_version: u16,
    /// Advisory load factor (0.0..) gossiped by the runtime.
    pub load: f32,
    /// Whether the runtime is draining.
    pub draining: bool,
    /// Capability strings advertised by the runtime.
    pub capabilities: Vec<String>,
    /// Per-runtime monotonic version. Only the owning runtime may increment its
    /// own record; receivers apply last-version-wins.
    pub version: u64,
    /// Wall-clock heartbeat timestamp (ms since UNIX_EPOCH).
    pub heartbeat_millis: u64,
}

impl GossipRecord {
    /// Create a fresh local record with default load (0.0) and version 1.
    pub fn new(runtime_id: RuntimeId, endpoint_addrs: Vec<String>, proto_version: u16) -> Self {
        Self {
            runtime_id,
            endpoint_addrs,
            proto_version,
            load: 0.0,
            draining: false,
            capabilities: Vec::new(),
            version: 1,
            heartbeat_millis: now_millis(),
        }
    }
}

/// One entry in a gossip digest: a runtime id paired with its latest version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GossipDigestEntry {
    /// The runtime the version refers to.
    pub runtime_id: RuntimeId,
    /// The version known to the digest sender.
    pub version: u64,
}

/// A gossip control message: either a digest request or a delta reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GossipMessage {
    /// Compact digest of the sender's known versions.
    Digest {
        /// Gossip payload version.
        version: u8,
        /// One entry per known runtime.
        entries: Vec<GossipDigestEntry>,
    },
    /// Delta of records newer than the receiver's digest.
    Delta {
        /// Gossip payload version.
        version: u8,
        /// Records the receiver does not yet have at this version.
        records: Vec<GossipRecord>,
    },
}

/// Serialize a gossip message with postcard.
pub fn encode_message(message: &GossipMessage) -> Result<Vec<u8>, MeshError> {
    postcard::to_extend(message, Vec::new()).map_err(MeshError::Encode)
}

/// Deserialize a gossip message and validate its payload version.
pub fn decode_message(bytes: &[u8]) -> Result<GossipMessage, MeshError> {
    let message: GossipMessage = postcard::from_bytes(bytes).map_err(MeshError::Decode)?;
    let version = match &message {
        GossipMessage::Digest { version, .. } | GossipMessage::Delta { version, .. } => *version,
    };
    if version != GOSSIP_PAYLOAD_VERSION {
        return Err(MeshError::Transport(format!(
            "unknown gossip payload version {version}"
        )));
    }
    Ok(message)
}

/// Pure scuttlebutt state: digest exchange + delta application.
#[derive(Clone, Debug)]
pub struct GossipState {
    records: HashMap<RuntimeId, GossipRecord>,
}

impl GossipState {
    /// Create a state seeded with the local runtime's own record.
    pub fn new(local: GossipRecord) -> Self {
        let mut records = HashMap::new();
        records.insert(local.runtime_id, local);
        Self { records }
    }

    /// Iterate over all known records (including the local one).
    pub fn records(&self) -> impl Iterator<Item = &GossipRecord> {
        self.records.values()
    }

    /// Look up the record for a runtime, if known.
    pub fn get(&self, runtime_id: RuntimeId) -> Option<&GossipRecord> {
        self.records.get(&runtime_id)
    }

    /// Apply `update` to the local copy of a runtime's record, bumping its
    /// version and heartbeat. Returns the updated record, or `None` if the
    /// runtime is unknown.
    pub fn update_local<F>(&mut self, runtime_id: RuntimeId, update: F) -> Option<GossipRecord>
    where
        F: FnOnce(&mut GossipRecord),
    {
        let record = self.records.get_mut(&runtime_id)?;
        update(record);
        record.version = record.version.saturating_add(1);
        record.heartbeat_millis = now_millis();
        Some(record.clone())
    }

    /// Build a digest message covering every known runtime, sorted by id.
    pub fn digest(&self) -> GossipMessage {
        let mut entries: Vec<_> = self
            .records
            .values()
            .map(|record| GossipDigestEntry {
                runtime_id: record.runtime_id,
                version: record.version,
            })
            .collect();
        entries.sort_by_key(|entry| entry.runtime_id.to_hex());
        GossipMessage::Digest {
            version: GOSSIP_PAYLOAD_VERSION,
            entries,
        }
    }

    /// Build the delta of records newer than the peer's `digest`.
    pub fn delta_for(&self, digest: &[GossipDigestEntry]) -> GossipMessage {
        let remote_versions: HashMap<_, _> = digest
            .iter()
            .map(|entry| (entry.runtime_id, entry.version))
            .collect();
        let mut records: Vec<_> = self
            .records
            .values()
            .filter(|record| {
                remote_versions
                    .get(&record.runtime_id)
                    .is_none_or(|remote| *remote < record.version)
            })
            .cloned()
            .collect();
        records.sort_by_key(|record| record.runtime_id.to_hex());
        GossipMessage::Delta {
            version: GOSSIP_PAYLOAD_VERSION,
            records,
        }
    }

    /// Applies records whose version is newer than the local copy. Returns the
    /// runtime ids that changed.
    pub fn apply_delta(&mut self, records: Vec<GossipRecord>) -> Vec<RuntimeId> {
        let mut changed = Vec::new();
        for record in records {
            let should_apply = self
                .records
                .get(&record.runtime_id)
                .is_none_or(|existing| record.version > existing.version);
            if should_apply {
                changed.push(record.runtime_id);
                self.records.insert(record.runtime_id, record);
            }
        }
        changed
    }
}

/// Phi-accrual failure detector: estimates a suspicion score from heartbeat
/// inter-arrival intervals. Higher `phi` means the peer is more likely down.
#[derive(Clone, Debug)]
pub struct PhiAccrual {
    samples: Vec<Duration>,
    last_heartbeat: Option<SystemTime>,
    max_samples: usize,
}

impl Default for PhiAccrual {
    fn default() -> Self {
        Self::new(100)
    }
}

impl PhiAccrual {
    /// Create a detector that retains at most `max_samples` intervals.
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::new(),
            last_heartbeat: None,
            max_samples: max_samples.max(1),
        }
    }

    /// Record a heartbeat observation at `at`, updating the interval window.
    pub fn observe(&mut self, at: SystemTime) {
        if let Some(prev) = self.last_heartbeat {
            if let Ok(interval) = at.duration_since(prev) {
                if !interval.is_zero() {
                    self.samples.push(interval);
                    if self.samples.len() > self.max_samples {
                        self.samples.remove(0);
                    }
                }
            }
        }
        self.last_heartbeat = Some(at);
    }

    /// Current suspicion score, or `None` until at least one interval is known.
    pub fn phi_at(&self, now: SystemTime) -> Option<f64> {
        let last = self.last_heartbeat?;
        if self.samples.is_empty() {
            return None;
        }
        let elapsed = now.duration_since(last).ok()?.as_secs_f64();
        let mean = self.mean_secs();
        if mean <= f64::EPSILON {
            return None;
        }
        // Exponential approximation: phi = -log10(e^(-elapsed/mean)).
        Some((elapsed / mean) / std::f64::consts::LN_10)
    }

    /// Mean heartbeat inter-arrival interval in seconds.
    pub fn mean_secs(&self) -> f64 {
        let total: f64 = self.samples.iter().map(Duration::as_secs_f64).sum();
        total / self.samples.len() as f64
    }
}

/// Current wall clock as milliseconds since UNIX_EPOCH.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Convert a `now_millis`-style timestamp back into a [`SystemTime`].
pub fn system_time_from_millis(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(byte: u8) -> RuntimeId {
        RuntimeId([byte; 32])
    }

    #[test]
    fn digest_delta_only_sends_newer_records() {
        let mut a = GossipState::new(GossipRecord::new(rid(1), vec!["a".into()], 1));
        a.apply_delta(vec![GossipRecord::new(rid(2), vec!["b".into()], 1)]);

        let b_digest = [GossipDigestEntry {
            runtime_id: rid(1),
            version: 1,
        }];

        let GossipMessage::Delta { records, .. } = a.delta_for(&b_digest) else {
            panic!("expected delta")
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].runtime_id, rid(2));
    }

    #[test]
    fn apply_delta_ignores_stale_versions() {
        let mut state = GossipState::new(GossipRecord::new(rid(1), vec![], 1));
        let newer = GossipRecord {
            version: 10,
            ..GossipRecord::new(rid(2), vec!["new".into()], 1)
        };
        assert_eq!(state.apply_delta(vec![newer.clone()]), vec![rid(2)]);
        let stale = GossipRecord {
            version: 9,
            endpoint_addrs: vec!["stale".into()],
            ..newer
        };
        assert!(state.apply_delta(vec![stale]).is_empty());
        assert_eq!(state.get(rid(2)).unwrap().endpoint_addrs, vec!["new"]);
    }

    #[test]
    fn gossip_payload_roundtrips() {
        let message = GossipMessage::Digest {
            version: GOSSIP_PAYLOAD_VERSION,
            entries: vec![GossipDigestEntry {
                runtime_id: rid(9),
                version: 3,
            }],
        };
        assert_eq!(
            decode_message(&encode_message(&message).unwrap()).unwrap(),
            message
        );
    }

    #[test]
    fn phi_rises_as_heartbeats_age() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000);
        let mut phi = PhiAccrual::default();
        phi.observe(start);
        phi.observe(start + Duration::from_secs(1));
        phi.observe(start + Duration::from_secs(2));
        let early = phi.phi_at(start + Duration::from_secs(3)).unwrap();
        let late = phi.phi_at(start + Duration::from_secs(12)).unwrap();
        assert!(late > early);
    }
}
