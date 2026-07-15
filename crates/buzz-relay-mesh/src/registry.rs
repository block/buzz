//! Redis ready-registry bootstrap for the relay mesh.
//!
//! The registry is only the way into the mesh. Entries are membership hints:
//! they tell a fresh runtime which peer endpoints to dial, but never decide
//! session ownership or takeover. The fenced Redis session directory remains
//! the arbiter for session generations.

use std::str::FromStr;
use std::time::Duration;

use nostr::secp256k1::schnorr::Signature;
use nostr::secp256k1::{Message, XOnlyPublicKey};
use nostr::PublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{MeshError, RuntimeId};

pub const READY_KEY_PREFIX: &str = "mesh:ready:";
pub const DEFAULT_REGISTRY_REFRESH: Duration = Duration::from_secs(15);
pub const REGISTRY_EXPIRY_MULTIPLIER: u64 = 3;
pub const ATTESTATION_CONTEXT: &str = "buzz-relay-mesh-ready-v1";

/// Relay-key-signed binding for a boot-unique runtime endpoint pubkey.
///
/// The relay public key is the deployment Nostr/secp256k1 identity. It never
/// becomes the mesh runtime id; it only signs this Redis-published binding so
/// peers can reject unauthenticated endpoint ids before dialing/accepting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAttestation {
    /// Nostr/secp256k1 relay public key, hex encoded.
    pub relay_pubkey: String,
    /// Schnorr signature by `relay_pubkey` over [`attestation_preimage`].
    pub relay_sig: String,
}

impl RuntimeAttestation {
    pub fn new(relay_keys: &nostr::Keys, runtime_id: RuntimeId) -> Self {
        let relay_pubkey = relay_keys.public_key().to_hex();
        let message = attestation_message(runtime_id, &relay_pubkey);
        let relay_sig = relay_keys.sign_schnorr(&message).to_string();
        Self {
            relay_pubkey,
            relay_sig,
        }
    }

    pub fn verify(&self, runtime_id: RuntimeId) -> Result<(), MeshError> {
        verify_attestation(runtime_id, &self.relay_pubkey, &self.relay_sig)
    }
}

fn verify_attestation(
    runtime_id: RuntimeId,
    relay_pubkey: &str,
    relay_sig: &str,
) -> Result<(), MeshError> {
    let relay_pubkey = PublicKey::from_hex(relay_pubkey).map_err(|err| {
        MeshError::Transport(format!(
            "ready registry attestation invalid relay_pubkey: {err}"
        ))
    })?;
    let xonly: XOnlyPublicKey = relay_pubkey.xonly().map_err(|err| {
        MeshError::Transport(format!(
            "ready registry attestation relay_pubkey xonly conversion failed: {err}"
        ))
    })?;
    let sig = Signature::from_str(relay_sig).map_err(|err| {
        MeshError::Transport(format!(
            "ready registry attestation invalid relay_sig: {err}"
        ))
    })?;
    let message = attestation_message(runtime_id, &relay_pubkey.to_hex());
    nostr::secp256k1::SECP256K1
        .verify_schnorr(&sig, &message, &xonly)
        .map_err(|err| {
            MeshError::Transport(format!(
                "ready registry attestation signature verification failed: {err}"
            ))
        })
}

/// Stable signed payload. Keep this textual and versioned so transport/relay
/// integration can reproduce it exactly without depending on JSON key order.
pub fn attestation_preimage(runtime_id: RuntimeId, relay_pubkey: &str) -> String {
    format!(
        "{ATTESTATION_CONTEXT}\nruntime_pubkey={}\nrelay_pubkey={relay_pubkey}",
        runtime_id.to_hex()
    )
}

fn attestation_message(runtime_id: RuntimeId, relay_pubkey: &str) -> Message {
    let digest = Sha256::digest(attestation_preimage(runtime_id, relay_pubkey).as_bytes());
    Message::from_digest(digest.into())
}

/// Value stored at `mesh:ready:{runtime_id}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadyRecord {
    pub runtime_id: RuntimeId,
    /// Explicit duplicate of `runtime_id` for the contract record shape: this
    /// is the boot-unique ed25519/iroh endpoint pubkey being attested.
    pub runtime_pubkey: String,
    /// Nostr/secp256k1 relay public key that signs `runtime_pubkey`.
    pub relay_pubkey: String,
    /// Schnorr signature by `relay_pubkey` over [`attestation_preimage`].
    pub relay_sig: String,
    /// Dialable iroh endpoint addresses, serialized as strings so this layer
    /// does not depend on transport internals.
    pub endpoint_addrs: Vec<String>,
    pub proto_version: u16,
    pub capabilities: Vec<String>,
}

impl ReadyRecord {
    pub fn new(
        runtime_id: RuntimeId,
        relay_keys: &nostr::Keys,
        endpoint_addrs: Vec<String>,
        proto_version: u16,
        capabilities: Vec<String>,
    ) -> Self {
        let attestation = RuntimeAttestation::new(relay_keys, runtime_id);
        Self {
            runtime_id,
            runtime_pubkey: runtime_id.to_hex(),
            relay_pubkey: attestation.relay_pubkey,
            relay_sig: attestation.relay_sig,
            endpoint_addrs,
            proto_version,
            capabilities,
        }
    }

    pub fn key(&self) -> String {
        ready_key(self.runtime_id)
    }

    pub fn verify_attestation(&self) -> Result<(), MeshError> {
        if self.runtime_pubkey != self.runtime_id.to_hex() {
            return Err(MeshError::Transport(format!(
                "ready registry runtime_id/runtime_pubkey mismatch: {} != {}",
                self.runtime_id, self.runtime_pubkey
            )));
        }
        verify_attestation(self.runtime_id, &self.relay_pubkey, &self.relay_sig)
    }
}

pub fn ready_key(runtime_id: RuntimeId) -> String {
    format!("{READY_KEY_PREFIX}{runtime_id}")
}

pub fn expiry_for(refresh: Duration) -> Duration {
    refresh.saturating_mul(REGISTRY_EXPIRY_MULTIPLIER as u32)
}

/// Redis-backed mesh bootstrap registry.
#[derive(Clone)]
pub struct ReadyRegistry {
    pool: deadpool_redis::Pool,
    refresh: Duration,
}

impl ReadyRegistry {
    pub fn new(pool: deadpool_redis::Pool, refresh: Duration) -> Self {
        Self { pool, refresh }
    }

    pub fn refresh_interval(&self) -> Duration {
        self.refresh
    }

    pub fn expiry(&self) -> Duration {
        expiry_for(self.refresh)
    }

    /// Publish this runtime as ready. Callers MUST only invoke this after the
    /// relay would pass readiness (shutdown=false, Postgres reachable, Redis
    /// reachable). This method deliberately has no hidden readiness probe so the
    /// rule stays explicit at the relay boundary.
    pub async fn publish_ready(&self, record: &ReadyRecord) -> Result<(), MeshError> {
        record.verify_attestation()?;
        let mut conn = self.conn().await?;
        let payload = serde_json::to_string(record)
            .map_err(|e| MeshError::Transport(format!("ready registry encode: {e}")))?;
        let ttl_secs = self.expiry().as_secs().max(1);
        redis::cmd("SET")
            .arg(record.key())
            .arg(payload)
            .arg("EX")
            .arg(ttl_secs)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Remove this runtime on clean shutdown. A crash is handled by TTL expiry.
    pub async fn clear_ready(&self, runtime_id: RuntimeId) -> Result<(), MeshError> {
        let mut conn = self.conn().await?;
        redis::cmd("DEL")
            .arg(ready_key(runtime_id))
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Scan all ready records. Malformed/stale/unauthenticated values are
    /// skipped with a warn: a bad registry entry must not prevent bootstrap
    /// from healthy peers.
    pub async fn scan_ready(&self) -> Result<Vec<ReadyRecord>, MeshError> {
        let mut conn = self.conn().await?;
        let mut cursor = 0u64;
        let mut out = Vec::new();

        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{READY_KEY_PREFIX}*"))
                .arg("COUNT")
                .arg(100u32)
                .query_async(&mut conn)
                .await?;

            for key in keys {
                let raw: Option<String> =
                    redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
                let Some(raw) = raw else { continue };
                match serde_json::from_str::<ReadyRecord>(&raw) {
                    Ok(record) if record.key() == key => match record.verify_attestation() {
                        Ok(()) => out.push(record),
                        Err(err) => tracing::warn!(
                            key,
                            runtime_id = %record.runtime_id,
                            %err,
                            "mesh ready registry attestation failed — skipping"
                        ),
                    },
                    Ok(record) => tracing::warn!(
                        key,
                        runtime_id = %record.runtime_id,
                        "mesh ready registry key/runtime mismatch — skipping"
                    ),
                    Err(err) => {
                        tracing::warn!(key, %err, "mesh ready registry decode failed — skipping")
                    }
                }
            }

            if next == 0 {
                break;
            }
            cursor = next;
        }

        Ok(out)
    }

    pub fn heartbeat(&self, record: ReadyRecord) -> ReadyHeartbeat {
        ReadyHeartbeat {
            registry: self.clone(),
            record,
            published: false,
        }
    }

    async fn conn(&self) -> Result<deadpool_redis::Connection, MeshError> {
        self.pool
            .get()
            .await
            .map_err(|e| MeshError::Transport(format!("redis pool: {e}")))
    }
}

/// Readiness-gated registry heartbeat.
///
/// The relay owns the readiness predicate; this helper owns the edge behavior:
/// publish only while ready, clear on ready→not-ready, and clear on shutdown.
pub struct ReadyHeartbeat {
    registry: ReadyRegistry,
    record: ReadyRecord,
    published: bool,
}

impl ReadyHeartbeat {
    pub fn record(&self) -> &ReadyRecord {
        &self.record
    }

    pub fn published(&self) -> bool {
        self.published
    }

    pub async fn tick(&mut self, ready: bool) -> Result<(), MeshError> {
        if ready {
            self.registry.publish_ready(&self.record).await?;
            self.published = true;
        } else if self.published {
            self.registry.clear_ready(self.record.runtime_id).await?;
            self.published = false;
        }
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), MeshError> {
        if self.published {
            self.registry.clear_ready(self.record.runtime_id).await?;
            self.published = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(byte: u8) -> RuntimeId {
        RuntimeId([byte; 32])
    }

    fn relay_keys() -> nostr::Keys {
        nostr::Keys::generate()
    }

    fn ready_record(byte: u8) -> ReadyRecord {
        ReadyRecord::new(rid(byte), &relay_keys(), vec![], 1, vec![])
    }

    #[test]
    fn ready_key_is_stable_and_namespaced() {
        assert_eq!(
            ready_key(rid(0xAB)),
            format!("mesh:ready:{}", "ab".repeat(32))
        );
    }

    #[test]
    fn expiry_is_three_refreshes() {
        assert_eq!(expiry_for(Duration::from_secs(15)), Duration::from_secs(45));
    }

    #[test]
    fn heartbeat_starts_unpublished() {
        let pool = deadpool_redis::Config::from_url("redis://127.0.0.1:6379")
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();
        let registry = ReadyRegistry::new(pool, Duration::from_secs(15));
        let heartbeat = registry.heartbeat(ready_record(1));
        assert!(!heartbeat.published());
        assert_eq!(heartbeat.record().runtime_id, rid(1));
    }

    #[test]
    fn ready_record_roundtrips_json() {
        let record = ReadyRecord::new(
            rid(7),
            &relay_keys(),
            vec!["127.0.0.1:3478".to_string()],
            1,
            vec!["realtime-media".to_string()],
        );
        let raw = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<ReadyRecord>(&raw).unwrap(), record);
    }

    #[test]
    fn ready_record_attestation_verifies_and_binds_runtime_pubkey() {
        let record = ready_record(9);
        record.verify_attestation().unwrap();

        let mut tampered = record.clone();
        tampered.runtime_pubkey = rid(10).to_hex();
        assert!(tampered.verify_attestation().is_err());
    }

    #[test]
    fn attestation_rejects_signature_for_other_runtime() {
        let mut record = ready_record(11);
        record.runtime_id = rid(12);
        record.runtime_pubkey = rid(12).to_hex();
        assert!(record.verify_attestation().is_err());
    }
}
