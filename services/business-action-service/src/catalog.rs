use business_action_contracts::{ActionCatalogEntry, CATALOG_VERSION};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFile {
    version: String,
    effective_from: DateTime<Utc>,
    entries: Vec<CatalogFileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFileEntry {
    action_code: String,
    title: String,
    description: String,
    types: Vec<String>,
    targets: Vec<String>,
    due_days: u16,
    roles: Vec<String>,
    draft_type: Option<String>,
}

pub fn bundled_catalog() -> Result<Vec<ActionCatalogEntry>, String> {
    parse_catalog(
        include_str!("../catalog/trade-action-v1.0.json"),
        CATALOG_VERSION,
    )
}

pub fn catalog_from_path(
    path: &Path,
    expected_version: &str,
) -> Result<Vec<ActionCatalogEntry>, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|_| "Action Catalog path is unreadable".to_string())?;
    parse_catalog(&contents, expected_version)
}

fn parse_catalog(
    contents: &str,
    expected_version: &str,
) -> Result<Vec<ActionCatalogEntry>, String> {
    let file: CatalogFile =
        serde_json::from_str(contents).map_err(|_| "invalid bundled Action Catalog".to_string())?;
    if file.version != CATALOG_VERSION
        || file.version != expected_version
        || file.entries.len() < 21
    {
        return Err("Action Catalog version or required entries are invalid".into());
    }
    let mut codes = BTreeSet::new();
    let mut out = Vec::with_capacity(file.entries.len());
    for entry in file.entries {
        if !safe_code(&entry.action_code)
            || !codes.insert(entry.action_code.clone())
            || entry.types.is_empty()
            || entry.targets.is_empty()
            || !(1..=30).contains(&entry.due_days)
            || entry.roles.is_empty()
        {
            return Err("Action Catalog contains an invalid entry".into());
        }
        let canonical = serde_json::json!({
            "version": file.version,
            "actionCode": entry.action_code,
            "title": entry.title,
            "description": entry.description,
            "types": entry.types,
            "targets": entry.targets,
            "dueDays": entry.due_days,
            "roles": entry.roles,
            "draftType": entry.draft_type,
        });
        let bytes = serde_json::to_vec(&canonical).map_err(|_| "catalog hash failed")?;
        let config_hash = hex::encode(Sha256::digest(bytes));
        let id = stable_uuid(format!("{}:{}", file.version, entry.action_code).as_bytes());
        out.push(ActionCatalogEntry {
            id,
            version: file.version.clone(),
            action_code: entry.action_code,
            title: entry.title,
            description: entry.description,
            supported_anomaly_types: entry.types,
            target_resource_types: entry.targets,
            default_due_days: entry.due_days,
            allowed_assignee_role_keys: entry.roles,
            approval_draft_type: entry.draft_type,
            requires_explicit_confirmation: true,
            enabled: true,
            effective_from: file.effective_from,
            effective_to: None,
            config_hash,
        });
    }
    Ok(out)
}

fn safe_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

pub fn stable_uuid(value: &[u8]) -> Uuid {
    let digest = Sha256::digest(value);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_versioned_unique_and_server_controlled() {
        let catalog = bundled_catalog().expect("catalog");
        assert_eq!(catalog.len(), 21);
        assert!(catalog.iter().all(|entry| {
            entry.version == CATALOG_VERSION
                && entry.requires_explicit_confirmation
                && entry.config_hash.len() == 64
        }));
        assert!(catalog
            .iter()
            .any(|entry| entry.action_code == "review_future_shipment_risk"));
        assert!(!catalog
            .iter()
            .any(|entry| entry.action_code == "approve_or_execute"));
    }
}
