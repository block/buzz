use super::*;

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RagSnapshotError {
    Invalid,
    Changed,
}

/// A cryptographically verified active RAG snapshot frozen for one brief run.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedRagSnapshot {
    snapshot_id: String,
    verified_at: String,
    snapshot_time: String,
    physical_collections: Vec<String>,
    logical_collections: Vec<String>,
}

/// Cryptographically verified RAG snapshot and re-attested read service used
/// by the production command-brief source backend.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "Task 8 installs the production command-brief source backend"
)]
pub(crate) struct RagSourceBinding {
    pub(crate) snapshot: VerifiedRagSnapshot,
    pub(crate) service: AuthenticatedSourceService,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl VerifiedRagSnapshot {
    pub(crate) fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    pub(crate) fn verified_at(&self) -> &str {
        &self.verified_at
    }

    pub(crate) fn snapshot_time(&self) -> &str {
        &self.snapshot_time
    }

    pub(crate) fn physical_collections(&self) -> &[String] {
        &self.physical_collections
    }

    pub(crate) fn logical_collections(&self) -> &[String] {
        &self.logical_collections
    }

    pub(crate) fn verify_unchanged(
        &self,
        observed_snapshot_id: &str,
    ) -> Result<(), RagSnapshotError> {
        if !valid_digest(observed_snapshot_id) {
            return Err(RagSnapshotError::Invalid);
        }
        if self.snapshot_id != observed_snapshot_id {
            return Err(RagSnapshotError::Changed);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_test(snapshot_id: &str, verified_at: &str, snapshot_time: &str) -> Self {
        Self {
            snapshot_id: snapshot_id.to_string(),
            verified_at: verified_at.to_string(),
            snapshot_time: snapshot_time.to_string(),
            physical_collections: vec!["documents".to_string()],
            logical_collections: vec!["navy-publications".to_string()],
        }
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
pub(crate) fn verified_snapshot_from_readiness(
    readiness: &RagServiceReadiness,
) -> Result<VerifiedRagSnapshot, RagSnapshotError> {
    if readiness.status != RagServiceStatus::Ready
        || readiness.validation != RagValidationState::Verified
        || readiness.freshness != RagFreshness::Fresh
    {
        return Err(RagSnapshotError::Invalid);
    }
    let snapshot_id = readiness
        .active_snapshot_id
        .as_deref()
        .filter(|value| valid_digest(value))
        .ok_or(RagSnapshotError::Invalid)?;
    let snapshot_time = readiness
        .snapshot_time
        .as_deref()
        .filter(|value| DateTime::parse_from_rfc3339(value).is_ok())
        .ok_or(RagSnapshotError::Invalid)?;
    if DateTime::parse_from_rfc3339(&readiness.observed_at).is_err() {
        return Err(RagSnapshotError::Invalid);
    }
    Ok(VerifiedRagSnapshot {
        snapshot_id: snapshot_id.to_string(),
        verified_at: readiness.observed_at.clone(),
        snapshot_time: snapshot_time.to_string(),
        physical_collections: readiness.verified_physical_collections.clone(),
        logical_collections: readiness.verified_logical_collections.clone(),
    })
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RagEvidenceRecord {
    pub(crate) source_id: String,
    pub(crate) collection: String,
    pub(crate) document_id: String,
    pub(crate) chunk_id: String,
    pub(crate) retrieved_at: String,
    pub(crate) location: String,
    pub(crate) quote: String,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
pub(crate) fn extract_verified_rag_evidence(
    snapshot: &VerifiedRagSnapshot,
    expected_query: &str,
    value: &Value,
) -> Result<Vec<RagEvidenceRecord>, RagSnapshotError> {
    let policy = crate::command_services::policy::AdviserContextPolicy {
        active_snapshot_id: snapshot.snapshot_id.clone(),
        allowed_apple_ids: std::collections::BTreeSet::new(),
        allowed_file_paths: std::collections::BTreeSet::new(),
    };
    crate::command_services::policy::validate_rag_context(&policy, value)
        .map_err(|_| RagSnapshotError::Invalid)?;
    if value.get("query").and_then(Value::as_str) != Some(expected_query) {
        return Err(RagSnapshotError::Invalid);
    }
    let retrieved_at = value
        .get("retrieved_at")
        .and_then(Value::as_str)
        .ok_or(RagSnapshotError::Invalid)?;
    value
        .get("results")
        .and_then(Value::as_array)
        .ok_or(RagSnapshotError::Invalid)?
        .iter()
        .map(|result| {
            let source = result
                .get("source")
                .and_then(Value::as_object)
                .ok_or(RagSnapshotError::Invalid)?;
            if source
                .get("collection")
                .and_then(Value::as_str)
                .is_none_or(|collection| {
                    !snapshot
                        .logical_collections
                        .iter()
                        .any(|allowed| allowed == collection)
                })
            {
                return Err(RagSnapshotError::Invalid);
            }
            let text = |key: &str| {
                source
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or(RagSnapshotError::Invalid)
            };
            let location = serde_jcs::to_vec(
                source
                    .get("quoted_location")
                    .ok_or(RagSnapshotError::Invalid)?,
            )
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(RagSnapshotError::Invalid)?;
            Ok(RagEvidenceRecord {
                source_id: text("source_id")?,
                collection: text("collection")?,
                document_id: text("document_id")?,
                chunk_id: text("chunk_id")?,
                retrieved_at: retrieved_at.to_string(),
                location,
                quote: result
                    .get("quoted_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or(RagSnapshotError::Invalid)?,
            })
        })
        .collect()
}
