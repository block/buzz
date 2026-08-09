use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{evidence_is_fresh, exact_object, text, EvidenceRejection};

pub(super) fn validate(
    value: &Value,
    active_snapshot: &str,
    now: DateTime<Utc>,
    maximum_age_seconds: i64,
) -> Result<(), EvidenceRejection> {
    if value.get("format").is_some() {
        return validate_readiness(value, active_snapshot);
    }
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or(EvidenceRejection::InvalidShape)?;
    match schema {
        "rag-evidence-v1" => {
            let object = exact_object(
                value,
                &[
                    "schema",
                    "tool_policy",
                    "query",
                    "snapshot",
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
            if text(policy, "mode")? != "read_only"
                || text(policy, "retrieved_content")? != "untrusted_evidence"
                || text(policy, "instruction_effect")? != "none"
                || text(object, "query")?.len() > 4_096
            {
                return Err(EvidenceRejection::InvalidShape);
            }
            require_snapshot_bindings(value, active_snapshot)?;
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
                validate_result(result, active_snapshot, retrieved_at)?;
            }
            Ok(())
        }
        "rag-catalogue-v1" => {
            let object = exact_object(
                value,
                &[
                    "schema",
                    "read_only",
                    "snapshot_id",
                    "retrieved_at",
                    "collections",
                    "total_chunks",
                ],
            )?;
            if object.get("read_only").and_then(Value::as_bool) != Some(true)
                || text(object, "snapshot_id")? != active_snapshot
                || !object.get("collections").is_some_and(Value::is_array)
                || object.get("total_chunks").and_then(Value::as_u64).is_none()
            {
                return Err(EvidenceRejection::MixedSnapshot);
            }
            evidence_is_fresh(text(object, "retrieved_at")?, now, maximum_age_seconds)
        }
        "rag-source-v1" => {
            let object = exact_object(
                value,
                &[
                    "schema",
                    "untrusted_evidence",
                    "found",
                    "snapshot_id",
                    "retrieved_at",
                    "source",
                ],
            )?;
            if object.get("untrusted_evidence").and_then(Value::as_bool) != Some(true)
                || text(object, "snapshot_id")? != active_snapshot
            {
                return Err(EvidenceRejection::MixedSnapshot);
            }
            let retrieved_at = text(object, "retrieved_at")?;
            evidence_is_fresh(retrieved_at, now, maximum_age_seconds)?;
            if object.get("found").and_then(Value::as_bool) == Some(false)
                && object.get("source") == Some(&Value::Null)
            {
                return Ok(());
            }
            let source = object
                .get("source")
                .and_then(Value::as_object)
                .ok_or(EvidenceRejection::MissingCitation)?;
            for key in [
                "source_id",
                "document_id",
                "collection",
                "snapshot_id",
                "retrieved_at",
            ] {
                if source
                    .get(key)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(EvidenceRejection::MissingCitation);
                }
            }
            if source.get("snapshot_id").and_then(Value::as_str) != Some(active_snapshot)
                || source.get("retrieved_at").and_then(Value::as_str) != Some(retrieved_at)
            {
                return Err(EvidenceRejection::MixedSnapshot);
            }
            Ok(())
        }
        _ => Err(EvidenceRejection::InvalidShape),
    }
}

fn validate_readiness(value: &Value, active_snapshot: &str) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &[
            "format",
            "active_activation_id",
            "active_snapshot_id",
            "signature_fingerprint",
            "snapshot_time",
            "service",
            "retrieval_models",
            "collections",
            "golden_queries",
            "last_successful_activation",
        ],
    )?;
    if text(object, "format")? != "rag-readiness-v2"
        || !valid_sha256(text(object, "active_activation_id")?)
        || text(object, "active_snapshot_id")? != active_snapshot
        || !valid_sha256(text(object, "signature_fingerprint")?)
        || !object.get("service").is_some_and(Value::is_object)
        || !object.get("retrieval_models").is_some_and(Value::is_object)
        || !object.get("collections").is_some_and(Value::is_array)
        || object
            .get("golden_queries")
            .and_then(Value::as_object)
            .and_then(|golden| golden.get("passed"))
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(EvidenceRejection::InvalidShape);
    }
    DateTime::parse_from_rfc3339(text(object, "snapshot_time")?)
        .map_err(|_| EvidenceRejection::InvalidShape)?;
    require_snapshot_bindings(value, active_snapshot)?;
    DateTime::parse_from_rfc3339(text(object, "last_successful_activation")?)
        .map(|_| ())
        .map_err(|_| EvidenceRejection::InvalidShape)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_result(
    value: &Value,
    active_snapshot: &str,
    retrieved_at: &str,
) -> Result<(), EvidenceRejection> {
    let object = exact_object(
        value,
        &[
            "untrusted_evidence",
            "source",
            "scores",
            "quoted_text",
            "metadata",
        ],
    )?;
    if object.get("untrusted_evidence").and_then(Value::as_bool) != Some(true)
        || text(object, "quoted_text")?.len() > 1024 * 1024
        || !object.get("scores").is_some_and(Value::is_object)
        || !object.get("metadata").is_some_and(Value::is_object)
    {
        return Err(EvidenceRejection::InvalidShape);
    }
    let source = object
        .get("source")
        .and_then(Value::as_object)
        .ok_or(EvidenceRejection::MissingCitation)?;
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
        return Err(EvidenceRejection::MissingCitation);
    }
    if source.get("snapshot_id").and_then(Value::as_str) != Some(active_snapshot)
        || source.get("retrieved_at").and_then(Value::as_str) != Some(retrieved_at)
    {
        return Err(EvidenceRejection::MixedSnapshot);
    }
    Ok(())
}

fn require_snapshot_bindings(
    value: &Value,
    active_snapshot: &str,
) -> Result<(), EvidenceRejection> {
    fn walk(value: &Value, active: &str) -> Result<(), EvidenceRejection> {
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(item, active)?;
                }
            }
            Value::Object(object) => {
                for (key, child) in object {
                    if matches!(key.as_str(), "snapshot_id" | "active_snapshot_id")
                        && child.as_str() != Some(active)
                    {
                        return Err(EvidenceRejection::MixedSnapshot);
                    }
                    walk(child, active)?;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
        Ok(())
    }
    walk(value, active_snapshot)
}
