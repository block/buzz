use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Deserialize;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceRejection {
    InvalidPolicy,
    InvalidShape,
    UntrustedService,
    IntegrityFailure,
    PromptInjection,
    StaleEvidence,
    MissingCitation,
    ConflictedMemory,
    MixedSnapshot,
    OutsideAllowlist,
}

impl EvidenceRejection {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "invalid_policy",
            Self::InvalidShape => "invalid_shape",
            Self::UntrustedService => "untrusted_service",
            Self::IntegrityFailure => "integrity_failure",
            Self::PromptInjection => "prompt_injection",
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

/// Catalog-owned gate applied to every native MCP result before Buzz adopts
/// LM Studio's response ID or emits any portion of the response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandEvidenceGate {
    services: BTreeMap<String, ServiceRule>,
    maximum_evidence_age_seconds: i64,
    allowed_apple_ids: BTreeSet<String>,
    allowed_file_paths: BTreeSet<String>,
}

impl CommandEvidenceGate {
    pub(crate) fn parse(
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

    pub(crate) fn validate_tool_call(
        &self,
        call: &ExecutedToolCall,
    ) -> Result<(), EvidenceRejection> {
        self.validate_tool_call_at(call, Utc::now())
    }

    pub(crate) fn validate_tool_call_at(
        &self,
        call: &ExecutedToolCall,
        now: DateTime<Utc>,
    ) -> Result<(), EvidenceRejection> {
        let label = match &call.provider {
            ExecutedToolProvider::EphemeralMcp { server_label } => server_label,
            ExecutedToolProvider::Plugin { .. } => return Err(EvidenceRejection::UntrustedService),
        };
        let rule = self
            .services
            .get(label)
            .ok_or(EvidenceRejection::UntrustedService)?;
        let value = decode_tool_output(&call.output)?;
        if contains_prompt_injection(&value)? {
            return Err(EvidenceRejection::PromptInjection);
        }
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
        }
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

fn contains_prompt_injection(value: &Value) -> Result<bool, EvidenceRejection> {
    fn suspicious(text: &str) -> bool {
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
    fn scan(value: &Value, depth: usize, nodes: &mut usize) -> Result<bool, EvidenceRejection> {
        if depth > MAX_JSON_DEPTH || *nodes >= MAX_JSON_NODES {
            return Err(EvidenceRejection::InvalidShape);
        }
        *nodes += 1;
        match value {
            Value::String(text) => Ok(suspicious(text)),
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
                    if suspicious(key) || scan(child, depth + 1, nodes)? {
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
