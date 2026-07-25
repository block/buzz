use super::*;

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MemoryEvidenceError {
    Invalid,
}

/// One cryptographically verified result from Phase 3 `memory-evidence-v1`.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug)]
pub(crate) struct VerifiedMemoryEvidence {
    entity_id: String,
    quoted_text: String,
    revision_hash: String,
    origin_node_id: String,
    origin_timestamp: String,
    serving_node_id: String,
    retrieved_at: String,
    cursor: u64,
    envelope_hash: String,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl VerifiedMemoryEvidence {
    pub(crate) fn entity_id(&self) -> &str {
        &self.entity_id
    }

    pub(crate) fn quoted_text(&self) -> &str {
        &self.quoted_text
    }

    pub(crate) fn revision_hash(&self) -> &str {
        &self.revision_hash
    }

    pub(crate) fn origin_node_id(&self) -> &str {
        &self.origin_node_id
    }

    pub(crate) fn origin_timestamp(&self) -> &str {
        &self.origin_timestamp
    }

    pub(crate) fn serving_node_id(&self) -> &str {
        &self.serving_node_id
    }

    pub(crate) fn retrieved_at(&self) -> &str {
        &self.retrieved_at
    }

    pub(crate) const fn cursor(&self) -> u64 {
        self.cursor
    }

    pub(crate) fn envelope_hash(&self) -> &str {
        &self.envelope_hash
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
pub(crate) fn extract_verified_memory_evidence(
    value: &Value,
) -> Result<Vec<VerifiedMemoryEvidence>, MemoryEvidenceError> {
    let object = value.as_object().ok_or(MemoryEvidenceError::Invalid)?;
    let wrapper_keys = [
        "schema",
        "tool_policy",
        "serving_node_id",
        "retrieved_at",
        "total",
        "results",
    ];
    if object.len() != wrapper_keys.len()
        || wrapper_keys.iter().any(|key| !object.contains_key(*key))
        || object.get("schema").and_then(Value::as_str) != Some("memory-evidence-v1")
    {
        return Err(MemoryEvidenceError::Invalid);
    }
    let policy = object
        .get("tool_policy")
        .and_then(Value::as_object)
        .ok_or(MemoryEvidenceError::Invalid)?;
    if policy.len() != 3
        || policy.get("mode").and_then(Value::as_str) != Some("read_only")
        || policy.get("retrieved_content").and_then(Value::as_str) != Some("untrusted_evidence")
        || policy.get("instruction_effect").and_then(Value::as_str) != Some("none")
    {
        return Err(MemoryEvidenceError::Invalid);
    }
    let serving_node_id = object
        .get("serving_node_id")
        .and_then(Value::as_str)
        .filter(|value| valid_memory_node_id(value))
        .ok_or(MemoryEvidenceError::Invalid)?;
    let retrieved_at = object
        .get("retrieved_at")
        .and_then(Value::as_str)
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .ok_or(MemoryEvidenceError::Invalid)?;
    let results = object
        .get("results")
        .and_then(Value::as_array)
        .filter(|results| results.len() <= 20)
        .ok_or(MemoryEvidenceError::Invalid)?;
    if object.get("total").and_then(Value::as_u64) != Some(results.len() as u64) {
        return Err(MemoryEvidenceError::Invalid);
    }
    results
        .iter()
        .map(|result| {
            let result = result.as_object().ok_or(MemoryEvidenceError::Invalid)?;
            let result_keys = [
                "untrusted_evidence",
                "revision",
                "replication_envelope",
                "conflicted_fields",
                "quoted_text",
                "citation",
            ];
            if result.len() != result_keys.len()
                || result_keys.iter().any(|key| !result.contains_key(*key))
                || result.get("untrusted_evidence").and_then(Value::as_bool) != Some(true)
                || result
                    .get("conflicted_fields")
                    .and_then(Value::as_array)
                    .is_none_or(|fields| !fields.is_empty())
            {
                return Err(MemoryEvidenceError::Invalid);
            }
            let revision = result.get("revision").ok_or(MemoryEvidenceError::Invalid)?;
            let envelope = result
                .get("replication_envelope")
                .ok_or(MemoryEvidenceError::Invalid)?;
            crate::command_services::policy::verify_memory_revision(revision)
                .map_err(|_| MemoryEvidenceError::Invalid)?;
            crate::command_services::policy::verify_replication_envelope(envelope)
                .map_err(|_| MemoryEvidenceError::Invalid)?;
            let revision = revision.as_object().ok_or(MemoryEvidenceError::Invalid)?;
            let envelope = envelope.as_object().ok_or(MemoryEvidenceError::Invalid)?;
            if revision.get("classification").and_then(Value::as_str) != Some("OFFICIAL")
                || envelope.get("payload") != result.get("revision")
            {
                return Err(MemoryEvidenceError::Invalid);
            }
            let entity_id = memory_text(revision, "entityId")?;
            let revision_hash = revision
                .get("hashes")
                .and_then(Value::as_object)
                .and_then(|hashes| hashes.get("revision"))
                .and_then(Value::as_str)
                .ok_or(MemoryEvidenceError::Invalid)?;
            let origin_node_id = memory_text(revision, "nodeId")?;
            let origin_timestamp = memory_text(revision, "timestamp")?;
            let cursor = memory_text(revision, "cursor")?
                .parse::<u64>()
                .ok()
                .filter(|cursor| *cursor > 0)
                .ok_or(MemoryEvidenceError::Invalid)?;
            let quoted_text = result
                .get("quoted_text")
                .and_then(Value::as_str)
                .filter(|quote| !quote.trim().is_empty() && quote.len() <= 64 * 1024)
                .ok_or(MemoryEvidenceError::Invalid)?;
            if revision
                .get("content")
                .and_then(Value::as_object)
                .and_then(|content| content.get("content"))
                .and_then(Value::as_str)
                != Some(quoted_text)
            {
                return Err(MemoryEvidenceError::Invalid);
            }
            let citation = result
                .get("citation")
                .and_then(Value::as_object)
                .ok_or(MemoryEvidenceError::Invalid)?;
            let citation_keys = ["event_id", "revision_hash", "node_id", "timestamp"];
            if citation.len() != citation_keys.len()
                || citation_keys.iter().any(|key| !citation.contains_key(*key))
                || citation.get("event_id").and_then(Value::as_str) != Some(revision_hash)
                || citation.get("revision_hash").and_then(Value::as_str) != Some(revision_hash)
                || citation.get("node_id").and_then(Value::as_str) != Some(origin_node_id)
                || citation.get("timestamp").and_then(Value::as_str) != Some(origin_timestamp)
            {
                return Err(MemoryEvidenceError::Invalid);
            }
            let envelope_hash = envelope
                .get("hashes")
                .and_then(Value::as_object)
                .and_then(|hashes| hashes.get("envelope"))
                .and_then(Value::as_str)
                .ok_or(MemoryEvidenceError::Invalid)?;
            Ok(VerifiedMemoryEvidence {
                entity_id: entity_id.to_string(),
                quoted_text: quoted_text.to_string(),
                revision_hash: revision_hash.to_string(),
                origin_node_id: origin_node_id.to_string(),
                origin_timestamp: origin_timestamp.to_string(),
                serving_node_id: serving_node_id.to_string(),
                retrieved_at: retrieved_at.to_string(),
                cursor,
                envelope_hash: envelope_hash.to_string(),
            })
        })
        .collect()
}

fn memory_text<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, MemoryEvidenceError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(MemoryEvidenceError::Invalid)
}

fn valid_memory_node_id(value: &str) -> bool {
    value.strip_prefix("node:").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 127
            && suffix.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
            })
    })
}
