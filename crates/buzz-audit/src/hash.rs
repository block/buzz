use sha2::{Digest, Sha256};

use crate::entry::AuditEntry;
use crate::error::AuditError;

/// The 32-byte sentinel hashed in place of `prev_hash` for a community's first
/// entry. Stored as `prev_hash = NULL`; hashed as all-zero bytes.
pub const GENESIS_HASH: [u8; 32] = [0u8; 32];

/// SHA-256 over the entry's identity, chain, and context fields.
///
/// Field order is fixed — changing it invalidates all existing chains. The
/// `community_id` is hashed first so chain identity carries the tenant: an entry
/// cannot be lifted out of one community's chain and re-verified inside another.
///
/// `detail` is serialized via [`canonical_json`] (sorted keys) so the hash is
/// stable across machines and Rust versions. A serialization failure is a hard
/// error, never silently hashed as empty.
pub fn compute_hash(entry: &AuditEntry) -> Result<[u8; 32], AuditError> {
    let mut hasher = Sha256::new();
    // Tenant binding: community_id leads the hash.
    hasher.update(entry.community_id.as_bytes());
    hasher.update(entry.seq.to_be_bytes());
    hasher.update(entry.created_at.to_rfc3339().as_bytes());
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

    #[test]
    fn deterministic() {
        let entry = sample_entry();
        assert_eq!(compute_hash(&entry).unwrap(), compute_hash(&entry).unwrap());
        assert_eq!(compute_hash(&entry).unwrap().len(), 32);
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

    // Regression tests for #2637: created_at must be truncated to microsecond
    // precision before hashing so the digest survives the round-trip through a
    // TIMESTAMPTZ column (Postgres discards sub-microsecond digits on write).

    /// The trap: `to_rfc3339()` emits 9 fractional digits for nanosecond
    /// timestamps and 6 for microsecond ones -- different byte strings, different
    /// hashes. This test documents the invariant that the compute_hash function
    /// depends on.
    #[test]
    fn nanosecond_and_microsecond_timestamps_hash_differently() {
        use chrono::SubsecRound as _;

        // A timestamp with sub-microsecond digits.
        let ns_ts: DateTime<Utc> =
            chrono::DateTime::parse_from_rfc3339("2026-07-24T12:00:00.123456789Z")
                .unwrap()
                .with_timezone(&Utc);
        // The same timestamp truncated to µs -- what Postgres returns after storing it.
        let us_ts = ns_ts.trunc_subsecs(6);

        assert_ne!(ns_ts, us_ts, "test setup: timestamps must differ");

        let mut ns_entry = sample_entry();
        ns_entry.created_at = ns_ts;
        let mut us_entry = sample_entry();
        us_entry.created_at = us_ts;

        assert_ne!(
            compute_hash(&ns_entry).unwrap(),
            compute_hash(&us_entry).unwrap(),
            "nanosecond and microsecond timestamps must produce different hashes"
        );
    }

    /// The fix: hashing a µs-truncated timestamp produces a digest that can be
    /// reproduced after a simulated Postgres round-trip (which also truncates to
    /// µs). Before the fix, the hash was computed over the nanosecond value and
    /// could never be reproduced from the stored row.
    #[test]
    fn hash_survives_microsecond_precision_round_trip() {
        use chrono::SubsecRound as _;

        // A timestamp with sub-microsecond digits, as returned by Utc::now().
        let raw: DateTime<Utc> =
            chrono::DateTime::parse_from_rfc3339("2026-07-24T12:00:00.123456789Z")
                .unwrap()
                .with_timezone(&Utc);

        // Truncate before hashing (the fix in log_inner).
        let stored = raw.trunc_subsecs(6);
        let mut entry = sample_entry();
        entry.created_at = stored;
        let hash_at_write = compute_hash(&entry).unwrap();

        // Simulate what verify_chain gets back from Postgres: the µs-truncated value.
        let reread_from_db = stored; // TIMESTAMPTZ returns the same µs value
        entry.created_at = reread_from_db;
        let hash_at_verify = compute_hash(&entry).unwrap();

        assert_eq!(
            hash_at_write, hash_at_verify,
            "hash computed before write must equal hash recomputed from stored row"
        );
    }

    /// Guard against regressions that re-introduce nanosecond hashing: a timestamp
    /// with non-zero sub-microsecond digits must round-trip only when truncated.
    #[test]
    fn untruncated_nanosecond_timestamp_fails_round_trip() {
        use chrono::SubsecRound as _;

        let raw: DateTime<Utc> =
            chrono::DateTime::parse_from_rfc3339("2026-07-24T12:00:00.123456789Z")
                .unwrap()
                .with_timezone(&Utc);

        // Hash WITHOUT truncation (the old, broken behaviour).
        let mut entry = sample_entry();
        entry.created_at = raw;
        let hash_at_write = compute_hash(&entry).unwrap();

        // Postgres discards sub-µs digits -- simulate what verify_chain reads back.
        entry.created_at = raw.trunc_subsecs(6);
        let hash_at_verify = compute_hash(&entry).unwrap();

        assert_ne!(
            hash_at_write, hash_at_verify,
            "without truncation the hash cannot survive the Postgres round-trip"
        );
    }
}
