use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use super::storage::{atomic_write_json_restricted, backup_invalid_store, managed_agents_base_dir};
use crate::app_state::AppState;

const STORE_FILENAME: &str = "registered-agent-references.json";
const AGENTS_DATA_CHANGED_EVENT: &str = "agents-data-changed";
const LABEL_LIMIT_BYTES: usize = 80;
const ROLE_SUMMARY_LIMIT_BYTES: usize = 240;

/// A keyless reference to an already-existing agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredAgentReference {
    /// The referenced agent public key as normalized 64-byte lowercase hex.
    pub pubkey: String,
    /// Optional user-facing label for the reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional short description of the agent's role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_summary: Option<String>,
    /// Creation timestamp in ISO-8601 UTC form.
    pub created_at: String,
    /// Last update timestamp in ISO-8601 UTC form.
    pub updated_at: String,
}

/// Request payload for registering or updating an existing agent reference.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterAgentReferenceRequest {
    /// Agent public key, accepted with leading/trailing whitespace and mixed case.
    pub pubkey: String,
    /// Optional label; blank strings are stored as `None`.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional role summary; blank strings are stored as `None`.
    #[serde(default)]
    pub role_summary: Option<String>,
}

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(STORE_FILENAME))
}

fn normalize_pubkey(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("agent pubkey must be exactly 64 ASCII hex characters".to_string());
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn normalize_optional(
    value: Option<String>,
    limit: usize,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > limit {
        return Err(format!("{field} must be at most {limit} UTF-8 bytes"));
    }
    Ok(Some(trimmed.to_string()))
}

fn load_from_path(path: &Path) -> Result<Vec<RegisteredAgentReference>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read registered agent references: {error}"))?;
    let mut refs: Vec<RegisteredAgentReference> =
        serde_json::from_str(&content).map_err(|error| {
            backup_invalid_store(path);
            format!("failed to parse registered agent references (preserved as .invalid): {error}")
        })?;
    refs.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    Ok(refs)
}

fn save_to_path(path: &Path, refs: &[RegisteredAgentReference]) -> Result<(), String> {
    let mut refs = refs.to_vec();
    refs.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    let payload = serde_json::to_vec_pretty(&refs)
        .map_err(|error| format!("failed to serialize registered agent references: {error}"))?;
    atomic_write_json_restricted(path, &payload)
}

/// Load all registered existing-agent references.
#[tauri::command]
pub fn list_registered_agent_references(
    app: AppHandle,
) -> Result<Vec<RegisteredAgentReference>, String> {
    load_from_path(&store_path(&app)?)
}

/// Register or update a keyless reference to an existing agent identity.
#[tauri::command]
pub fn register_existing_agent_reference(
    input: RegisterAgentReferenceRequest,
    app: AppHandle,
) -> Result<RegisteredAgentReference, String> {
    let pubkey = normalize_pubkey(&input.pubkey)?;
    let label = normalize_optional(input.label, LABEL_LIMIT_BYTES, "label")?;
    let role_summary =
        normalize_optional(input.role_summary, ROLE_SUMMARY_LIMIT_BYTES, "roleSummary")?;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    if super::storage::managed_agent_record_exists(&app, &pubkey)? {
        return Err(format!("agent {pubkey} is already a managed agent"));
    }
    let path = store_path(&app)?;
    let mut refs = load_from_path(&path)?;
    let now = crate::util::now_iso();
    let reference = match refs.iter_mut().find(|reference| reference.pubkey == pubkey) {
        Some(existing) => {
            existing.label = label;
            existing.role_summary = role_summary;
            existing.updated_at = now;
            existing.clone()
        }
        None => {
            let reference = RegisteredAgentReference {
                pubkey,
                label,
                role_summary,
                created_at: now.clone(),
                updated_at: now,
            };
            refs.push(reference.clone());
            reference
        }
    };
    save_to_path(&path, &refs)?;
    let _ = app.emit(AGENTS_DATA_CHANGED_EVENT, ());
    Ok(reference)
}

/// Remove a registered existing-agent reference by public key.
#[tauri::command]
pub fn unregister_existing_agent_reference(pubkey: String, app: AppHandle) -> Result<(), String> {
    let pubkey = normalize_pubkey(&pubkey)?;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let path = store_path(&app)?;
    let mut refs = load_from_path(&path)?;
    let initial_len = refs.len();
    refs.retain(|reference| reference.pubkey != pubkey);
    if refs.len() == initial_len {
        return Err(format!("agent {pubkey} not found"));
    }
    save_to_path(&path, &refs)?;
    let _ = app.emit(AGENTS_DATA_CHANGED_EVENT, ());
    Ok(())
}

/// Require a managed-agent record before any lifecycle/config/delete side
/// effect. Registered references and unknown pubkeys both fail closed.
pub(crate) fn reject_registered_reference_target(
    app: &AppHandle,
    pubkey: &str,
) -> Result<(), String> {
    let normalized = normalize_pubkey(pubkey).unwrap_or_else(|_| pubkey.to_string());
    if !super::storage::managed_agent_record_exists(app, &normalized)? {
        return Err(format!("agent {normalized} not found"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    const PUBKEY_A_UPPER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const PUBKEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PUBKEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn request(
        pubkey: &str,
        label: Option<&str>,
        role_summary: Option<&str>,
    ) -> RegisterAgentReferenceRequest {
        RegisterAgentReferenceRequest {
            pubkey: pubkey.to_string(),
            label: label.map(str::to_string),
            role_summary: role_summary.map(str::to_string),
        }
    }

    #[test]
    fn normalizes_pubkey_and_blank_optionals() {
        let normalized = normalize_pubkey(&format!("  {PUBKEY_A_UPPER}  ")).unwrap();
        assert_eq!(normalized, PUBKEY_A);
        assert_eq!(
            normalize_optional(Some("   ".to_string()), LABEL_LIMIT_BYTES, "label").unwrap(),
            None
        );
        assert_eq!(
            normalize_optional(Some("  hello  ".to_string()), LABEL_LIMIT_BYTES, "label").unwrap(),
            Some("hello".to_string())
        );
    }

    #[test]
    fn rejects_non_exact_ascii_hex_pubkeys_and_overlong_fields() {
        assert!(normalize_pubkey("abc").is_err());
        assert!(normalize_pubkey(
            "g000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        assert!(normalize_optional(Some("é".repeat(41)), LABEL_LIMIT_BYTES, "label").is_err());
        assert!(normalize_optional(
            Some("x".repeat(241)),
            ROLE_SUMMARY_LIMIT_BYTES,
            "roleSummary"
        )
        .is_err());
    }

    #[test]
    fn save_load_sorts_by_pubkey_and_uses_restricted_mode() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let refs = vec![
            RegisteredAgentReference {
                pubkey: PUBKEY_B.to_string(),
                label: None,
                role_summary: None,
                created_at: "2026-01-02T00:00:00Z".to_string(),
                updated_at: "2026-01-02T00:00:00Z".to_string(),
            },
            RegisteredAgentReference {
                pubkey: PUBKEY_A.to_string(),
                label: Some("A".to_string()),
                role_summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];

        save_to_path(&path, &refs).unwrap();
        let loaded = load_from_path(&path).unwrap();

        assert_eq!(
            loaded.iter().map(|r| r.pubkey.as_str()).collect::<Vec<_>>(),
            vec![PUBKEY_A, PUBKEY_B]
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn malformed_json_fails_closed_and_preserves_invalid_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let bytes = b"{ definitely not json";
        fs::write(&path, bytes).unwrap();

        let error = load_from_path(&path).unwrap_err();

        assert!(error.contains("failed to parse registered agent references"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::read(path.with_extension("json.invalid")).unwrap(),
            bytes
        );
    }

    #[test]
    fn update_preserves_created_at_and_replaces_optionals() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let mut refs = Vec::new();
        let first = upsert_for_test(
            &path,
            request(PUBKEY_A, Some("one"), Some("old")),
            &mut refs,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let second = upsert_for_test(
            &path,
            request(PUBKEY_A_UPPER, Some("two"), None),
            &mut refs,
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

        assert_eq!(first.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.updated_at, "2026-01-02T00:00:00Z");
        assert_eq!(second.label.as_deref(), Some("two"));
        assert_eq!(second.role_summary, None);
    }

    #[test]
    fn request_denies_unknown_fields() {
        let json = format!(r#"{{"pubkey":"{PUBKEY_A}","label":"hi","privateKeyNsec":"nope"}}"#);
        let error = serde_json::from_str::<RegisterAgentReferenceRequest>(&json).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    fn upsert_for_test(
        path: &Path,
        input: RegisterAgentReferenceRequest,
        refs: &mut Vec<RegisteredAgentReference>,
        now: &str,
    ) -> Result<RegisteredAgentReference, String> {
        let pubkey = normalize_pubkey(&input.pubkey)?;
        let label = normalize_optional(input.label, LABEL_LIMIT_BYTES, "label")?;
        let role_summary =
            normalize_optional(input.role_summary, ROLE_SUMMARY_LIMIT_BYTES, "roleSummary")?;
        let reference = match refs.iter_mut().find(|reference| reference.pubkey == pubkey) {
            Some(existing) => {
                existing.label = label;
                existing.role_summary = role_summary;
                existing.updated_at = now.to_string();
                existing.clone()
            }
            None => {
                let reference = RegisteredAgentReference {
                    pubkey,
                    label,
                    role_summary,
                    created_at: now.to_string(),
                    updated_at: now.to_string(),
                };
                refs.push(reference.clone());
                reference
            }
        };
        save_to_path(path, refs)?;
        Ok(reference)
    }
}
