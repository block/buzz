use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{ExecutedToolCall, ExecutedToolProvider};

mod memory;
mod rag;

const MAX_POLICY_BYTES: usize = 128 * 1024;
const MAX_EVIDENCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 10_000;
const MAX_SERVICES: usize = 8;
const MAX_ALLOWLIST_ENTRIES: usize = 1_024;
const MAX_CLOCK_SKEW_SECONDS: i64 = 300;
const MAX_VALIDATED_RECORD_QUOTE_BYTES: usize = 16 * 1024;

/// A reason native evidence was rejected before it could become model input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceRejection {
    InvalidPolicy,
    InvalidShape,
    UntrustedService,
    IntegrityFailure,
    StaleEvidence,
    MissingCitation,
    ConflictedMemory,
    MixedSnapshot,
    OutsideAllowlist,
}

impl EvidenceRejection {
    /// Returns the stable diagnostic code suitable for redacted error reporting.
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "invalid_policy",
            Self::InvalidShape => "invalid_shape",
            Self::UntrustedService => "untrusted_service",
            Self::IntegrityFailure => "integrity_failure",
            Self::StaleEvidence => "stale_evidence",
            Self::MissingCitation => "missing_citation",
            Self::ConflictedMemory => "conflicted_memory",
            Self::MixedSnapshot => "mixed_snapshot",
            Self::OutsideAllowlist => "outside_allowlist",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum EvidenceKind {
    Memory,
    Rag,
    Apple,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServiceRule {
    server_label: String,
    kind: EvidenceKind,
    active_identity: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvidencePolicy {
    version: u32,
    maximum_evidence_age_seconds: u64,
    services: Vec<RawServiceRule>,
    allowed_apple_ids: Vec<String>,
    allowed_file_paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ServiceRule {
    kind: EvidenceKind,
    active_identity: String,
}

/// One source record extracted only after the catalog-owned evidence gate has
/// checked the native MCP result. Its quote remains untrusted data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidatedEvidenceRecord {
    /// The admitted MCP server that returned this record.
    pub server_label: String,
    /// The allowlisted tool name that returned this record.
    pub tool_name: String,
    /// The source family selected by the catalog policy.
    pub source_kind: String,
    /// A source identifier from the validated native result.
    pub source_id: String,
    /// The validated source collection or input family.
    pub collection: String,
    /// A document, entity, allowlist, or path identity.
    pub document_id: String,
    /// A chunk, revision, or stable sub-source identity.
    pub chunk_id: String,
    /// The frozen RAG snapshot when the source supplies one.
    pub snapshot_id: Option<String>,
    /// The result retrieval timestamp when the source supplies one.
    pub retrieved_at: Option<String>,
    /// The native observation time used to validate the result.
    pub observed_at: String,
    /// A bounded source location label.
    pub location: String,
    /// A bounded, inert quote retained for trusted orchestration only.
    pub quote: String,
    /// Always true: consumers must never treat this content as instructions.
    pub untrusted_evidence: bool,
}

/// The validated source records emitted by one explicitly allowlisted tool call.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidatedToolEvidence {
    /// The admitted MCP server that produced the result.
    pub server_label: String,
    /// The allowlisted tool that produced the result.
    pub tool_name: String,
    /// Parsed source records. Readiness-only calls may intentionally have none.
    pub records: Vec<ValidatedEvidenceRecord>,
}

/// Catalog-owned gate applied to every native MCP result before Buzz adopts
/// LM Studio's response ID or emits any portion of the response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEvidenceGate {
    services: BTreeMap<String, ServiceRule>,
    maximum_evidence_age_seconds: i64,
    allowed_apple_ids: BTreeSet<String>,
    allowed_file_paths: BTreeSet<String>,
}

impl CommandEvidenceGate {
    /// Parses the catalog-owned evidence policy for the exact configured MCP labels.
    pub fn parse(
        raw: Option<&str>,
        expected_server_labels: &BTreeSet<String>,
    ) -> Result<Self, EvidenceRejection> {
        if expected_server_labels.is_empty() && raw.is_none_or(str::is_empty) {
            return Ok(Self {
                services: BTreeMap::new(),
                maximum_evidence_age_seconds: 1,
                allowed_apple_ids: BTreeSet::new(),
                allowed_file_paths: BTreeSet::new(),
            });
        }
        let raw = raw
            .filter(|raw| !raw.is_empty() && raw.len() <= MAX_POLICY_BYTES)
            .ok_or(EvidenceRejection::InvalidPolicy)?;
        let policy: RawEvidencePolicy =
            serde_json::from_str(raw).map_err(|_| EvidenceRejection::InvalidPolicy)?;
        if policy.version != 1
            || !(1..=30 * 24 * 60 * 60).contains(&policy.maximum_evidence_age_seconds)
            || policy.services.is_empty()
            || policy.services.len() > MAX_SERVICES
            || policy.allowed_apple_ids.len() > MAX_ALLOWLIST_ENTRIES
            || policy.allowed_file_paths.len() > MAX_ALLOWLIST_ENTRIES
        {
            return Err(EvidenceRejection::InvalidPolicy);
        }
        let mut services = BTreeMap::new();
        for rule in policy.services {
            if !valid_identifier(&rule.server_label)
                || !valid_active_identity(rule.kind, &rule.active_identity)
                || services
                    .insert(
                        rule.server_label,
                        ServiceRule {
                            kind: rule.kind,
                            active_identity: rule.active_identity,
                        },
                    )
                    .is_some()
            {
                return Err(EvidenceRejection::InvalidPolicy);
            }
        }
        if services.keys().cloned().collect::<BTreeSet<_>>() != *expected_server_labels {
            return Err(EvidenceRejection::InvalidPolicy);
        }
        Ok(Self {
            services,
            maximum_evidence_age_seconds: i64::try_from(policy.maximum_evidence_age_seconds)
                .map_err(|_| EvidenceRejection::InvalidPolicy)?,
            allowed_apple_ids: exact_string_set(policy.allowed_apple_ids, false)?,
            allowed_file_paths: exact_string_set(policy.allowed_file_paths, true)?,
        })
    }

    /// Validates an executed MCP tool call while discarding parsed source records.
    pub fn validate_tool_call(&self, call: &ExecutedToolCall) -> Result<(), EvidenceRejection> {
        self.validate_tool_call_at(call, Utc::now())
    }

    /// Validates an executed MCP tool call at a supplied trusted clock value.
    pub fn validate_tool_call_at(
        &self,
        call: &ExecutedToolCall,
        now: DateTime<Utc>,
    ) -> Result<(), EvidenceRejection> {
        self.validated_tool_call_at(call, now).map(|_| ())
    }

    /// Validates an executed MCP tool call and returns bounded, explicitly
    /// untrusted source records for native orchestration. Retrieved prose is
    /// deliberately not inspected as instructions: policy is owned by the
    /// runtime catalog and model prompts separately delimit these records.
    pub fn validated_tool_call_at(
        &self,
        call: &ExecutedToolCall,
        now: DateTime<Utc>,
    ) -> Result<ValidatedToolEvidence, EvidenceRejection> {
        let label = match &call.provider {
            ExecutedToolProvider::EphemeralMcp { server_label } => server_label,
            ExecutedToolProvider::Plugin { .. } => return Err(EvidenceRejection::UntrustedService),
        };
        let rule = self
            .services
            .get(label)
            .ok_or(EvidenceRejection::UntrustedService)?;
        let value = decode_tool_output(&call.output)?;
        match rule.kind {
            EvidenceKind::Memory => memory::validate(
                &value,
                &rule.active_identity,
                now,
                self.maximum_evidence_age_seconds,
            ),
            EvidenceKind::Rag => rag::validate(
                &value,
                &rule.active_identity,
                now,
                self.maximum_evidence_age_seconds,
            ),
            EvidenceKind::Apple => {
                validate_apple_evidence(&value, &self.allowed_apple_ids, &self.allowed_file_paths)
            }
        }?;
        Ok(ValidatedToolEvidence {
            server_label: label.clone(),
            tool_name: call.name.clone(),
            records: extract_validated_records(rule.kind, &value, label, &call.name, now)?,
        })
    }
}

fn exact_string_set(
    values: Vec<String>,
    require_absolute: bool,
) -> Result<BTreeSet<String>, EvidenceRejection> {
    let mut result = BTreeSet::new();
    for value in values {
        if value.is_empty()
            || value.len() > 4_096
            || value.chars().any(char::is_control)
            || (require_absolute && !value.starts_with('/'))
            || !result.insert(value)
        {
            return Err(EvidenceRejection::InvalidPolicy);
        }
    }
    Ok(result)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_active_identity(kind: EvidenceKind, value: &str) -> bool {
    match kind {
        EvidenceKind::Memory => valid_node_id(value),
        EvidenceKind::Rag => valid_sha256_digest(value),
        EvidenceKind::Apple => value == "local",
    }
}

fn decode_tool_output(output: &str) -> Result<Value, EvidenceRejection> {
    if output.is_empty() || output.len() > MAX_EVIDENCE_BYTES {
        return Err(EvidenceRejection::InvalidShape);
    }
    let value: Value = serde_json::from_str(output).map_err(|_| EvidenceRejection::InvalidShape)?;
    bounded_json(&value)?;
    let Some(object) = value.as_object() else {
        return Ok(value);
    };
    if object.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(EvidenceRejection::InvalidShape);
    }
    if let Some(structured) = object.get("structuredContent") {
        return Ok(structured.clone());
    }
    Ok(value)
}

fn extract_validated_records(
    kind: EvidenceKind,
    value: &Value,
    server_label: &str,
    tool_name: &str,
    observed_at: DateTime<Utc>,
) -> Result<Vec<ValidatedEvidenceRecord>, EvidenceRejection> {
    match kind {
        EvidenceKind::Rag => extract_rag_records(value, server_label, tool_name, observed_at),
        EvidenceKind::Memory => extract_memory_records(value, server_label, tool_name, observed_at),
        EvidenceKind::Apple => extract_apple_record(value, server_label, tool_name, observed_at),
    }
}

fn extract_rag_records(
    value: &Value,
    server_label: &str,
    tool_name: &str,
    observed_at: DateTime<Utc>,
) -> Result<Vec<ValidatedEvidenceRecord>, EvidenceRejection> {
    let Some(object) = value.as_object() else {
        return Ok(Vec::new());
    };
    match object.get("schema").and_then(Value::as_str) {
        Some("rag-evidence-v1") => object
            .get("results")
            .and_then(Value::as_array)
            .ok_or(EvidenceRejection::InvalidShape)?
            .iter()
            .map(|result| {
                let source = result
                    .get("source")
                    .and_then(Value::as_object)
                    .ok_or(EvidenceRejection::MissingCitation)?;
                let location = source
                    .get("quoted_location")
                    .map(json_text)
                    .unwrap_or_default();
                Ok(validated_record(
                    server_label,
                    tool_name,
                    "rag",
                    source_text(source, "source_id")?,
                    source_text(source, "collection")?,
                    source_text(source, "document_id")?,
                    source_text(source, "chunk_id")?,
                    Some(source_text(source, "snapshot_id")?),
                    Some(source_text(source, "retrieved_at")?),
                    observed_at,
                    location,
                    result
                        .get("quoted_text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ))
            })
            .collect(),
        Some("rag-source-v1") if object.get("found").and_then(Value::as_bool) == Some(true) => {
            let source = object
                .get("source")
                .and_then(Value::as_object)
                .ok_or(EvidenceRejection::MissingCitation)?;
            Ok(vec![validated_record(
                server_label,
                tool_name,
                "rag",
                source_text(source, "source_id")?,
                source_text(source, "collection")?,
                source_text(source, "document_id")?,
                source
                    .get("chunk_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| source_text(source, "source_id").unwrap_or_default()),
                Some(source_text(source, "snapshot_id")?),
                Some(source_text(source, "retrieved_at")?),
                observed_at,
                source
                    .get("quoted_location")
                    .map(json_text)
                    .unwrap_or_default(),
                String::new(),
            )])
        }
        _ => Ok(Vec::new()),
    }
}

fn extract_memory_records(
    value: &Value,
    server_label: &str,
    tool_name: &str,
    observed_at: DateTime<Utc>,
) -> Result<Vec<ValidatedEvidenceRecord>, EvidenceRejection> {
    if let Some(results) = value.get("results").and_then(Value::as_array) {
        return results
            .iter()
            .map(|result| {
                let revision = result
                    .get("revision")
                    .and_then(Value::as_object)
                    .ok_or(EvidenceRejection::IntegrityFailure)?;
                let citation = result
                    .get("citation")
                    .and_then(Value::as_object)
                    .ok_or(EvidenceRejection::MissingCitation)?;
                Ok(validated_record(
                    server_label,
                    tool_name,
                    "memory",
                    source_text(citation, "event_id")?,
                    "memory".to_string(),
                    source_text(revision, "entityId")?,
                    source_text(citation, "revision_hash")?,
                    None,
                    Some(source_text(citation, "timestamp")?),
                    observed_at,
                    source_text(citation, "node_id")?,
                    result
                        .get("quoted_text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ))
            })
            .collect();
    }
    let object = value.as_object().ok_or(EvidenceRejection::InvalidShape)?;
    let citation = object
        .get("citation")
        .and_then(Value::as_object)
        .ok_or(EvidenceRejection::MissingCitation)?;
    Ok(vec![validated_record(
        server_label,
        tool_name,
        "memory",
        source_text(citation, "event_id")?,
        "memory".to_string(),
        object
            .get("entity_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        source_text(citation, "revision_hash")?,
        None,
        Some(source_text(citation, "timestamp")?),
        observed_at,
        source_text(citation, "node_id")?,
        object.get("content").map(json_text).unwrap_or_default(),
    )])
}

fn extract_apple_record(
    value: &Value,
    server_label: &str,
    tool_name: &str,
    observed_at: DateTime<Utc>,
) -> Result<Vec<ValidatedEvidenceRecord>, EvidenceRejection> {
    let object = value.as_object().ok_or(EvidenceRejection::InvalidShape)?;
    let source = text(object, "source")?.to_string();
    let document_id = if source == "files" {
        text(object, "path")?.to_string()
    } else {
        text(object, "allowlist_id")?.to_string()
    };
    Ok(vec![validated_record(
        server_label,
        tool_name,
        "apple",
        document_id.clone(),
        source,
        document_id.clone(),
        document_id,
        None,
        None,
        observed_at,
        "admitted local input".to_string(),
        object.get("fields").map(json_text).unwrap_or_default(),
    )])
}

#[allow(clippy::too_many_arguments)]
fn validated_record(
    server_label: &str,
    tool_name: &str,
    source_kind: &str,
    source_id: String,
    collection: String,
    document_id: String,
    chunk_id: String,
    snapshot_id: Option<String>,
    retrieved_at: Option<String>,
    observed_at: DateTime<Utc>,
    location: String,
    quote: String,
) -> ValidatedEvidenceRecord {
    ValidatedEvidenceRecord {
        server_label: server_label.to_string(),
        tool_name: tool_name.to_string(),
        source_kind: source_kind.to_string(),
        source_id,
        collection,
        document_id,
        chunk_id,
        snapshot_id,
        retrieved_at,
        observed_at: observed_at.to_rfc3339(),
        location: bounded_text(location, 4_096),
        quote: bounded_text(quote, MAX_VALIDATED_RECORD_QUOTE_BYTES),
        untrusted_evidence: true,
    }
}

fn source_text(object: &Map<String, Value>, key: &str) -> Result<String, EvidenceRejection> {
    text(object, key).map(str::to_string)
}

fn json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn bounded_text(value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn bounded_json(value: &Value) -> Result<(), EvidenceRejection> {
    fn walk(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), EvidenceRejection> {
        if depth > MAX_JSON_DEPTH || *nodes >= MAX_JSON_NODES {
            return Err(EvidenceRejection::InvalidShape);
        }
        *nodes += 1;
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(item, depth + 1, nodes)?;
                }
            }
            Value::Object(object) => {
                for child in object.values() {
                    walk(child, depth + 1, nodes)?;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
        Ok(())
    }
    let mut nodes = 0;
    walk(value, 0, &mut nodes)
}

fn evidence_is_fresh(
    timestamp: &str,
    now: DateTime<Utc>,
    maximum_age_seconds: i64,
) -> Result<(), EvidenceRejection> {
    let timestamp = DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| EvidenceRejection::StaleEvidence)?
        .with_timezone(&Utc);
    let age = now.signed_duration_since(timestamp).num_seconds();
    if age < -MAX_CLOCK_SKEW_SECONDS || age > maximum_age_seconds {
        Err(EvidenceRejection::StaleEvidence)
    } else {
        Ok(())
    }
}

fn validate_apple_evidence(
    value: &Value,
    allowed_apple_ids: &BTreeSet<String>,
    allowed_file_paths: &BTreeSet<String>,
) -> Result<(), EvidenceRejection> {
    let object = value.as_object().ok_or(EvidenceRejection::InvalidShape)?;
    let source = text(object, "source")?;
    if source == "files" {
        let object = exact_object(value, &["source", "path", "fields"])?;
        if !allowed_file_paths.contains(text(object, "path")?) {
            return Err(EvidenceRejection::OutsideAllowlist);
        }
        if !object.get("fields").is_some_and(Value::is_object) {
            return Err(EvidenceRejection::InvalidShape);
        }
        return Ok(());
    }
    if !matches!(source, "calendar" | "reminders" | "notes") {
        return Err(EvidenceRejection::InvalidShape);
    }
    let object = exact_object(value, &["source", "allowlist_id", "fields"])?;
    if !allowed_apple_ids.contains(text(object, "allowlist_id")?) {
        return Err(EvidenceRejection::OutsideAllowlist);
    }
    if !object.get("fields").is_some_and(Value::is_object) {
        return Err(EvidenceRejection::InvalidShape);
    }
    Ok(())
}

fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, EvidenceRejection> {
    let object = value.as_object().ok_or(EvidenceRejection::InvalidShape)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(EvidenceRejection::InvalidShape);
    }
    Ok(object)
}

fn text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, EvidenceRejection> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4_096)
        .ok_or(EvidenceRejection::InvalidShape)
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

fn valid_sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
