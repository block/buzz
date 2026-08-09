use chrono::DateTime;
use serde_json::Value;

use super::{CandidateSource, SourceCollectionError};
use crate::command_brief::types::SourceKind;
use crate::command_services::rag::{RagSnapshotError, VerifiedRagSnapshot};
use crate::command_services::trusted_lan::{catalogue_fingerprint, TrustedLanError};

pub(crate) fn trusted_lan_snapshot_from_catalogue(
    catalogue: Result<Value, TrustedLanError>,
    rag_endpoint: &str,
    observed_at: &str,
) -> Result<(VerifiedRagSnapshot, bool), SourceCollectionError> {
    let (catalogue, collections, available) = match catalogue {
        Ok(catalogue) => {
            let collections = observed_collection_names(&catalogue)?;
            (catalogue, collections, true)
        }
        Err(_) => (
            serde_json::json!({
                "endpoint": rag_endpoint,
                "status": "unavailable",
            }),
            vec!["rag-unavailable".to_string()],
            false,
        ),
    };
    let fingerprint =
        catalogue_fingerprint(&catalogue).map_err(|_| SourceCollectionError::RagInvalid)?;
    let snapshot =
        VerifiedRagSnapshot::from_trusted_lan_observation(&fingerprint, observed_at, collections)
            .map_err(|_| SourceCollectionError::RagInvalid)?;
    Ok((snapshot, available))
}

pub(super) fn observed_collection_names(
    value: &Value,
) -> Result<Vec<String>, SourceCollectionError> {
    let collections = value
        .get("collections")
        .and_then(Value::as_array)
        .filter(|collections| !collections.is_empty() && collections.len() <= 256)
        .ok_or(SourceCollectionError::RagInvalid)?;
    let mut names = collections
        .iter()
        .filter_map(|collection| {
            collection
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| valid_observed_field(name, 256))
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(SourceCollectionError::RagInvalid);
    }
    names.sort();
    names.dedup();
    Ok(names)
}

pub(super) fn extract_trusted_lan_rag_evidence(
    value: &Value,
    expected_query: &str,
    observed_at: &str,
    allowed_collections: &[String],
) -> Result<Vec<CandidateSource>, RagSnapshotError> {
    if value.get("query").and_then(Value::as_str) != Some(expected_query)
        || DateTime::parse_from_rfc3339(observed_at).is_err()
    {
        return Err(RagSnapshotError::Invalid);
    }
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .filter(|results| results.len() <= 50)
        .ok_or(RagSnapshotError::Invalid)?;
    if value.get("total").and_then(Value::as_u64) != Some(results.len() as u64) {
        return Err(RagSnapshotError::Invalid);
    }
    results
        .iter()
        .map(|result| {
            let object = result.as_object().ok_or(RagSnapshotError::Invalid)?;
            let text = |key: &str, maximum: usize| {
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| valid_observed_field(value, maximum))
                    .ok_or(RagSnapshotError::Invalid)
            };
            let source_id = text("point_id", 256)?;
            let document_id = text("doc_id", 512)?;
            let document_name = text("doc_name", 1024)?;
            let collection = text("collection", 256)?;
            let quote = object
                .get("text")
                .and_then(Value::as_str)
                .filter(|value| valid_observed_content(value, 1024 * 1024))
                .ok_or(RagSnapshotError::Invalid)?;
            if !allowed_collections
                .iter()
                .any(|allowed| allowed == collection)
            {
                return Err(RagSnapshotError::Invalid);
            }
            let page = object
                .get("page_no")
                .and_then(Value::as_u64)
                .map(|page| format!("; page {page}"))
                .unwrap_or_default();
            let section = object
                .get("section_path")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|part| valid_observed_field(part, 1024))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .filter(|section| !section.is_empty())
                .map(|section| format!("; section {section}"))
                .unwrap_or_default();
            Ok(CandidateSource {
                source_id: source_id.to_string(),
                source_kind: SourceKind::Rag,
                collection: collection.to_string(),
                document_id: document_id.to_string(),
                chunk_id: source_id.to_string(),
                timestamp: observed_at.to_string(),
                location: format!("trusted-lan-observed; document {document_name}{page}{section}"),
                retrieved_at: observed_at.to_string(),
                observed_at: observed_at.to_string(),
                quote: quote.to_string(),
            })
        })
        .collect()
}

pub(super) fn extract_trusted_lan_memory_evidence(
    value: &Value,
    observed_at: &str,
) -> Result<Vec<CandidateSource>, RagSnapshotError> {
    if DateTime::parse_from_rfc3339(observed_at).is_err() {
        return Err(RagSnapshotError::Invalid);
    }
    value
        .as_array()
        .filter(|records| records.len() <= 50)
        .ok_or(RagSnapshotError::Invalid)?
        .iter()
        .map(|record| {
            let object = record.as_object().ok_or(RagSnapshotError::Invalid)?;
            let field = |key: &str, maximum: usize| {
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| valid_observed_field(value, maximum))
                    .ok_or(RagSnapshotError::Invalid)
            };
            let event_id = field("id", 256)?;
            let quote = object
                .get("content")
                .and_then(Value::as_str)
                .filter(|value| valid_observed_content(value, 1024 * 1024))
                .ok_or(RagSnapshotError::Invalid)?;
            let event_date = object
                .get("event_date")
                .and_then(Value::as_str)
                .or_else(|| object.get("recorded_at").and_then(Value::as_str))
                .filter(|timestamp| DateTime::parse_from_rfc3339(timestamp).is_ok())
                .ok_or(RagSnapshotError::Invalid)?;
            let entities = object
                .get("entities")
                .and_then(Value::as_array)
                .map(|entities| {
                    entities
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|entity| valid_observed_field(entity, 256))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            Ok(CandidateSource {
                source_id: event_id.to_string(),
                source_kind: SourceKind::Memory,
                collection: "command_memory".to_string(),
                document_id: event_id.to_string(),
                chunk_id: event_id.to_string(),
                timestamp: event_date.to_string(),
                location: format!("trusted-lan-observed; event {event_id}; entities {entities}"),
                retrieved_at: observed_at.to_string(),
                observed_at: observed_at.to_string(),
                quote: quote.to_string(),
            })
        })
        .collect()
}

fn valid_observed_field(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_observed_content(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .chars()
            .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
}
