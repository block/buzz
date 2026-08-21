//! Owner-scoped, read-only Activity Ledger snapshot for local consumers.
//!
//! The frontend already owns the canonical journal projection. This module is
//! deliberately only the secure persistence seam: it validates the projection
//! envelope against the active owner, writes it atomically with mode 0600 on
//! Unix, and revalidates it on read. It never receives or serializes secret
//! identity key material.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const TODAY_SNAPSHOT_SCHEMA: &str = "buzz.activity-ledger.today/v1";
pub const TODAY_SNAPSHOT_CAPABILITY: &str = "buzz.activity-ledger.today.read/v1";
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RAW_EVENTS: usize = 10_000;
const MAX_LIFETIME_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerTodaySnapshot {
    pub schema: String,
    pub owner_pubkey: String,
    pub generated_at: i64,
    pub expires_at: i64,
    pub capability: String,
    pub surface: serde_json::Value,
    pub raw_events: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodaySnapshotReceipt {
    pub path: String,
    pub owner_pubkey: String,
    pub generated_at: i64,
    pub expires_at: i64,
    pub byte_length: usize,
    pub sha256: String,
}

fn snapshot_path(nest_dir: &Path, owner_pubkey: &str) -> PathBuf {
    nest_dir
        .join("archive")
        .join(format!("activity-ledger-today-{owner_pubkey}.json"))
}

fn validate_owner_pubkey(owner_pubkey: &str) -> Result<(), String> {
    if owner_pubkey.len() != 64
        || !owner_pubkey
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("Today snapshot ownerPubkey must be lowercase 64-character hex".into());
    }
    Ok(())
}

fn reject_identity_secret_material(value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                let normalized_key = key
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if matches!(
                    normalized_key.as_str(),
                    "nsec" | "secretkey" | "privatekey" | "nostrsecretkey"
                ) {
                    return Err("Today snapshot cannot contain identity secret fields".into());
                }
                reject_identity_secret_material(child)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                reject_identity_secret_material(child)?;
            }
        }
        serde_json::Value::String(text) => {
            let lowercase = text.to_ascii_lowercase();
            if lowercase.contains("nsec1") || lowercase.contains("nostr_secret_key=") {
                return Err("Today snapshot cannot contain identity secret material".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_and_validate(
    snapshot_json: &str,
    expected_owner_pubkey: &str,
    now: i64,
    require_unexpired: bool,
) -> Result<OwnerTodaySnapshot, String> {
    if snapshot_json.is_empty() || snapshot_json.len() > MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "Today snapshot must contain between 1 and {MAX_SNAPSHOT_BYTES} bytes"
        ));
    }
    validate_owner_pubkey(expected_owner_pubkey)?;
    let snapshot: OwnerTodaySnapshot = serde_json::from_str(snapshot_json)
        .map_err(|error| format!("parse Today snapshot: {error}"))?;
    if snapshot.schema != TODAY_SNAPSHOT_SCHEMA {
        return Err("unsupported Today snapshot schema".into());
    }
    if snapshot.capability != TODAY_SNAPSHOT_CAPABILITY {
        return Err("unsupported Today snapshot capability".into());
    }
    if snapshot.owner_pubkey != expected_owner_pubkey {
        return Err("Today snapshot owner does not match the active identity".into());
    }
    if snapshot.generated_at > now + 300 {
        return Err("Today snapshot generatedAt is too far in the future".into());
    }
    if snapshot.expires_at <= snapshot.generated_at
        || snapshot.expires_at - snapshot.generated_at > MAX_LIFETIME_SECS
    {
        return Err("Today snapshot lifetime must be positive and at most 24 hours".into());
    }
    if require_unexpired && snapshot.expires_at <= now {
        return Err("Today snapshot has expired".into());
    }
    if !snapshot.surface.is_object() {
        return Err("Today snapshot surface must be a JSON object".into());
    }
    if snapshot.raw_events.len() > MAX_RAW_EVENTS {
        return Err(format!(
            "Today snapshot rawEvents exceeds {MAX_RAW_EVENTS} records"
        ));
    }
    reject_identity_secret_material(&snapshot.surface)?;
    for event in &snapshot.raw_events {
        reject_identity_secret_material(event)?;
    }
    Ok(snapshot)
}

#[cfg(unix)]
fn secure_open(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("create Today snapshot temp file: {error}"))
}

#[cfg(not(unix))]
fn secure_open(path: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create Today snapshot temp file: {error}"))
}

#[cfg(unix)]
fn enforce_private_permissions(path: &Path, directory: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| format!("set private Today snapshot permissions: {error}"))
}

#[cfg(not(unix))]
fn enforce_private_permissions(_path: &Path, _directory: bool) -> Result<(), String> {
    Ok(())
}

pub fn write_owner_today_snapshot(
    nest_dir: &Path,
    expected_owner_pubkey: &str,
    snapshot_json: &str,
    now: i64,
) -> Result<TodaySnapshotReceipt, String> {
    let snapshot = parse_and_validate(snapshot_json, expected_owner_pubkey, now, true)?;
    // Canonicalize the bytes that local readers hash and consume.
    let canonical_json = serde_json::to_string(&snapshot)
        .map_err(|error| format!("serialize canonical Today snapshot: {error}"))?;
    let archive_dir = nest_dir.join("archive");
    std::fs::create_dir_all(&archive_dir)
        .map_err(|error| format!("create Today snapshot directory: {error}"))?;
    enforce_private_permissions(&archive_dir, true)?;

    let destination = snapshot_path(nest_dir, expected_owner_pubkey);
    let temp_path = archive_dir.join(format!(
        ".activity-ledger-today-{}.{}.tmp",
        expected_owner_pubkey,
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| -> Result<(), String> {
        let mut file = secure_open(&temp_path)?;
        file.write_all(canonical_json.as_bytes())
            .map_err(|error| format!("write Today snapshot: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync Today snapshot: {error}"))?;
        drop(file);
        enforce_private_permissions(&temp_path, false)?;
        std::fs::rename(&temp_path, &destination)
            .map_err(|error| format!("atomically publish Today snapshot: {error}"))?;
        enforce_private_permissions(&destination, false)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    write_result?;

    let sha256 = hex::encode(Sha256::digest(canonical_json.as_bytes()));
    Ok(TodaySnapshotReceipt {
        path: destination.to_string_lossy().into_owned(),
        owner_pubkey: snapshot.owner_pubkey,
        generated_at: snapshot.generated_at,
        expires_at: snapshot.expires_at,
        byte_length: canonical_json.len(),
        sha256,
    })
}

pub fn read_owner_today_snapshot(
    nest_dir: &Path,
    expected_owner_pubkey: &str,
    now: i64,
) -> Result<String, String> {
    let path = snapshot_path(nest_dir, expected_owner_pubkey);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("read owner Today snapshot: {error}"))?;
    parse_and_validate(&raw, expected_owner_pubkey, now, true)?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(owner: &str, generated_at: i64) -> String {
        serde_json::json!({
            "schema": TODAY_SNAPSHOT_SCHEMA,
            "ownerPubkey": owner,
            "generatedAt": generated_at,
            "expiresAt": generated_at + 3600,
            "capability": TODAY_SNAPSHOT_CAPABILITY,
            "surface": {"date": "2026-08-21", "journals": []},
            "rawEvents": [{"journalId": "j-1", "proofState": "OBSERVED"}]
        })
        .to_string()
    }

    #[test]
    fn snapshot_is_atomic_private_and_owner_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let owner = "a".repeat(64);
        let receipt =
            write_owner_today_snapshot(dir.path(), &owner, &snapshot(&owner, 1000), 1000).unwrap();
        assert_eq!(receipt.owner_pubkey, owner);
        assert_eq!(receipt.sha256.len(), 64);
        let raw = read_owner_today_snapshot(dir.path(), &owner, 1001).unwrap();
        assert!(raw.contains(TODAY_SNAPSHOT_CAPABILITY));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&receipt.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn snapshot_wrong_owner_expiry_and_capability_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let owner = "a".repeat(64);
        let other = "b".repeat(64);
        assert!(
            write_owner_today_snapshot(dir.path(), &other, &snapshot(&owner, 1000), 1000)
                .unwrap_err()
                .contains("owner does not match")
        );

        let mut value: serde_json::Value = serde_json::from_str(&snapshot(&owner, 1000)).unwrap();
        value["capability"] = "write-anything".into();
        assert!(
            write_owner_today_snapshot(dir.path(), &owner, &value.to_string(), 1000)
                .unwrap_err()
                .contains("capability")
        );

        assert!(
            write_owner_today_snapshot(dir.path(), &owner, &snapshot(&owner, 1000), 5000)
                .unwrap_err()
                .contains("expired")
        );
    }

    #[test]
    fn snapshot_read_revalidates_tampering_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let owner = "a".repeat(64);
        let first =
            write_owner_today_snapshot(dir.path(), &owner, &snapshot(&owner, 1000), 1000).unwrap();
        let second =
            write_owner_today_snapshot(dir.path(), &owner, &snapshot(&owner, 1001), 1001).unwrap();
        assert_eq!(first.path, second.path);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&second.path).unwrap()).unwrap();
        value["ownerPubkey"] = "b".repeat(64).into();
        std::fs::write(&second.path, value.to_string()).unwrap();
        assert!(read_owner_today_snapshot(dir.path(), &owner, 1002)
            .unwrap_err()
            .contains("owner does not match"));
    }

    #[test]
    fn snapshot_rejects_identity_secret_fields_and_nsec_values() {
        let dir = tempfile::tempdir().unwrap();
        let owner = "a".repeat(64);
        let mut field: serde_json::Value = serde_json::from_str(&snapshot(&owner, 1000)).unwrap();
        field["surface"]["secretKey"] = "do-not-export".into();
        assert!(
            write_owner_today_snapshot(dir.path(), &owner, &field.to_string(), 1000)
                .unwrap_err()
                .contains("identity secret")
        );

        let mut value: serde_json::Value = serde_json::from_str(&snapshot(&owner, 1000)).unwrap();
        value["rawEvents"][0]["detail"] = "nsec1should-never-leave-the-owner-boundary".into();
        assert!(
            write_owner_today_snapshot(dir.path(), &owner, &value.to_string(), 1000)
                .unwrap_err()
                .contains("identity secret")
        );
    }
}
