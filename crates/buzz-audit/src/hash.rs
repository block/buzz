use chrono::{DateTime, SubsecRound, Utc};
use sha2::{Digest, Sha256};

use crate::entry::AuditEntry;
use crate::error::AuditError;

/// The 32-byte sentinel hashed in place of `prev_hash` for a community's first
/// entry. Stored as `prev_hash = NULL`; hashed as all-zero bytes.
pub const GENESIS_HASH: [u8; 32] = [0u8; 32];

/// Reduce a timestamp to the precision the audit store round-trips.
///
/// `audit_log.created_at` is `TIMESTAMPTZ`, which Postgres keeps at microsecond
/// resolution. [`compute_hash`] covers `created_at.to_rfc3339()`, and that
/// string's sub-second digit count follows the value (chrono emits 0, 3, 6 or 9
/// digits), so a timestamp carrying nanoseconds hashes to a digest that can
/// never be recomputed from the stored row — the entry is written with one
/// preimage and verified against another.
///
/// Every `created_at` must therefore pass through here *before* it is hashed
/// and stored, so the in-memory entry and the row are byte-identical.
pub fn to_storage_precision(created_at: DateTime<Utc>) -> DateTime<Utc> {
    created_at.trunc_subsecs(6)
}

/// SHA-256 over the entry's identity, chain, and context fields.
///
/// Field order is fixed — changing it invalidates all existing chains. The
/// `community_id` is hashed first so chain identity carries the tenant: an entry
/// cannot be lifted out of one community's chain and re-verified inside another.
///
/// `created_at` is normalized through [`to_storage_precision`] here rather than
/// hashed as given. Write paths truncate before storing so the row matches the
/// in-memory entry, but normalizing again at the single point that consumes the
/// value means no future caller can reintroduce the write/read preimage split
/// by forgetting to. Values already at storage precision are unaffected —
/// truncation is idempotent — so this does not change any digest.
///
/// `detail` is serialized via [`canonical_json`] (sorted keys) so the hash is
/// stable across machines and Rust versions. A serialization failure is a hard
/// error, never silently hashed as empty.
pub fn compute_hash(entry: &AuditEntry) -> Result<[u8; 32], AuditError> {
    let mut hasher = Sha256::new();
    // Tenant binding: community_id leads the hash.
    hasher.update(entry.community_id.as_bytes());
    hasher.update(entry.seq.to_be_bytes());
    hasher.update(
        to_storage_precision(entry.created_at)
            .to_rfc3339()
            .as_bytes(),
    );
    hasher.update(entry.action.as_str().as_bytes());
    match &entry.actor_pubkey {
        Some(pk) => {
            hasher.update([1u8]); // presence tag — distinguishes Some(empty) from None
            hasher.update(pk);
        }
        None => hasher.update([0u8]),
    }
    match &entry.object_id {
        Some(id) => {
            hasher.update([1u8]);
            hasher.update(id.as_bytes());
        }
        None => hasher.update([0u8]),
    }
    hasher.update(canonical_json(&entry.detail)?.as_bytes());
    match &entry.prev_hash {
        Some(h) => hasher.update(h),
        None => hasher.update(GENESIS_HASH),
    }
    Ok(hasher.finalize().into())
}

/// Serialize a JSON value with sorted object keys for deterministic output.
///
/// Propagates any scalar serialization error rather than substituting a
/// placeholder — a hash must never silently stand in an empty value for a real
/// payload.
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
    use uuid::Uuid;

    fn sample_entry() -> AuditEntry {
        AuditEntry {
            community_id: Uuid::from_u128(1),
            seq: 1,
            hash: Vec::new(),
            prev_hash: None,
            action: AuditAction::EventCreated,
            actor_pubkey: Some(vec![0xab; 32]),
            object_id: Some("abc123".into()),
            detail: serde_json::Value::Null,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    /// A wall-clock instant carrying sub-microsecond digits, like `Utc::now()`
    /// returns on Linux (`clock_gettime`, nanosecond resolution).
    fn nanosecond_instant() -> chrono::DateTime<Utc> {
        chrono::DateTime::from_timestamp_nanos(1_700_000_000_123_456_789)
    }

    /// What Postgres hands back for a `TIMESTAMPTZ`: microsecond resolution.
    fn after_database_round_trip(ts: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
        ts.trunc_subsecs(6)
    }

    #[test]
    fn deterministic() {
        let entry = sample_entry();
        assert_eq!(compute_hash(&entry).unwrap(), compute_hash(&entry).unwrap());
        assert_eq!(compute_hash(&entry).unwrap().len(), 32);
    }

    #[test]
    fn storage_precision_drops_sub_microsecond_digits() {
        let stored = to_storage_precision(nanosecond_instant());
        assert_eq!(stored.timestamp_subsec_nanos(), 123_456_000);
        // Idempotent, so a stored value re-read from Postgres is unchanged.
        assert_eq!(stored, after_database_round_trip(stored));
    }

    #[test]
    fn rfc3339_sub_second_width_follows_the_value() {
        // The underlying trap, pinned on the preimage rather than the digest:
        // chrono emits 0/3/6/9 fractional digits depending on the value, so a
        // nanosecond timestamp and its microsecond truncation are *different
        // strings*. Hashing the untruncated value therefore produces a digest
        // that cannot be recomputed from the stored row — which is what made
        // every entry fail `verify_chain` with `HashMismatch`.
        let ns = nanosecond_instant();
        assert_eq!(ns.to_rfc3339(), "2023-11-14T22:13:20.123456789+00:00");
        assert_eq!(
            after_database_round_trip(ns).to_rfc3339(),
            "2023-11-14T22:13:20.123456+00:00"
        );
        assert_ne!(ns.to_rfc3339(), after_database_round_trip(ns).to_rfc3339());
    }

    #[test]
    fn compute_hash_normalizes_sub_microsecond_timestamps() {
        // The enforcement point: even handed an untruncated `created_at`,
        // `compute_hash` digests the storage-precision value, so a write path
        // that forgot to truncate cannot split the write/read preimage.
        let ns = nanosecond_instant();
        let mut written = sample_entry();
        written.created_at = ns;
        let mut read_back = sample_entry();
        read_back.created_at = after_database_round_trip(ns);

        assert_eq!(
            compute_hash(&written).unwrap(),
            compute_hash(&read_back).unwrap()
        );
    }

    #[test]
    fn storage_precision_timestamps_survive_a_database_round_trip() {
        // The invariant the write path must hold: hash what will be stored, so
        // recomputing from the row reproduces the digest.
        let mut written = sample_entry();
        written.created_at = to_storage_precision(nanosecond_instant());
        let mut read_back = written.clone();
        read_back.created_at = after_database_round_trip(read_back.created_at);

        assert_eq!(
            compute_hash(&written).unwrap(),
            compute_hash(&read_back).unwrap()
        );
    }

    #[test]
    fn community_id_is_part_of_identity() {
        // The whole point: the same logical entry in two communities hashes
        // differently, so a row can't be replayed across chains.
        let a = sample_entry();
        let mut b = a.clone();
        b.community_id = Uuid::from_u128(2);
        assert_ne!(compute_hash(&a).unwrap(), compute_hash(&b).unwrap());
    }

    #[test]
    fn sensitive_to_each_field() {
        let base = sample_entry();
        let h0 = compute_hash(&base).unwrap();

        let mut e = base.clone();
        e.seq = 2;
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.action = AuditAction::EventDeleted;
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.actor_pubkey = Some(vec![0xcd; 32]);
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.object_id = Some("different".into());
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.detail = serde_json::json!({"key": "value"});
        assert_ne!(h0, compute_hash(&e).unwrap());

        let mut e = base.clone();
        e.prev_hash = Some(vec![0xff; 32]);
        assert_ne!(h0, compute_hash(&e).unwrap());
    }

    #[test]
    fn presence_tag_distinguishes_none_from_empty() {
        // Some(empty) must not collide with None — the presence tag prevents it.
        let mut none = sample_entry();
        none.actor_pubkey = None;
        let mut empty = sample_entry();
        empty.actor_pubkey = Some(Vec::new());
        assert_ne!(compute_hash(&none).unwrap(), compute_hash(&empty).unwrap());
    }

    #[test]
    fn canonical_json_key_order_is_stable() {
        let a = serde_json::json!({"z": 1, "a": 2, "m": 3});
        let b = serde_json::json!({"a": 2, "m": 3, "z": 1});
        assert_eq!(canonical_json(&a).unwrap(), canonical_json(&b).unwrap());
    }
}
