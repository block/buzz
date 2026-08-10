use thiserror::Error;

/// Errors that can occur during audit log operations.
///
/// These are **operator-internal** diagnostics (logged by the audit worker, or
/// returned to an operator-scoped verification call) — they are never relayed to
/// a client on the wire. Even so, no variant embeds a `community_id` or any
/// cross-community object identifier: a `seq` is per-community and meaningless
/// without its chain, and hashes are opaque. An error raised while verifying
/// community A's chain therefore cannot reveal a fact about community B.
#[derive(Debug, Error)]
pub enum AuditError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// The `prev_hash` of an entry does not match the hash of the preceding
    /// entry in the same community's chain.
    #[error(
        "hash chain integrity violation at seq {seq}: prev_hash does not match preceding entry"
    )]
    ChainViolation {
        /// Per-community sequence number of the offending entry.
        seq: i64,
    },

    /// The stored hash of an entry does not match the recomputed hash.
    #[error("hash mismatch at seq {seq}: stored hash does not match recomputed hash")]
    HashMismatch {
        /// Per-community sequence number of the offending entry.
        seq: i64,
    },

    /// A verification that should have started at the chain's genesis found a
    /// different first entry — the earliest entries have been removed.
    #[error("chain does not start at genesis: expected seq 1 with no predecessor, found seq {found_seq} first")]
    MissingGenesis {
        /// Per-community sequence number of the first entry actually found.
        found_seq: i64,
    },

    /// A mid-chain verification could not find the entry immediately before
    /// its range. In an append-only chain every predecessor must exist, so a
    /// missing anchor means entries in front of the range were removed.
    #[error("missing anchor entry at seq {anchor_seq}: the verified segment cannot be linked to the preceding chain")]
    MissingAnchor {
        /// Per-community sequence number of the absent anchor entry.
        anchor_seq: i64,
    },

    /// Sequence numbers stopped being contiguous — one or more entries were
    /// removed from the middle of the chain.
    #[error("chain gap: expected seq {expected_seq} next but found seq {found_seq}")]
    SequenceGap {
        /// Per-community sequence number that should have come next.
        expected_seq: i64,
        /// Per-community sequence number actually found.
        found_seq: i64,
    },

    /// The verified chain head is behind the head the caller expected (from an
    /// externally recorded report) — the newest entries have been removed.
    #[error("chain head at seq {head_seq} is behind the expected head seq {expected_seq}: tail entries are missing")]
    TruncatedTail {
        /// Per-community sequence number of the verified head.
        head_seq: i64,
        /// Per-community sequence number the caller expected the head to reach.
        expected_seq: i64,
    },

    /// An unrecognised action string was found in the database.
    #[error("unknown audit action in database")]
    UnknownAction,

    /// A JSON serialization error occurred (e.g. while canonicalising `detail`).
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sanitization obligation for the conformance `audit_log` row: an error
    /// raised while verifying or appending to one community's chain must not let
    /// its rendered text become a cross-community identifier — no `community_id`,
    /// no constraint name. Only `seq` may appear, and `seq` is per-community and
    /// meaningless without the chain it indexes.
    ///
    /// This is the *complement* to the structural fence in the variant
    /// definitions above: those variants simply have no `community_id` field, so
    /// there is no slot to leak one from. This test pins the observable form —
    /// if anyone adds a `community_id` to a variant and threads it into the
    /// `#[error(...)]` format string, the assertion below reds.
    #[test]
    fn audit_error_text_carries_no_community_id_or_constraint() {
        // A concrete community whose chain is "being verified" when these errors
        // fire. If its id leaked into any error text, the error would identify a
        // specific tenant.
        let community = uuid::Uuid::new_v4();
        let community_str = community.to_string();
        let community_simple = community.simple().to_string();

        // The variants the audit crate constructs itself with chain-derived data.
        let domain_errors = [
            AuditError::ChainViolation { seq: 7 },
            AuditError::HashMismatch { seq: 42 },
            AuditError::MissingGenesis { found_seq: 9 },
            AuditError::MissingAnchor { anchor_seq: 4 },
            AuditError::SequenceGap {
                expected_seq: 5,
                found_seq: 8,
            },
            AuditError::TruncatedTail {
                head_seq: 11,
                expected_seq: 15,
            },
            AuditError::UnknownAction,
        ];

        for err in &domain_errors {
            let text = err.to_string();

            // No form of the community id may appear.
            assert!(
                !text.contains(&community_str) && !text.contains(&community_simple),
                "audit error text leaked a community_id: {text:?}"
            );

            // No Postgres constraint/PK names that would reveal schema shape or
            // the existence of a cross-community key.
            for needle in [
                "community_id",
                "audit_log_pkey",
                "constraint",
                "communities",
            ] {
                assert!(
                    !text.to_ascii_lowercase().contains(needle),
                    "audit error text leaked a constraint/identifier '{needle}': {text:?}"
                );
            }
        }

        // The two chain-integrity variants must still carry their per-community
        // `seq` (the diagnostic is useless without it) — proves the assertion
        // above isn't vacuously passing on empty strings.
        assert!(AuditError::ChainViolation { seq: 7 }
            .to_string()
            .contains('7'));
        assert!(AuditError::HashMismatch { seq: 42 }
            .to_string()
            .contains("42"));
    }
}
