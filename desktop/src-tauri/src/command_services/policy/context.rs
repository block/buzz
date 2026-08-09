use super::{MAXIMUM_CANONICAL_BYTES, MAXIMUM_JSON_DEPTH, MAXIMUM_JSON_NODES};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
pub(crate) enum IntegrityError {
    InvalidShape,
    CanonicalEncoding,
    ContentHashMismatch,
    RevisionHashMismatch,
    EnvelopeHashMismatch,
    UnverifiableTombstone,
}

pub(crate) fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, IntegrityError> {
    fn write(
        value: &Value,
        output: &mut Vec<u8>,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<(), IntegrityError> {
        if depth > MAXIMUM_JSON_DEPTH || *nodes >= MAXIMUM_JSON_NODES {
            return Err(IntegrityError::CanonicalEncoding);
        }
        *nodes += 1;
        match value {
            Value::Null => output.extend_from_slice(b"null"),
            Value::Bool(true) => output.extend_from_slice(b"true"),
            Value::Bool(false) => output.extend_from_slice(b"false"),
            Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
            Value::String(text) => output.extend_from_slice(
                serde_json::to_string(text)
                    .map_err(|_| IntegrityError::CanonicalEncoding)?
                    .as_bytes(),
            ),
            Value::Array(items) => {
                output.push(b'[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(item, output, depth + 1, nodes)?;
                }
                output.push(b']');
            }
            Value::Object(object) => {
                output.push(b'{');
                let mut keys: Vec<&String> = object.keys().collect();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend_from_slice(
                        serde_json::to_string(key)
                            .map_err(|_| IntegrityError::CanonicalEncoding)?
                            .as_bytes(),
                    );
                    output.push(b':');
                    write(&object[key], output, depth + 1, nodes)?;
                }
                output.push(b'}');
            }
        }
        if output.len() > MAXIMUM_CANONICAL_BYTES {
            return Err(IntegrityError::CanonicalEncoding);
        }
        Ok(())
    }

    let mut output = Vec::new();
    let mut nodes = 0;
    write(value, &mut output, 0, &mut nodes)?;
    Ok(output)
}

pub(crate) fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn sha256_identifier(value: &Value) -> Result<String, IntegrityError> {
    Ok(format!(
        "sha256:{}",
        sha256_hex(&canonical_json_bytes(value)?)
    ))
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, IntegrityError> {
    let object = value.as_object().ok_or(IntegrityError::InvalidShape)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(IntegrityError::InvalidShape);
    }
    Ok(object)
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, IntegrityError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .ok_or(IntegrityError::InvalidShape)
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn inferred_subject_type(entity_id: &str) -> &'static str {
    if valid_ulid(entity_id) {
        "event"
    } else {
        "entity"
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn valid_ulid(value: &str) -> bool {
    const ULID: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    value.len() == 26
        && value
            .bytes()
            .all(|byte| ULID.as_bytes().contains(&byte.to_ascii_uppercase()))
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn valid_node_id(value: &str) -> bool {
    value.strip_prefix("node:").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 127
            && suffix.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
            })
    })
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
pub(crate) fn verify_memory_revision(value: &Value) -> Result<(), IntegrityError> {
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
    )?;
    if text(object, "kind")? != "memory-revision"
        || object.get("version").and_then(Value::as_u64) != Some(1)
        || !matches!(text(object, "classification")?, "PUBLIC" | "OFFICIAL")
        || chrono::DateTime::parse_from_rfc3339(text(object, "timestamp")?).is_err()
        || text(object, "cursor")?.parse::<u64>().is_err()
    {
        return Err(IntegrityError::InvalidShape);
    }
    let tombstone = object
        .get("tombstone")
        .and_then(Value::as_bool)
        .ok_or(IntegrityError::InvalidShape)?;
    if tombstone {
        // The v1 wire contract deliberately redacts the canonical tombstone
        // payload, so its object hash cannot be independently recomputed.
        return Err(IntegrityError::UnverifiableTombstone);
    }
    let content = object.get("content").ok_or(IntegrityError::InvalidShape)?;
    if !content.is_object() {
        return Err(IntegrityError::InvalidShape);
    }
    let entity_id = text(object, "entityId")?;
    let node_id = text(object, "nodeId")?;
    let timestamp = text(object, "timestamp")?;
    let event_id = text(object, "eventId")?;
    if entity_id.len() > 256
        || !valid_node_id(node_id)
        || timestamp.len() > 64
        || !valid_sha256_identifier(event_id)
    {
        return Err(IntegrityError::InvalidShape);
    }
    let hashes = exact_object(
        object.get("hashes").ok_or(IntegrityError::InvalidShape)?,
        &["content", "revision"],
    )?;
    let claimed_content = text(hashes, "content")?;
    let claimed_revision = text(hashes, "revision")?;
    let subject_type = inferred_subject_type(entity_id);
    // AgentMemory's Buzz adapter deliberately overloads `eventId`: the outer
    // wire field is the canonical revision SHA-256, while an event subject's
    // globally unique ULID remains in both `entityId` and `content.id`.
    if subject_type == "event" && content.get("id").and_then(Value::as_str) != Some(entity_id) {
        return Err(IntegrityError::InvalidShape);
    }
    let content_basis = serde_json::json!({
        "schema_version": 1,
        "kind": subject_type,
        "payload": content,
    });
    if sha256_identifier(&content_basis)? != claimed_content {
        return Err(IntegrityError::ContentHashMismatch);
    }
    let parent_values = object
        .get("parentRevisionIds")
        .and_then(Value::as_array)
        .ok_or(IntegrityError::InvalidShape)?;
    if parent_values.len() > 32 {
        return Err(IntegrityError::InvalidShape);
    }
    let mut parents = Vec::with_capacity(parent_values.len());
    for parent in parent_values {
        let parent = parent.as_str().ok_or(IntegrityError::InvalidShape)?;
        if !valid_sha256_identifier(parent) {
            return Err(IntegrityError::InvalidShape);
        }
        parents.push(parent);
    }
    if !parents.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(IntegrityError::InvalidShape);
    }
    let revision_basis = serde_json::json!({
        "schema_version": 1,
        "node_id": node_id,
        "subject_type": subject_type,
        "subject_id": entity_id,
        "object_id": claimed_content,
        "parent_ids": parent_values,
        "created_at": timestamp,
    });
    let recomputed = sha256_identifier(&revision_basis)?;
    if recomputed != claimed_revision || recomputed != event_id {
        return Err(IntegrityError::RevisionHashMismatch);
    }
    Ok(())
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
pub(crate) fn verify_replication_envelope(value: &Value) -> Result<(), IntegrityError> {
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
    )?;
    if text(object, "kind")? != "replication-envelope"
        || object.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Err(IntegrityError::InvalidShape);
    }
    let payload = object.get("payload").ok_or(IntegrityError::InvalidShape)?;
    verify_memory_revision(payload)?;
    let payload_object = payload.as_object().ok_or(IntegrityError::InvalidShape)?;
    let hashes = exact_object(
        object.get("hashes").ok_or(IntegrityError::InvalidShape)?,
        &["payload", "envelope"],
    )?;
    let payload_hashes = payload_object
        .get("hashes")
        .and_then(Value::as_object)
        .ok_or(IntegrityError::InvalidShape)?;
    let payload_event_id = text(payload_object, "eventId")?;
    let payload_cursor = text(payload_object, "cursor")?;
    let expected_event_id = format!("replication:{payload_event_id}:{payload_cursor}");
    if text(object, "entityId")? != text(payload_object, "entityId")?
        || text(object, "nodeId")? != text(payload_object, "nodeId")?
        || text(object, "classification")? != text(payload_object, "classification")?
        || text(object, "timestamp")? != text(payload_object, "timestamp")?
        || text(object, "cursor")? != payload_cursor
        || text(object, "eventId")? != expected_event_id
        || object.get("tombstone") != payload_object.get("tombstone")
        || text(hashes, "payload")? != text(payload_hashes, "revision")?
        || object
            .get("parentRevisionIds")
            .and_then(Value::as_array)
            .is_none_or(|parents| {
                parents.len() != 1
                    || parents.first().and_then(Value::as_str) != Some(payload_event_id)
            })
    {
        return Err(IntegrityError::InvalidShape);
    }
    let mut basis = object.clone();
    let basis_hashes = basis
        .get_mut("hashes")
        .and_then(Value::as_object_mut)
        .ok_or(IntegrityError::InvalidShape)?;
    basis_hashes.remove("envelope");
    let recomputed = sha256_identifier(&Value::Object(basis))?;
    if recomputed != text(hashes, "envelope")? {
        return Err(IntegrityError::EnvelopeHashMismatch);
    }
    Ok(())
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 4 context ingestion calls this verifier")
)]
fn valid_sha256_identifier(value: &str) -> bool {
    value.len() == 71
        && value.strip_prefix("sha256:").is_some_and(|digest| {
            digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

#[derive(Clone, Debug)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
pub(crate) struct AdviserContextPolicy {
    pub(crate) active_snapshot_id: String,
    pub(crate) allowed_apple_ids: BTreeSet<String>,
    pub(crate) allowed_file_paths: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
pub(crate) enum ContextRejection {
    InvalidShape,
    PromptInjection,
    StaleSnapshot,
    MissingCitation,
    ConflictedMemory,
    OutsideAllowlist,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
fn contains_prompt_injection(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    [
        "ignore previous instruction",
        "ignore all previous",
        "system prompt",
        "developer message",
        "activate_snapshot",
        "resolve_conflict",
        "tool_call",
        "<|im_start|>",
        "[inst]",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
fn context_contains_prompt_injection(value: &Value) -> Result<bool, ContextRejection> {
    fn scan(value: &Value, depth: usize, nodes: &mut usize) -> Result<bool, ContextRejection> {
        if depth > MAXIMUM_JSON_DEPTH || *nodes >= MAXIMUM_JSON_NODES {
            return Err(ContextRejection::InvalidShape);
        }
        *nodes += 1;
        match value {
            Value::String(text) => Ok(contains_prompt_injection(text)),
            Value::Array(items) => {
                for item in items {
                    if scan(item, depth + 1, nodes)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::Object(object) => {
                for (key, child) in object {
                    if contains_prompt_injection(key) || scan(child, depth + 1, nodes)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(false),
        }
    }

    let mut nodes = 0;
    scan(value, 0, &mut nodes)
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
pub(crate) fn validate_rag_context(
    policy: &AdviserContextPolicy,
    value: &Value,
) -> Result<(), ContextRejection> {
    canonical_json_bytes(value).map_err(|_| ContextRejection::InvalidShape)?;
    let object = value.as_object().ok_or(ContextRejection::InvalidShape)?;
    if object.len() != 7
        || [
            "schema",
            "tool_policy",
            "query",
            "snapshot",
            "retrieved_at",
            "total",
            "results",
        ]
        .iter()
        .any(|key| !object.contains_key(*key))
        || object.get("schema").and_then(Value::as_str) != Some("rag-evidence-v1")
    {
        return Err(ContextRejection::InvalidShape);
    }
    let tool_policy = object
        .get("tool_policy")
        .and_then(Value::as_object)
        .ok_or(ContextRejection::InvalidShape)?;
    if tool_policy.len() != 3
        || tool_policy.get("mode").and_then(Value::as_str) != Some("read_only")
        || tool_policy.get("retrieved_content").and_then(Value::as_str)
            != Some("untrusted_evidence")
        || tool_policy
            .get("instruction_effect")
            .and_then(Value::as_str)
            != Some("none")
    {
        return Err(ContextRejection::InvalidShape);
    }
    let _query = object
        .get("query")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= 4096)
        .ok_or(ContextRejection::InvalidShape)?;
    let retrieved_at = object
        .get("retrieved_at")
        .and_then(Value::as_str)
        .filter(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).is_ok())
        .ok_or(ContextRejection::InvalidShape)?;
    let snapshot = object
        .get("snapshot")
        .and_then(Value::as_object)
        .ok_or(ContextRejection::StaleSnapshot)?;
    if snapshot.get("active_snapshot_id").and_then(Value::as_str)
        != Some(policy.active_snapshot_id.as_str())
    {
        return Err(ContextRejection::StaleSnapshot);
    }
    let results = object
        .get("results")
        .and_then(Value::as_array)
        .filter(|results| results.len() <= 200)
        .ok_or(ContextRejection::InvalidShape)?;
    if object.get("total").and_then(Value::as_u64) != Some(results.len() as u64) {
        return Err(ContextRejection::InvalidShape);
    }
    for result in results {
        let result = result.as_object().ok_or(ContextRejection::InvalidShape)?;
        if result.len() != 5
            || [
                "untrusted_evidence",
                "source",
                "scores",
                "quoted_text",
                "metadata",
            ]
            .iter()
            .any(|key| !result.contains_key(*key))
            || result.get("untrusted_evidence").and_then(Value::as_bool) != Some(true)
            || !result.get("scores").is_some_and(Value::is_object)
            || !result.get("metadata").is_some_and(Value::is_object)
        {
            return Err(ContextRejection::InvalidShape);
        }
        let quoted_text = result
            .get("quoted_text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty() && text.len() <= 1024 * 1024)
            .ok_or(ContextRejection::InvalidShape)?;
        let source = result
            .get("source")
            .and_then(Value::as_object)
            .ok_or(ContextRejection::MissingCitation)?;
        let required = [
            "source_id",
            "collection",
            "document_id",
            "chunk_id",
            "snapshot_id",
            "retrieved_at",
            "quoted_location",
        ];
        if source.len() != required.len()
            || required.iter().any(|key| {
                source.get(*key).is_none_or(|value| {
                    if *key == "quoted_location" {
                        !value.is_object()
                    } else {
                        value.as_str().is_none_or(str::is_empty)
                    }
                })
            })
        {
            return Err(ContextRejection::MissingCitation);
        }
        if source.get("snapshot_id").and_then(Value::as_str)
            != Some(policy.active_snapshot_id.as_str())
            || source.get("retrieved_at").and_then(Value::as_str) != Some(retrieved_at)
        {
            return Err(ContextRejection::StaleSnapshot);
        }
        if contains_prompt_injection(quoted_text)
            || context_contains_prompt_injection(&Value::Object(result.clone()))?
        {
            return Err(ContextRejection::PromptInjection);
        }
    }
    Ok(())
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
pub(crate) fn validate_memory_context(value: &Value) -> Result<(), ContextRejection> {
    canonical_json_bytes(value).map_err(|_| ContextRejection::InvalidShape)?;
    let object = value.as_object().ok_or(ContextRejection::InvalidShape)?;
    if object.len() > 4
        || object.keys().any(|key| {
            !["entity_id", "content", "conflicted_fields", "citation"].contains(&key.as_str())
        })
        || ["entity_id", "content", "conflicted_fields"]
            .iter()
            .any(|key| !object.contains_key(*key))
        || object
            .get("entity_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || !object.get("content").is_some_and(Value::is_object)
    {
        return Err(ContextRejection::InvalidShape);
    }
    let citation = object
        .get("citation")
        .and_then(Value::as_object)
        .ok_or(ContextRejection::MissingCitation)?;
    let citation_keys = ["event_id", "revision_hash", "node_id", "timestamp"];
    if citation.len() != citation_keys.len()
        || citation_keys.iter().any(|key| {
            citation
                .get(*key)
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        })
    {
        return Err(ContextRejection::MissingCitation);
    }
    let event_id = citation
        .get("event_id")
        .and_then(Value::as_str)
        .ok_or(ContextRejection::MissingCitation)?;
    let revision_hash = citation
        .get("revision_hash")
        .and_then(Value::as_str)
        .ok_or(ContextRejection::MissingCitation)?;
    let node_id = citation
        .get("node_id")
        .and_then(Value::as_str)
        .ok_or(ContextRejection::MissingCitation)?;
    let timestamp = citation
        .get("timestamp")
        .and_then(Value::as_str)
        .ok_or(ContextRejection::MissingCitation)?;
    if !valid_sha256_identifier(event_id)
        || revision_hash != event_id
        || !valid_node_id(node_id)
        || chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
    {
        return Err(ContextRejection::MissingCitation);
    }
    if object
        .get("conflicted_fields")
        .and_then(Value::as_array)
        .is_none_or(|fields| !fields.is_empty())
    {
        return Err(ContextRejection::ConflictedMemory);
    }
    if context_contains_prompt_injection(value)? {
        return Err(ContextRejection::PromptInjection);
    }
    Ok(())
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 4 adviser construction uses this sealed context policy"
    )
)]
pub(crate) fn validate_apple_context(
    policy: &AdviserContextPolicy,
    value: &Value,
) -> Result<(), ContextRejection> {
    canonical_json_bytes(value).map_err(|_| ContextRejection::InvalidShape)?;
    let object = value.as_object().ok_or(ContextRejection::InvalidShape)?;
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .ok_or(ContextRejection::InvalidShape)?;
    if !matches!(source, "calendar" | "reminders" | "notes" | "files") {
        return Err(ContextRejection::InvalidShape);
    }
    if source == "files" {
        if object.len() != 3
            || ["source", "path", "fields"]
                .iter()
                .any(|key| !object.contains_key(*key))
        {
            return Err(ContextRejection::InvalidShape);
        }
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or(ContextRejection::InvalidShape)?;
        if !policy.allowed_file_paths.contains(path) {
            return Err(ContextRejection::OutsideAllowlist);
        }
    } else {
        if object.len() != 3
            || ["source", "allowlist_id", "fields"]
                .iter()
                .any(|key| !object.contains_key(*key))
        {
            return Err(ContextRejection::InvalidShape);
        }
        let allowlist_id = object
            .get("allowlist_id")
            .and_then(Value::as_str)
            .ok_or(ContextRejection::InvalidShape)?;
        if !policy.allowed_apple_ids.contains(allowlist_id) {
            return Err(ContextRejection::OutsideAllowlist);
        }
    }
    if !object.get("fields").is_some_and(Value::is_object) {
        return Err(ContextRejection::InvalidShape);
    }
    if context_contains_prompt_injection(value)? {
        return Err(ContextRejection::PromptInjection);
    }
    Ok(())
}
