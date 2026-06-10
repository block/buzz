//! Typesense collection schema management.

use serde_json::json;
use tracing::{debug, info, warn};

use crate::error::SearchError;

/// Returns the Typesense collection schema JSON for the events collection.
pub fn events_schema(collection_name: &str) -> serde_json::Value {
    json!({
        "name": collection_name,
        "fields": [
            {"name": "id",          "type": "string"},
            {"name": "content",     "type": "string"},
            {"name": "kind",        "type": "int32"},
            {"name": "pubkey",      "type": "string", "facet": true},
            {"name": "channel_id",  "type": "string", "facet": true, "optional": true},
            {"name": "created_at",  "type": "int64"},
            {"name": "tags_flat",   "type": "string[]", "optional": true}
        ],
        "default_sorting_field": "created_at"
    })
}

/// Ensures the Typesense collection exists, creating it if absent (idempotent).
pub async fn ensure_collection(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    collection_name: &str,
) -> Result<(), SearchError> {
    let check_url = format!("{}/collections/{}", base_url, collection_name);
    let resp = client
        .get(&check_url)
        .header("X-TYPESENSE-API-KEY", api_key)
        .send()
        .await?;

    match resp.status().as_u16() {
        200 => {
            debug!(collection = collection_name, "Collection already exists");
            return Ok(());
        }
        404 => {
            debug!(
                collection = collection_name,
                "Collection not found, creating"
            );
        }
        status => {
            let body = resp.text().await.unwrap_or_default();
            return Err(SearchError::Api { status, body });
        }
    }

    let schema = events_schema(collection_name);
    let create_url = format!("{}/collections", base_url);
    let resp = client
        .post(&create_url)
        .header("X-TYPESENSE-API-KEY", api_key)
        .header("Content-Type", "application/json")
        .json(&schema)
        .send()
        .await?;

    let status = resp.status().as_u16();
    match status {
        200 | 201 => {
            info!(
                collection = collection_name,
                "Collection created successfully"
            );
            Ok(())
        }
        409 => {
            // Race condition: another process created it between our check and create.
            warn!(
                collection = collection_name,
                "Collection created concurrently (409 conflict), treating as success"
            );
            Ok(())
        }
        _ => {
            let body = resp.text().await.unwrap_or_default();
            Err(SearchError::Api { status, body })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_events_schema_structure() {
        let schema = events_schema("events");
        assert_eq!(schema["name"], "events");
        assert_eq!(schema["default_sorting_field"], "created_at");

        let fields = schema["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 7);

        let field_names: Vec<&str> = fields.iter().map(|f| f["name"].as_str().unwrap()).collect();
        for expected in [
            "id",
            "content",
            "kind",
            "pubkey",
            "channel_id",
            "created_at",
            "tags_flat",
        ] {
            assert!(field_names.contains(&expected));
        }
    }

    #[test]
    fn test_events_schema_field_types() {
        let schema = events_schema("test");
        let fields = schema["fields"].as_array().unwrap();
        let find = |name: &str| fields.iter().find(|f| f["name"] == name).unwrap().clone();

        assert_eq!(find("kind")["type"], "int32");
        assert_eq!(find("pubkey")["facet"], true);
        assert_eq!(find("channel_id")["optional"], true);
        assert_eq!(find("created_at")["type"], "int64");
        assert_eq!(find("tags_flat")["type"], "string[]");
    }
}
