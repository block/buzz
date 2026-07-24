use super::{MemoryError, MAXIMUM_OBJECTS_PER_PAGE, PAGE_SIZE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAXIMUM_CANONICAL_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_JSON_DEPTH: usize = 64;
const MAXIMUM_JSON_NODES: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub(super) schema_version: u32,
    pub(super) source_node_id: String,
    pub(super) from_cursor: u64,
    pub(super) to_cursor: u64,
    pub(super) has_more: bool,
    pub(super) revisions: Vec<InternalRevision>,
    pub(super) objects: BTreeMap<String, ImmutableObject>,
    pub(super) contracts: Vec<BuzzReplicationEnvelope>,
    pub(super) envelope_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImmutableObject {
    object_id: String,
    kind: String,
    payload: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InternalRevision {
    revision_id: String,
    node_id: String,
    subject_type: String,
    subject_id: String,
    object_id: String,
    parent_ids: Vec<String>,
    created_at: String,
    sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BuzzRevisionHashes {
    content: String,
    revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BuzzMemoryRevision {
    kind: String,
    version: u32,
    classification: String,
    #[serde(rename = "entityId")]
    entity_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
    #[serde(rename = "parentRevisionIds")]
    parent_revision_ids: Vec<String>,
    #[serde(rename = "nodeId")]
    node_id: String,
    timestamp: String,
    hashes: BuzzRevisionHashes,
    tombstone: bool,
    cursor: String,
    content: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BuzzEnvelopeHashes {
    payload: String,
    envelope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BuzzReplicationEnvelope {
    kind: String,
    version: u32,
    classification: String,
    #[serde(rename = "entityId")]
    entity_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
    #[serde(rename = "parentRevisionIds")]
    parent_revision_ids: Vec<String>,
    #[serde(rename = "nodeId")]
    node_id: String,
    timestamp: String,
    hashes: BuzzEnvelopeHashes,
    tombstone: bool,
    cursor: String,
    payload: BuzzMemoryRevision,
}

pub(super) fn object_is_tombstone(value: &ImmutableObject) -> bool {
    value.kind == "tombstone"
}

fn write_python_canonical(value: &Value, output: &mut Vec<u8>) -> Result<(), MemoryError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => {
            let mut encoded = number.to_string();
            if let Some(index) = encoded.find(['e', 'E']) {
                let mantissa = encoded[..index].to_string();
                let exponent = &encoded[index + 1..];
                let (sign, digits) = exponent
                    .strip_prefix('-')
                    .map_or(("+", exponent), |digits| ("-", digits));
                let digits = digits.strip_prefix('+').unwrap_or(digits);
                encoded = format!("{mantissa}e{sign}{digits:0>2}");
            }
            output.extend_from_slice(encoded.as_bytes());
        }
        Value::String(text) => output.extend_from_slice(
            serde_json::to_string(text)
                .map_err(|_| MemoryError::InvalidResponse)?
                .as_bytes(),
        ),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_python_canonical(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            output.push(b'{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|_| MemoryError::InvalidResponse)?
                        .as_bytes(),
                );
                output.push(b':');
                write_python_canonical(value, output)?;
            }
            output.push(b'}');
        }
    }
    if output.len() > MAXIMUM_CANONICAL_BYTES {
        return Err(MemoryError::ResponseTooLarge);
    }
    Ok(())
}

pub(super) fn python_canonical_json_bytes(value: &Value) -> Result<Vec<u8>, MemoryError> {
    let mut output = Vec::new();
    write_python_canonical(value, &mut output)?;
    Ok(output)
}

fn sha256_value(value: &Value) -> Result<String, MemoryError> {
    Ok(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(python_canonical_json_bytes(value)?))
    ))
}

pub(super) fn valid_envelope_id(value: &Value, supplied: &str) -> bool {
    let Some(mut unsigned) = value.as_object().cloned() else {
        return false;
    };
    unsigned.remove("envelope_id");
    let Ok(actual) = sha256_value(&Value::Object(unsigned)) else {
        return false;
    };
    supplied == actual
}

fn valid_sha256_id(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
        && value[7..].bytes().all(|byte| !byte.is_ascii_uppercase())
}

fn valid_node_id(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("node:") else {
        return false;
    };
    !suffix.is_empty()
        && suffix.len() <= 127
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_rfc3339(value: &str) -> bool {
    value.len() <= 64 && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

fn bounded_json(value: &Value) -> bool {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> bool {
        *nodes += 1;
        if depth > MAXIMUM_JSON_DEPTH || *nodes > MAXIMUM_JSON_NODES {
            return false;
        }
        match value {
            Value::Array(values) => values.iter().all(|value| visit(value, depth + 1, nodes)),
            Value::Object(values) => values
                .iter()
                .all(|(key, value)| key.len() <= 4096 && visit(value, depth + 1, nodes)),
            Value::String(value) => value.len() <= MAXIMUM_CANONICAL_BYTES,
            _ => true,
        }
    }
    visit(value, 0, &mut 0)
}

fn sorted_unique_hashes(values: &[String]) -> bool {
    values.len() <= 32
        && values.iter().all(|value| valid_sha256_id(value))
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_object(key: &str, value: &ImmutableObject) -> Result<(), MemoryError> {
    if key != value.object_id
        || !valid_sha256_id(key)
        || value.kind.is_empty()
        || value.kind.len() > 64
        || !value
            .kind
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
        || !value.payload.is_object()
        || !bounded_json(&value.payload)
    {
        return Err(MemoryError::InvalidResponse);
    }
    let basis = serde_json::json!({
        "schema_version": 1,
        "kind": value.kind,
        "payload": value.payload
    });
    if sha256_value(&basis)? != value.object_id {
        return Err(MemoryError::InvalidResponse);
    }
    Ok(())
}

fn validate_revision(
    revision: &InternalRevision,
    source_node_id: &str,
    objects: &BTreeMap<String, ImmutableObject>,
) -> Result<(), MemoryError> {
    if !valid_sha256_id(&revision.revision_id)
        || revision.node_id != source_node_id
        || !valid_node_id(&revision.node_id)
        || !matches!(
            revision.subject_type.as_str(),
            "event" | "entity" | "tombstone"
        )
        || revision.subject_id.is_empty()
        || revision.subject_id.len() > 256
        || !valid_sha256_id(&revision.object_id)
        || !sorted_unique_hashes(&revision.parent_ids)
        || !valid_rfc3339(&revision.created_at)
        || !objects.contains_key(&revision.object_id)
    {
        return Err(MemoryError::InvalidResponse);
    }
    let basis = serde_json::json!({
        "schema_version": 1,
        "node_id": revision.node_id,
        "subject_type": revision.subject_type,
        "subject_id": revision.subject_id,
        "object_id": revision.object_id,
        "parent_ids": revision.parent_ids,
        "created_at": revision.created_at
    });
    if sha256_value(&basis)? != revision.revision_id {
        return Err(MemoryError::InvalidResponse);
    }
    let object = objects
        .get(&revision.object_id)
        .ok_or(MemoryError::InvalidResponse)?;
    if object.kind == "tombstone" {
        let payload = object
            .payload
            .as_object()
            .ok_or(MemoryError::InvalidResponse)?;
        if payload.get("target_type").and_then(Value::as_str)
            != Some(revision.subject_type.as_str())
            || payload.get("target_id").and_then(Value::as_str)
                != Some(revision.subject_id.as_str())
            || !payload
                .get("deleted_at")
                .and_then(Value::as_str)
                .is_some_and(valid_rfc3339)
            || !payload
                .get("retain_until")
                .and_then(Value::as_str)
                .is_some_and(valid_rfc3339)
            || !payload
                .get("prior_object_id")
                .and_then(Value::as_str)
                .is_some_and(valid_sha256_id)
        {
            return Err(MemoryError::InvalidResponse);
        }
    } else if object.kind != revision.subject_type {
        return Err(MemoryError::InvalidResponse);
    }
    Ok(())
}

fn validate_contract(
    contract: &BuzzReplicationEnvelope,
    revision: &InternalRevision,
    object: &ImmutableObject,
) -> Result<(), MemoryError> {
    let payload = &contract.payload;
    let cursor = payload
        .cursor
        .parse::<u64>()
        .map_err(|_| MemoryError::InvalidResponse)?;
    let contract_cursor = contract
        .cursor
        .parse::<u64>()
        .map_err(|_| MemoryError::InvalidResponse)?;
    let tombstone = object.kind == "tombstone";
    let expected_content = if tombstone {
        Value::Null
    } else {
        object.payload.clone()
    };
    if contract.kind != "replication-envelope"
        || contract.version != 1
        || !matches!(contract.classification.as_str(), "PUBLIC" | "OFFICIAL")
        || contract.entity_id != revision.subject_id
        || contract.event_id
            != format!("replication:{}:{}", revision.revision_id, revision.sequence)
        || contract.parent_revision_ids != [revision.revision_id.clone()]
        || contract.node_id != revision.node_id
        || contract.timestamp != revision.created_at
        || !valid_rfc3339(&contract.timestamp)
        || contract.hashes.payload != revision.revision_id
        || !valid_sha256_id(&contract.hashes.envelope)
        || contract.tombstone != tombstone
        || contract_cursor != revision.sequence
        || payload.kind != "memory-revision"
        || payload.version != 1
        || !matches!(payload.classification.as_str(), "PUBLIC" | "OFFICIAL")
        || payload.entity_id != revision.subject_id
        || payload.event_id != revision.revision_id
        || payload.parent_revision_ids != revision.parent_ids
        || payload.node_id != revision.node_id
        || payload.timestamp != revision.created_at
        || payload.hashes.content != object.object_id
        || payload.hashes.revision != revision.revision_id
        || payload.tombstone != tombstone
        || cursor != revision.sequence
        || payload.content != expected_content
        || contract.classification == "PUBLIC" && payload.classification != "PUBLIC"
        || !bounded_json(&payload.content)
    {
        return Err(MemoryError::InvalidResponse);
    }
    let mut basis = serde_json::to_value(contract).map_err(|_| MemoryError::InvalidResponse)?;
    basis["hashes"] = serde_json::json!({"payload": contract.hashes.payload});
    if sha256_value(&basis)? != contract.hashes.envelope {
        return Err(MemoryError::InvalidResponse);
    }
    Ok(())
}

pub(super) fn validate_envelope(
    envelope: &Envelope,
    raw: &Value,
    expected_source: &str,
    expected_cursor: u64,
    source_max_items: u64,
) -> Result<(), MemoryError> {
    if envelope.schema_version != 1
        || envelope.source_node_id != expected_source
        || !valid_node_id(&envelope.source_node_id)
        || envelope.from_cursor != expected_cursor
        || envelope.to_cursor < expected_cursor
        || envelope.revisions.len() > PAGE_SIZE as usize
        || envelope.revisions.len() as u64 > source_max_items
        || envelope.objects.len() > MAXIMUM_OBJECTS_PER_PAGE
        || envelope.contracts.len() != envelope.revisions.len()
        || !valid_envelope_id(raw, &envelope.envelope_id)
    {
        return Err(MemoryError::InvalidResponse);
    }
    let sequences = envelope
        .revisions
        .iter()
        .map(|revision| revision.sequence)
        .collect::<Vec<_>>();
    if sequences.is_empty() {
        if envelope.to_cursor != expected_cursor
            || envelope.has_more
            || !envelope.objects.is_empty()
        {
            return Err(MemoryError::InvalidResponse);
        }
        return Ok(());
    }
    if sequences[0] <= envelope.from_cursor
        || sequences.last() != Some(&envelope.to_cursor)
        || !sequences.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(MemoryError::InvalidResponse);
    }
    for (key, object) in &envelope.objects {
        validate_object(key, object)?;
    }
    let referenced_objects = envelope
        .revisions
        .iter()
        .map(|revision| revision.object_id.as_str())
        .collect::<BTreeSet<_>>();
    if referenced_objects
        != envelope
            .objects
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
    {
        return Err(MemoryError::InvalidResponse);
    }
    let revision_positions = envelope
        .revisions
        .iter()
        .enumerate()
        .map(|(index, revision)| (revision.revision_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    for (index, (revision, contract)) in envelope
        .revisions
        .iter()
        .zip(&envelope.contracts)
        .enumerate()
    {
        validate_revision(revision, &envelope.source_node_id, &envelope.objects)?;
        for parent_id in &revision.parent_ids {
            if let Some(parent_index) = revision_positions.get(parent_id.as_str()) {
                let parent = &envelope.revisions[*parent_index];
                if *parent_index >= index
                    || parent.subject_type != revision.subject_type
                    || parent.subject_id != revision.subject_id
                {
                    return Err(MemoryError::InvalidResponse);
                }
            }
        }
        let object = envelope
            .objects
            .get(&revision.object_id)
            .ok_or(MemoryError::InvalidResponse)?;
        validate_contract(contract, revision, object)?;
    }
    Ok(())
}
