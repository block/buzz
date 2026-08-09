use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{evidence_is_fresh, exact_object, text, EvidenceRejection, MAX_EVIDENCE_BYTES};

pub(super) fn validate(
    value: &Value,
    active_node_id: &str,
    now: DateTime<Utc>,
    maximum_age_seconds: i64,
) -> Result<(), EvidenceRejection> {
    if value.get("schema").and_then(Value::as_str) == Some("memory-evidence-v1") {
        return validate_evidence_wrapper(value, active_node_id, now, maximum_age_seconds);
    }
    if let Some(result) = value.get("result") {
        return validate(result, active_node_id, now, maximum_age_seconds);
    }
    if let Some(items) = value.as_array() {
        if items.is_empty() || items.len() > 200 {
            return Err(EvidenceRejection::InvalidShape);
        }
        for item in items {
            validate(item, active_node_id, now, maximum_age_seconds)?;
        }
        return Ok(());
    }
    let object = value.as_object().ok_or(EvidenceRejection::InvalidShape)?;
    match object.get("kind").and_then(Value::as_str) {
        Some("memory-revision") => {
            verify_revision(value)?;
            evidence_is_fresh(text(object, "timestamp")?, now, maximum_age_seconds)
        }
        Some("replication-envelope") => {
            verify_envelope(value)?;
            evidence_is_fresh(text(object, "timestamp")?, now, maximum_age_seconds)
        }
        Some(_) => Err(EvidenceRejection::InvalidShape),
        None => validate_context(value, now, maximum_age_seconds),
    }
}

fn validate_evidence_wrapper(
    value: &Value,
    active_node_id: &str,
    now: DateTime<Utc>,
    maximum_age_seconds: i64,
) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &[
            "schema",
            "tool_policy",
            "serving_node_id",
            "retrieved_at",
            "total",
            "results",
        ],
    )?;
    let policy = exact_object(
        object
            .get("tool_policy")
            .ok_or(EvidenceRejection::InvalidShape)?,
        &["mode", "retrieved_content", "instruction_effect"],
    )?;
    if text(object, "schema")? != "memory-evidence-v1"
        || text(policy, "mode")? != "read_only"
        || text(policy, "retrieved_content")? != "untrusted_evidence"
        || text(policy, "instruction_effect")? != "none"
        || text(object, "serving_node_id")? != active_node_id
    {
        return Err(EvidenceRejection::InvalidShape);
    }
    let retrieved_at = text(object, "retrieved_at")?;
    evidence_is_fresh(retrieved_at, now, maximum_age_seconds)?;
    let results = object
        .get("results")
        .and_then(Value::as_array)
        .filter(|results| results.len() <= 200)
        .ok_or(EvidenceRejection::InvalidShape)?;
    if object.get("total").and_then(Value::as_u64) != Some(results.len() as u64) {
        return Err(EvidenceRejection::InvalidShape);
    }
    for result in results {
        validate_evidence_result(result)?;
    }
    Ok(())
}

fn validate_evidence_result(value: &Value) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &[
            "untrusted_evidence",
            "revision",
            "replication_envelope",
            "conflicted_fields",
            "quoted_text",
            "citation",
        ],
    )?;
    if object.get("untrusted_evidence").and_then(Value::as_bool) != Some(true)
        || text(object, "quoted_text")?.len() > 1024 * 1024
    {
        return Err(EvidenceRejection::InvalidShape);
    }
    if object
        .get("conflicted_fields")
        .and_then(Value::as_array)
        .is_none_or(|fields| !fields.is_empty())
    {
        return Err(EvidenceRejection::ConflictedMemory);
    }
    let revision = object
        .get("revision")
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    verify_revision(revision)?;
    if let Some(envelope) = object
        .get("replication_envelope")
        .filter(|envelope| !envelope.is_null())
    {
        verify_envelope(envelope)?;
        if envelope.get("payload") != Some(revision) {
            return Err(EvidenceRejection::IntegrityFailure);
        }
    }
    let revision = revision
        .as_object()
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    let hashes = revision
        .get("hashes")
        .and_then(Value::as_object)
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    if revision
        .get("content")
        .and_then(Value::as_object)
        .and_then(|content| content.get("content"))
        .and_then(Value::as_str)
        != object.get("quoted_text").and_then(Value::as_str)
    {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let citation = exact_object(
        object
            .get("citation")
            .ok_or(EvidenceRejection::MissingCitation)?,
        &["event_id", "revision_hash", "node_id", "timestamp"],
    )
    .map_err(|_| EvidenceRejection::MissingCitation)?;
    if text(citation, "event_id").map_err(|_| EvidenceRejection::MissingCitation)?
        != text(revision, "eventId").map_err(|_| EvidenceRejection::MissingCitation)?
        || text(citation, "revision_hash").map_err(|_| EvidenceRejection::MissingCitation)?
            != text(hashes, "revision").map_err(|_| EvidenceRejection::MissingCitation)?
        || text(citation, "node_id").map_err(|_| EvidenceRejection::MissingCitation)?
            != text(revision, "nodeId").map_err(|_| EvidenceRejection::MissingCitation)?
        || text(citation, "timestamp").map_err(|_| EvidenceRejection::MissingCitation)?
            != text(revision, "timestamp").map_err(|_| EvidenceRejection::MissingCitation)?
    {
        return Err(EvidenceRejection::MissingCitation);
    }
    Ok(())
}

fn validate_context(
    value: &Value,
    now: DateTime<Utc>,
    maximum_age_seconds: i64,
) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &["entity_id", "content", "conflicted_fields", "citation"],
    )?;
    if text(object, "entity_id")?.len() > 256
        || !object.get("content").is_some_and(Value::is_object)
    {
        return Err(EvidenceRejection::InvalidShape);
    }
    if object
        .get("conflicted_fields")
        .and_then(Value::as_array)
        .is_none_or(|fields| !fields.is_empty())
    {
        return Err(EvidenceRejection::ConflictedMemory);
    }
    let citation = exact_object(
        object
            .get("citation")
            .ok_or(EvidenceRejection::MissingCitation)?,
        &["event_id", "revision_hash", "node_id", "timestamp"],
    )
    .map_err(|_| EvidenceRejection::MissingCitation)?;
    let event_id = text(citation, "event_id").map_err(|_| EvidenceRejection::MissingCitation)?;
    if !valid_sha256_identifier(event_id)
        || text(citation, "revision_hash").map_err(|_| EvidenceRejection::MissingCitation)?
            != event_id
        || !valid_node_id(
            text(citation, "node_id").map_err(|_| EvidenceRejection::MissingCitation)?,
        )
    {
        return Err(EvidenceRejection::MissingCitation);
    }
    evidence_is_fresh(
        text(citation, "timestamp").map_err(|_| EvidenceRejection::MissingCitation)?,
        now,
        maximum_age_seconds,
    )
}

fn agent_memory_canonical_json(value: &Value) -> Result<Vec<u8>, EvidenceRejection> {
    buzz_core::agent_memory_canonical::canonical_json_bytes(value, MAX_EVIDENCE_BYTES)
        .map_err(|_| EvidenceRejection::IntegrityFailure)
}

fn sha256_identifier(value: &Value) -> Result<String, EvidenceRejection> {
    Ok(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(agent_memory_canonical_json(value)?))
    ))
}

fn verify_revision(value: &Value) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &[
            "kind",
            "version",
            "classification",
            "entityId",
            "eventId",
            "parentRevisionIds",
            "nodeId",
            "timestamp",
            "hashes",
            "tombstone",
            "cursor",
            "content",
        ],
    )
    .map_err(|_| EvidenceRejection::IntegrityFailure)?;
    if object.get("version").and_then(Value::as_u64) != Some(1)
        || !matches!(
            object.get("classification").and_then(Value::as_str),
            Some("PUBLIC" | "OFFICIAL")
        )
        || object.get("tombstone").and_then(Value::as_bool) != Some(false)
    {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let entity_id = text(object, "entityId").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let node_id = text(object, "nodeId").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let timestamp = text(object, "timestamp").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let event_id = text(object, "eventId").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let cursor = text(object, "cursor").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    if !valid_node_id(node_id)
        || DateTime::parse_from_rfc3339(timestamp).is_err()
        || cursor.parse::<u64>().is_err()
        || !valid_sha256_identifier(event_id)
    {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let content = object
        .get("content")
        .filter(|value| value.is_object())
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    let subject_type = if valid_ulid(entity_id) {
        "event"
    } else {
        "entity"
    };
    if subject_type == "event" && content.get("id").and_then(Value::as_str) != Some(entity_id) {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let hashes = exact_object(
        object
            .get("hashes")
            .ok_or(EvidenceRejection::IntegrityFailure)?,
        &["content", "revision"],
    )
    .map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let content_hash = text(hashes, "content").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let revision_hash =
        text(hashes, "revision").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    if sha256_identifier(&serde_json::json!({
        "schema_version": 1,
        "kind": subject_type,
        "payload": content,
    }))? != content_hash
    {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let parents = object
        .get("parentRevisionIds")
        .and_then(Value::as_array)
        .filter(|parents| parents.len() <= 32)
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    let mut previous: Option<&str> = None;
    for parent in parents {
        let parent = parent
            .as_str()
            .filter(|parent| valid_sha256_identifier(parent))
            .ok_or(EvidenceRejection::IntegrityFailure)?;
        if previous.is_some_and(|previous| previous >= parent) {
            return Err(EvidenceRejection::IntegrityFailure);
        }
        previous = Some(parent);
    }
    let recomputed = sha256_identifier(&serde_json::json!({
        "schema_version": 1,
        "node_id": node_id,
        "subject_type": subject_type,
        "subject_id": entity_id,
        "object_id": content_hash,
        "parent_ids": parents,
        "created_at": timestamp,
    }))?;
    if recomputed != revision_hash || recomputed != event_id {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    Ok(())
}

fn verify_envelope(value: &Value) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &[
            "kind",
            "version",
            "classification",
            "entityId",
            "eventId",
            "parentRevisionIds",
            "nodeId",
            "timestamp",
            "hashes",
            "tombstone",
            "cursor",
            "payload",
        ],
    )
    .map_err(|_| EvidenceRejection::IntegrityFailure)?;
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let payload = object
        .get("payload")
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    verify_revision(payload)?;
    let payload_object = payload
        .as_object()
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    let payload_hashes = payload_object
        .get("hashes")
        .and_then(Value::as_object)
        .ok_or(EvidenceRejection::IntegrityFailure)?;
    let hashes = exact_object(
        object
            .get("hashes")
            .ok_or(EvidenceRejection::IntegrityFailure)?,
        &["payload", "envelope"],
    )
    .map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let payload_event =
        text(payload_object, "eventId").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    let payload_cursor =
        text(payload_object, "cursor").map_err(|_| EvidenceRejection::IntegrityFailure)?;
    if text(object, "eventId").map_err(|_| EvidenceRejection::IntegrityFailure)?
        != format!("replication:{payload_event}:{payload_cursor}")
        || text(object, "entityId").map_err(|_| EvidenceRejection::IntegrityFailure)?
            != text(payload_object, "entityId").map_err(|_| EvidenceRejection::IntegrityFailure)?
        || text(object, "nodeId").map_err(|_| EvidenceRejection::IntegrityFailure)?
            != text(payload_object, "nodeId").map_err(|_| EvidenceRejection::IntegrityFailure)?
        || text(object, "cursor").map_err(|_| EvidenceRejection::IntegrityFailure)?
            != payload_cursor
        || text(hashes, "payload").map_err(|_| EvidenceRejection::IntegrityFailure)?
            != text(payload_hashes, "revision").map_err(|_| EvidenceRejection::IntegrityFailure)?
    {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    let mut basis = object.clone();
    basis
        .get_mut("hashes")
        .and_then(Value::as_object_mut)
        .ok_or(EvidenceRejection::IntegrityFailure)?
        .remove("envelope");
    if sha256_identifier(&Value::Object(basis))?
        != text(hashes, "envelope").map_err(|_| EvidenceRejection::IntegrityFailure)?
    {
        return Err(EvidenceRejection::IntegrityFailure);
    }
    Ok(())
}

fn valid_node_id(value: &str) -> bool {
    value.strip_prefix("node:").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 127
            && suffix.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
            })
    })
}

fn valid_sha256_identifier(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn valid_ulid(value: &str) -> bool {
    const ULID: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    value.len() == 26
        && value
            .bytes()
            .all(|byte| ULID.as_bytes().contains(&byte.to_ascii_uppercase()))
}
