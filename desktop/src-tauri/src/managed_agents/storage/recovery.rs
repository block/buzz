use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use tauri::AppHandle;

use super::{atomic_write_json_restricted, managed_agents_base_dir};
use crate::managed_agents::{
    ManagedAgentRecord, ManagedAgentRecoveryAuthority, ManagedAgentRecoveryStore,
    ManagedAgentRuntimeKey,
};

const MAX_RECOVERY_STORE_BYTES: u64 = 1024 * 1024;
static RECOVERY_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn recovery_store_guard() -> Result<MutexGuard<'static, ()>, String> {
    RECOVERY_STORE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "managed-agent recovery store lock is poisoned".to_string())
}

fn recovery_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("managed-agent-recovery.json"))
}

fn load_from_path(path: &Path) -> Result<ManagedAgentRecoveryStore, String> {
    if !path.exists() {
        return Ok(ManagedAgentRecoveryStore::default());
    }
    let size = fs::metadata(path)
        .map_err(|error| format!("failed to inspect managed-agent recovery store: {error}"))?
        .len();
    if size > MAX_RECOVERY_STORE_BYTES {
        return Err(format!(
            "managed-agent recovery store exceeds {MAX_RECOVERY_STORE_BYTES} bytes"
        ));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read managed-agent recovery store: {error}"))?;
    let migration_field_missing = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(serde_json::Value::Object(object)) => !object.contains_key("migrationComplete"),
        _ => false,
    };
    let mut store: ManagedAgentRecoveryStore = serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse managed-agent recovery store: {error}"))?;
    if store.version == 2 && store.authorities.is_empty() && migration_field_missing {
        store.migration_complete = true;
    }
    store.validate()?;
    Ok(store)
}

fn save_to_path(path: &Path, store: &ManagedAgentRecoveryStore) -> Result<(), String> {
    store.validate()?;
    let payload = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize managed-agent recovery store: {error}"))?;
    if payload.len() as u64 > MAX_RECOVERY_STORE_BYTES {
        return Err(format!(
            "managed-agent recovery store exceeds {MAX_RECOVERY_STORE_BYTES} bytes"
        ));
    }
    atomic_write_json_restricted(path, &payload)
}

fn update_path<T>(
    path: &Path,
    update: impl FnOnce(&mut ManagedAgentRecoveryStore) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = recovery_store_guard()?;
    let mut store = load_from_path(path)?;
    let original = store.clone();
    let result = update(&mut store)?;
    if store != original {
        save_to_path(path, &store)?;
    }
    Ok(result)
}

fn compact_tombstones_at_path(path: &Path, records: &[ManagedAgentRecord]) -> Result<(), String> {
    let _guard = recovery_store_guard()?;
    let mut store = load_from_path(path)?;
    let mut changed = false;
    let tombstones = store
        .authorities
        .iter()
        .filter_map(|(pubkey, authority)| authority.is_empty().then_some(pubkey.clone()))
        .collect::<Vec<_>>();
    for pubkey in tombstones {
        let compatibility_is_clear = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .map(authority_from_record)
            .is_none_or(|authority| authority.is_empty());
        if compatibility_is_clear {
            store.authorities.remove(&pubkey);
            store.migration_complete = true;
            changed = true;
        }
    }
    if changed {
        save_to_path(path, &store)?;
    }
    Ok(())
}

pub(super) fn compact_tombstones(
    app: &AppHandle,
    records: &[ManagedAgentRecord],
) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = (app, records);
        Ok(())
    }
    #[cfg(windows)]
    {
        compact_tombstones_at_path(&recovery_store_path(app)?, records)
    }
}

fn authority_from_record(record: &ManagedAgentRecord) -> ManagedAgentRecoveryAuthority {
    #[cfg(windows)]
    {
        ManagedAgentRecoveryAuthority::from_legacy(record.runtime_pid, record.last_error.as_deref())
    }
    #[cfg(not(windows))]
    {
        let _ = record;
        ManagedAgentRecoveryAuthority::default()
    }
}

fn reconcile_authority(
    mut authority: ManagedAgentRecoveryAuthority,
    record: &mut ManagedAgentRecord,
) -> Result<ManagedAgentRecoveryAuthority, String> {
    if !authority.accepts_compatibility_record(record) {
        authority.merge_compatibility_record(record)?;
        authority.project_compatibility(record);
        authority.capture_compatibility_snapshot(record);
    }
    Ok(authority)
}

fn authority_for_path(
    path: &Path,
    record: &ManagedAgentRecord,
) -> Result<ManagedAgentRecoveryAuthority, String> {
    let mut store = load_from_path(path)?;
    if record.runtime_pid.is_none() && crate::managed_agents::has_unverified_job_reap(record) {
        // Malformed fallback evidence is an admission blocker, not migration
        // authority. Preserve both persisted bytes and the caller's evidence.
        let mut authority = store
            .authorities
            .get(&record.pubkey)
            .cloned()
            .unwrap_or_default();
        authority.merge_compatibility_record(record)?;
        return Ok(authority);
    }
    if let Some(existing) = store.authorities.remove(&record.pubkey) {
        let mut projected_record = record.clone();
        let authority = reconcile_authority(existing, &mut projected_record)?;
        if projected_record != *record {
            store
                .authorities
                .insert(record.pubkey.clone(), authority.clone());
            save_to_path(path, &store)?;
        }
        return Ok(authority);
    }
    let mut authority = authority_from_record(record);
    if !authority.is_empty() {
        let mut projected_record = record.clone();
        authority.project_compatibility(&mut projected_record);
        authority.capture_compatibility_snapshot(&projected_record);
        store
            .authorities
            .insert(record.pubkey.clone(), authority.clone());
        save_to_path(path, &store)?;
    }
    Ok(authority)
}

pub(crate) fn authority_for(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Result<ManagedAgentRecoveryAuthority, String> {
    #[cfg(not(windows))]
    {
        let _ = (app, record);
        return Ok(ManagedAgentRecoveryAuthority::default());
    }
    #[cfg(windows)]
    {
        let _guard = recovery_store_guard()?;
        let path = recovery_store_path(app)?;
        authority_for_path(&path, record)
    }
}

pub(crate) fn admission_error(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
) -> Result<Option<String>, String> {
    Ok(authority_for(app, record)?.admission_error(key))
}

pub(crate) fn ensure_admission(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    match admission_error(app, record, key)? {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(crate) fn has_uncertainty(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Result<bool, String> {
    Ok(!authority_for(app, record)?.is_empty())
}

pub(crate) fn mark_pair_uncertain(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    pid: u32,
    detail: String,
) -> Result<String, String> {
    #[cfg(not(windows))]
    {
        let _ = (app, record, key, detail);
        return Ok(format!(
            "managed-agent recovery remains uncertain for pid {pid}"
        ));
    }
    #[cfg(windows)]
    {
        mark_pair_at_path(&recovery_store_path(app)?, record, key, pid, detail)
    }
}

fn mark_pair_at_path(
    path: &Path,
    record: &mut ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    pid: u32,
    detail: String,
) -> Result<String, String> {
    let mut projected = record.clone();
    let result = update_path(path, |store| {
        let authority = store
            .authorities
            .remove(&projected.pubkey)
            .unwrap_or_else(|| authority_from_record(&projected));
        let mut authority = reconcile_authority(authority, &mut projected)?;
        authority.mark_pair(key, pid, detail);
        authority.project_compatibility(&mut projected);
        authority.capture_compatibility_snapshot(&projected);
        store
            .authorities
            .insert(projected.pubkey.clone(), authority);
        Ok(projected
            .last_error
            .clone()
            .unwrap_or_else(|| format!("managed-agent recovery remains uncertain for pid {pid}")))
    })?;
    *record = projected;
    Ok(result)
}

pub(crate) fn clear_pair_with_terminal_proof(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
) -> Result<bool, String> {
    #[cfg(not(windows))]
    {
        let _ = (app, record, key);
        return Ok(false);
    }
    #[cfg(windows)]
    {
        clear_pair_at_path(&recovery_store_path(app)?, record, key)
    }
}

fn clear_pair_at_path(
    path: &Path,
    record: &mut ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
) -> Result<bool, String> {
    let mut projected = record.clone();
    let cleared = update_path(path, |store| {
        let existing = store.authorities.remove(&projected.pubkey);
        let had_entry = existing.is_some();
        let authority = existing.unwrap_or_else(|| authority_from_record(&projected));
        let mut authority = reconcile_authority(authority, &mut projected)?;
        let requires_tombstone = had_entry || !authority.is_empty();
        if requires_tombstone {
            authority.project_compatibility(&mut projected);
            authority.capture_compatibility_snapshot(&projected);
        }
        let cleared = authority.clear_pair_with_terminal_proof(key);
        if cleared {
            authority.project_compatibility(&mut projected);
        }
        if requires_tombstone {
            store
                .authorities
                .insert(projected.pubkey.clone(), authority);
        }
        Ok(cleared)
    })?;
    *record = projected;
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn key(pubkey_byte: &str, relay: &str) -> ManagedAgentRuntimeKey {
        ManagedAgentRuntimeKey::new(pubkey_byte.repeat(32), relay).unwrap()
    }

    #[test]
    fn concurrent_real_read_modify_write_transactions_preserve_both_agents() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("recovery.json"));
        let barrier = Arc::new(Barrier::new(3));
        let mut joins = Vec::new();
        for (owner, relay, pid) in [
            ("a1", "wss://a.example", 8101),
            ("b2", "wss://b.example", 8102),
        ] {
            let path = path.clone();
            let barrier = barrier.clone();
            joins.push(std::thread::spawn(move || {
                let runtime_key = key(owner, relay);
                barrier.wait();
                update_path(&path, |store| {
                    let mut authority = ManagedAgentRecoveryAuthority::default();
                    authority.mark_pair(&runtime_key, pid, "concurrent uncertainty".into());
                    store
                        .authorities
                        .insert(runtime_key.pubkey.clone(), authority);
                    Ok(())
                })
                .unwrap();
            }));
        }
        barrier.wait();
        for join in joins {
            join.join().unwrap();
        }
        let store = load_from_path(&path).unwrap();
        assert_eq!(store.authorities.len(), 2);
    }

    #[test]
    fn no_op_update_does_not_materialize_unmigrated_absence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        update_path(&path, |_| Ok(())).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn persisted_unmigrated_semantic_absence_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        fs::write(
            &path,
            r#"{"version":2,"authorities":{},"migrationComplete":false}"#,
        )
        .unwrap();
        assert!(load_from_path(&path)
            .unwrap_err()
            .contains("cannot persist unmigrated semantic absence"));
    }

    #[test]
    fn legacy_v2_empty_store_migrates_to_completed_without_authority() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        fs::write(&path, r#"{"version":2,"authorities":{}}"#).unwrap();
        let store = load_from_path(&path).unwrap();
        assert!(store.authorities.is_empty());
        assert!(store.migration_complete);
    }

    #[test]
    fn missing_pid_recovery_evidence_blocks_without_creating_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        let pubkey = "d5".repeat(32);
        let key = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://malformed.example").unwrap();
        let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
            r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
        ))
        .unwrap();
        record.last_error = Some(format!(
            "{} generation={} pair={} pid=9401; missing persisted PID",
            crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX,
            uuid::Uuid::new_v4(),
            serde_json::to_string(&key).unwrap()
        ));
        record.last_error_code = Some(73);
        let before = record.clone();

        let authority = authority_for_path(&path, &record).unwrap();

        assert!(authority.admission_error(&key).is_some());
        assert_eq!(record, before);
        assert!(!path.exists());
    }

    #[test]
    fn missing_pid_recovery_evidence_does_not_rewrite_empty_tombstone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        let pubkey = "d6".repeat(32);
        let key = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://malformed.example").unwrap();
        let mut authority = ManagedAgentRecoveryAuthority::default();
        let mut projected: ManagedAgentRecord = serde_json::from_str(&format!(
            r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
        ))
        .unwrap();
        authority.mark_pair(&key, 9402, "prior evidence".into());
        authority.project_compatibility(&mut projected);
        authority.capture_compatibility_snapshot(&projected);
        assert!(authority.clear_pair_with_terminal_proof(&key));
        let mut store = ManagedAgentRecoveryStore::default();
        store.authorities.insert(pubkey, authority);
        save_to_path(&path, &store).unwrap();

        projected.runtime_pid = None;
        projected.last_error = Some(format!(
            "{} generation={} pair={} pid=9402; missing persisted PID",
            crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX,
            uuid::Uuid::new_v4(),
            serde_json::to_string(&key).unwrap()
        ));
        projected.last_error_code = Some(74);
        let record_before = projected.clone();
        let sidecar_before = fs::read(&path).unwrap();

        let merged = authority_for_path(&path, &projected).unwrap();

        assert!(merged.admission_error(&key).is_some());
        assert_eq!(projected, record_before);
        assert_eq!(fs::read(path).unwrap(), sidecar_before);
    }

    #[test]
    fn failed_clear_sidecar_write_does_not_mutate_fallback_record() {
        let dir = tempfile::tempdir().unwrap();
        let blocked_parent = dir.path().join("not-a-directory");
        fs::write(&blocked_parent, "block child creation").unwrap();
        let path = blocked_parent.join("recovery.json");
        let runtime_key = key("d4", "wss://atomic-clear.example");
        let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
            r#"{{"pubkey":"{}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#,
            runtime_key.pubkey
        ))
        .unwrap();
        let mut authority = ManagedAgentRecoveryAuthority::default();
        authority.mark_pair(&runtime_key, 8301, "uncertain".into());
        authority.project_compatibility(&mut record);
        let before = record.clone();

        assert!(clear_pair_at_path(&path, &mut record, &runtime_key).is_err());
        assert_eq!(record, before);

        crate::managed_agents::record_terminal_proof_pending_recovery_clear(
            &mut record,
            &runtime_key,
            8301,
            "synthetic save failure",
        );
        authority.merge_compatibility_record(&record).unwrap();
        assert!(authority.agent_quarantine.is_none());
        assert!(authority.admission_error(&runtime_key).is_some());
    }

    #[test]
    fn oversized_valid_store_is_rejected_before_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        let mut store = ManagedAgentRecoveryStore::default();
        for agent in 0..300_u32 {
            let pubkey = format!("{agent:064x}");
            let runtime_key =
                ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://large.example").unwrap();
            let mut authority = ManagedAgentRecoveryAuthority::default();
            authority.mark_pair(&runtime_key, agent + 1, "x".repeat(4096));
            store.authorities.insert(pubkey, authority);
        }
        let error = save_to_path(&path, &store).unwrap_err();
        assert!(error.contains("exceeds 1048576 bytes"));
        assert!(!path.exists());
    }

    #[test]
    fn persisted_nonempty_sidecar_merges_divergent_record_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        let pubkey = "c3".repeat(32);
        let pair_a = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://a.example").unwrap();
        let pair_b = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://b.example").unwrap();
        update_path(&path, |store| {
            let mut authority = ManagedAgentRecoveryAuthority::default();
            authority.mark_pair(&pair_a, 8201, "A uncertain".into());
            store.authorities.insert(pubkey.clone(), authority);
            Ok(())
        })
        .unwrap();

        let mut fallback = ManagedAgentRecoveryAuthority::default();
        fallback.mark_pair(&pair_b, 8202, "B fallback".into());
        let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
            r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
        ))
        .unwrap();
        fallback.project_compatibility(&mut record);
        update_path(&path, |store| {
            let existing = store.authorities.remove(&pubkey).unwrap();
            let authority = reconcile_authority(existing, &mut record)?;
            assert!(authority.admission_error(&pair_a).is_some());
            assert!(authority.admission_error(&pair_b).is_some());
            store.authorities.insert(pubkey.clone(), authority);
            Ok(())
        })
        .unwrap();
        let reloaded = load_from_path(&path).unwrap();
        let authority = reloaded.authorities.get(&pubkey).unwrap();
        assert!(authority.admission_error(&pair_a).is_some());
        assert!(authority.admission_error(&pair_b).is_some());
    }

    #[test]
    fn resolved_tombstones_compact_to_bounded_global_migration_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.json");
        let mut records = Vec::new();
        update_path(&path, |store| {
            for agent in 0..200_u32 {
                let pubkey = format!("{agent:064x}");
                let runtime_key =
                    ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://compact.example").unwrap();
                let mut authority = ManagedAgentRecoveryAuthority::default();
                authority.mark_pair(&runtime_key, agent + 1, "x".repeat(4096));
                let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
                    r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
                ))
                .unwrap();
                authority.project_compatibility(&mut record);
                authority.capture_compatibility_snapshot(&record);
                assert!(authority.clear_pair_with_terminal_proof(&runtime_key));
                authority.project_compatibility(&mut record);
                records.push(record);
                store.authorities.insert(pubkey, authority);
            }
            Ok(())
        })
        .unwrap();
        records.pop();

        compact_tombstones_at_path(&path, &records).unwrap();
        let store = load_from_path(&path).unwrap();
        assert!(store.authorities.is_empty());
        assert!(store.migration_complete);
        assert!(fs::metadata(path).unwrap().len() < 100_000);
    }
}
