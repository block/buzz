use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

const BUZZ_RELEASE_IDENTIFIER_PREFIX: &str = "xyz.block.buzz.app";
const SPROUT_RELEASE_IDENTIFIER: &str = "xyz.block.sprout.app";
const BUZZ_DEV_IDENTIFIER_PREFIX: &str = "xyz.block.buzz.app.dev";
const SPROUT_DEV_IDENTIFIER_PREFIX: &str = "xyz.block.sprout.app.dev";

const SPROUT_WORKSPACES_KEY: &str = "sprout-workspaces";
const SPROUT_ACTIVE_WORKSPACE_KEY: &str = "sprout-active-workspace-id";
const SPROUT_ONBOARDING_COMPLETE_PREFIX: &str = "sprout-onboarding-complete.v1:";

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyWorkspaceStorage {
    workspaces: Option<String>,
    active_workspace_id: Option<String>,
    onboarding_completions: Vec<LegacyOnboardingCompletion>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyOnboardingCompletion {
    pubkey: String,
    value: String,
}

fn legacy_identifier(current_identifier: &str) -> Option<String> {
    if current_identifier.starts_with(BUZZ_DEV_IDENTIFIER_PREFIX) {
        Some(current_identifier.replacen(
            BUZZ_DEV_IDENTIFIER_PREFIX,
            SPROUT_DEV_IDENTIFIER_PREFIX,
            1,
        ))
    } else if current_identifier.starts_with(BUZZ_RELEASE_IDENTIFIER_PREFIX) {
        Some(current_identifier.replacen(
            BUZZ_RELEASE_IDENTIFIER_PREFIX,
            SPROUT_RELEASE_IDENTIFIER,
            1,
        ))
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn legacy_webkit_data_root(identifier: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join("Library")
            .join("WebKit")
            .join(identifier)
            .join("WebsiteData")
    })
}

#[cfg(not(target_os = "macos"))]
fn legacy_webkit_data_root(_identifier: &str) -> Option<PathBuf> {
    None
}

fn collect_local_storage_databases(root: &Path, databases: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_local_storage_databases(&path, databases);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("localstorage.sqlite3") {
            databases.push(path);
        }
    }
}

fn decode_webkit_local_storage_value(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }

    if bytes.len().is_multiple_of(2) {
        let has_utf16_ascii_shape = bytes.chunks_exact(2).any(|chunk| chunk[1] == 0);
        if has_utf16_ascii_shape {
            let utf16: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect();
            if let Ok(value) = String::from_utf16(&utf16) {
                return Some(value.trim_end_matches('\0').to_string());
            }
        }
    }

    String::from_utf8(bytes.to_vec()).ok()
}

fn read_legacy_workspace_storage_db(path: &Path) -> Result<LegacyWorkspaceStorage, String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("open legacy localStorage db: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT key, value FROM ItemTable \
             WHERE key = ?1 OR key = ?2 OR key LIKE ?3",
        )
        .map_err(|e| format!("prepare legacy localStorage query: {e}"))?;
    let mut rows = stmt
        .query([
            SPROUT_WORKSPACES_KEY,
            SPROUT_ACTIVE_WORKSPACE_KEY,
            &format!("{SPROUT_ONBOARDING_COMPLETE_PREFIX}%"),
        ])
        .map_err(|e| format!("query legacy localStorage: {e}"))?;

    let mut result = LegacyWorkspaceStorage::default();
    while let Some(row) = rows
        .next()
        .map_err(|e| format!("read legacy localStorage row: {e}"))?
    {
        let key: String = row
            .get(0)
            .map_err(|e| format!("read legacy localStorage key: {e}"))?;
        let value_bytes: Vec<u8> = row
            .get(1)
            .map_err(|e| format!("read legacy localStorage value: {e}"))?;
        let Some(value) = decode_webkit_local_storage_value(&value_bytes) else {
            continue;
        };

        if key == SPROUT_WORKSPACES_KEY {
            result.workspaces = Some(value);
        } else if key == SPROUT_ACTIVE_WORKSPACE_KEY {
            result.active_workspace_id = Some(value);
        } else if let Some(pubkey) = key.strip_prefix(SPROUT_ONBOARDING_COMPLETE_PREFIX) {
            result
                .onboarding_completions
                .push(LegacyOnboardingCompletion {
                    pubkey: pubkey.to_string(),
                    value,
                });
        }
    }

    Ok(result)
}

fn merge_legacy_workspace_storage(
    target: &mut LegacyWorkspaceStorage,
    source: LegacyWorkspaceStorage,
) {
    if target.workspaces.is_none() {
        target.workspaces = source.workspaces;
    }
    if target.active_workspace_id.is_none() {
        target.active_workspace_id = source.active_workspace_id;
    }
    target
        .onboarding_completions
        .extend(source.onboarding_completions);
}

/// Return workspace-scoped localStorage values from the legacy Sprout WebKit
/// data directory so the frontend can seed Buzz localStorage before first
/// render. This is separate from `migrate_legacy_app_data_dir`: Tauri app data
/// migration copies files such as `identity.key`, but WebKit localStorage lives
/// under `~/Library/WebKit/<identifier>/...` on macOS and is not included in the
/// app data directory.
#[tauri::command]
pub async fn get_legacy_workspace_storage(
    app: tauri::AppHandle,
) -> Result<LegacyWorkspaceStorage, String> {
    let identifier = app.config().identifier.clone();
    tokio::task::spawn_blocking(move || {
        let Some(identifier) = legacy_identifier(&identifier) else {
            return Ok(LegacyWorkspaceStorage::default());
        };
        let Some(root) = legacy_webkit_data_root(&identifier) else {
            return Ok(LegacyWorkspaceStorage::default());
        };
        if !root.exists() {
            return Ok(LegacyWorkspaceStorage::default());
        }

        let mut databases = Vec::new();
        collect_local_storage_databases(&root, &mut databases);

        let mut result = LegacyWorkspaceStorage::default();
        for database in databases {
            match read_legacy_workspace_storage_db(&database) {
                Ok(storage) => merge_legacy_workspace_storage(&mut result, storage),
                Err(error) => eprintln!(
                    "buzz-desktop: legacy-local-storage-migration: {}: {error}",
                    database.display()
                ),
            }
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_identifier_maps_release_identifier() {
        assert_eq!(
            legacy_identifier("xyz.block.buzz.app"),
            Some("xyz.block.sprout.app".to_string())
        );
    }

    #[test]
    fn legacy_identifier_maps_dev_worktree_identifier() {
        assert_eq!(
            legacy_identifier("xyz.block.buzz.app.dev.my-branch"),
            Some("xyz.block.sprout.app.dev.my-branch".to_string())
        );
    }

    #[test]
    fn decode_webkit_local_storage_value_reads_utf16le() {
        let bytes: Vec<u8> = "true".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            decode_webkit_local_storage_value(&bytes).as_deref(),
            Some("true")
        );
    }

    #[test]
    fn decode_webkit_local_storage_value_reads_utf8_fallback() {
        assert_eq!(
            decode_webkit_local_storage_value(b"plain utf8").as_deref(),
            Some("plain utf8")
        );
    }

    #[test]
    fn read_legacy_workspace_storage_db_reads_workspace_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("localstorage.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB NOT NULL ON CONFLICT FAIL)",
            [],
        )
        .unwrap();

        fn utf16le(value: &str) -> Vec<u8> {
            value.encode_utf16().flat_map(u16::to_le_bytes).collect()
        }

        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            (
                SPROUT_WORKSPACES_KEY,
                utf16le("[{\"relayUrl\":\"wss://relay.example.com\"}]"),
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            (SPROUT_ACTIVE_WORKSPACE_KEY, utf16le("workspace-1")),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            (
                format!("{SPROUT_ONBOARDING_COMPLETE_PREFIX}abc123"),
                utf16le("true"),
            ),
        )
        .unwrap();
        drop(conn);

        let storage = read_legacy_workspace_storage_db(&path).unwrap();
        assert_eq!(
            storage.workspaces.as_deref(),
            Some("[{\"relayUrl\":\"wss://relay.example.com\"}]")
        );
        assert_eq!(storage.active_workspace_id.as_deref(), Some("workspace-1"));
        assert_eq!(storage.onboarding_completions.len(), 1);
        assert_eq!(storage.onboarding_completions[0].pubkey, "abc123");
        assert_eq!(storage.onboarding_completions[0].value, "true");
    }
}
