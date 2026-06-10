use sha2::{Digest, Sha256};

use crate::entry::AuditEntry;
use crate::error::AuditError;

/// Sentinel `prev_hash` value used for the first entry in the chain.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// SHA-256 over all identity, chain, and context fields.
/// Field order is fixed — changing it invalidates all existing chains.
///
/// Metadata is serialized via `BTreeMap` to guarantee key ordering across
/// machines and Rust versions. `serde_json::Value` does not guarantee order.
///
/// Returns `Err(AuditError::Serialization)` if metadata cannot be serialized.
/// Never hashes a default/empty value as a stand-in for a real payload —
/// a serialization failure is a hard error, not a silent degradation.
pub fn compute_hash(entry: &AuditEntry) -> Result<String, AuditError> {
    let mut hasher = Sha256::new();
    hasher.update(entry.seq.to_be_bytes());
    hasher.update(entry.timestamp.to_rfc3339().as_bytes());
    hasher.update(entry.event_id.as_bytes());
    // event_kind is u32 — 4 bytes in big-endian for the hash chain.
    hasher.update(entry.event_kind.to_be_bytes());
    hasher.update(entry.actor_pubkey.as_bytes());
    hasher.update(entry.action.as_str().as_bytes());
    match &entry.channel_id {
        Some(id) => hasher.update(id.as_bytes()),
        None => hasher.update([0u8; 16]),
    }
    hasher.update(canonical_json(&entry.metadata)?.as_bytes());
    hasher.update(entry.prev_hash.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

/// Serialize a JSON value with sorted keys for deterministic output.
///
/// Returns `Err` if any scalar value cannot be serialized. This should never
/// happen for well-formed `serde_json::Value`, but we propagate rather than
/// silently substitute an empty string.
fn canonical_json(value: &serde_json::Value) -> Result<String, serde_json::Error> {
    use serde_json::Value;
    use std::collections::BTreeMap;

    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&str, &Value> = map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            let mut out = String::from("{");
            let mut first = true;
            for (k, v) in &sorted {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&serde_json::to_string(k)?);
                out.push(':');
                out.push_str(&canonical_json(v)?);
            }
            out.push('}');
            Ok(out)
        }
        Value::Array(arr) => {
            let mut out = String::from("[");
            let mut first = true;
            for v in arr {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&canonical_json(v)?);
            }
            out.push(']');
            Ok(out)
        }
        other => serde_json::to_string(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{action::AuditAction, entry::AuditEntry};
    use chrono::Utc;

    fn sample_entry() -> AuditEntry {
        AuditEntry {
            seq: 1,
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            event_id: "abc123".to_string(),
            event_kind: 1,
            actor_pubkey: "pubkey_alice".to_string(),
            action: AuditAction::EventCreated,
            channel_id: None,
            metadata: serde_json::Value::Null,
            prev_hash: GENESIS_HASH.to_string(),
            hash: String::new(),
        }
    }

    #[test]
    fn deterministic() {
        let entry = sample_entry();
        assert_eq!(compute_hash(&entry).unwrap(), compute_hash(&entry).unwrap());
        assert_eq!(compute_hash(&entry).unwrap().len(), 64);
    }

    #[test]
    fn sensitive_to_each_field() {
        let base = sample_entry();
        let h0 = compute_hash(&base).unwrap();

        let mut e = base.clone();
        e.event_id = "different_event".into();
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.seq = 2;
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.actor_pubkey = "pubkey_bob".into();
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.channel_id = Some(uuid::Uuid::new_v4());
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base;
        e.metadata = serde_json::json!({"key": "value"});
        assert_ne!(h0, compute_hash(&e).unwrap());
    }

    #[test]
    fn canonical_json_key_order_is_stable() {
        // Same keys in different insertion order must produce the same hash.
        let a = serde_json::json!({"z": 1, "a": 2, "m": 3});
        let b = serde_json::json!({"a": 2, "m": 3, "z": 1});
        assert_eq!(canonical_json(&a).unwrap(), canonical_json(&b).unwrap());
    }

    #[test]
    fn genesis_hash_format() {
        assert_eq!(GENESIS_HASH.len(), 64);
        assert!(GENESIS_HASH.chars().all(|c| c == '0'));
    }
}
