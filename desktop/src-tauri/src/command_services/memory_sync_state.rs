use super::{valid_node_id, MemoryConfig, MemoryError, MemorySyncResponse};
use crate::command_services::ssh::ProtectedFile;
use atomic_write_file::AtomicWriteFile;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

const SCHEMA_VERSION: u32 = 1;
const MAXIMUM_STATE_BYTES: u64 = 64 * 1024;
const MAXIMUM_CLOCK_SKEW_MINUTES: i64 = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemorySyncState {
    schema_version: u32,
    local_node_id: String,
    home_node_id: String,
    local_replication_cursor: u64,
    home_replication_cursor: u64,
    conflict_count: u64,
    last_successful_sync: String,
    sync_interval_minutes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemorySyncFreshness {
    NeverSynced,
    Fresh,
    Stale,
    Corrupt,
}

impl MemorySyncFreshness {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NeverSynced => "never_synced",
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Corrupt => "corrupt",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MemorySyncStatus {
    pub(crate) freshness: MemorySyncFreshness,
    pub(crate) local_node_id: Option<String>,
    pub(crate) home_node_id: Option<String>,
    pub(crate) local_replication_cursor: Option<u64>,
    pub(crate) home_replication_cursor: Option<u64>,
    pub(crate) conflict_count: Option<u64>,
    pub(crate) last_successful_sync: Option<String>,
}

impl MemorySyncStatus {
    fn empty(freshness: MemorySyncFreshness) -> Self {
        Self {
            freshness,
            local_node_id: None,
            home_node_id: None,
            local_replication_cursor: None,
            home_replication_cursor: None,
            conflict_count: None,
            last_successful_sync: None,
        }
    }

    fn from_state(state: MemorySyncState, freshness: MemorySyncFreshness) -> Self {
        Self {
            freshness,
            local_node_id: Some(state.local_node_id),
            home_node_id: Some(state.home_node_id),
            local_replication_cursor: Some(state.local_replication_cursor),
            home_replication_cursor: Some(state.home_replication_cursor),
            conflict_count: Some(state.conflict_count),
            last_successful_sync: Some(state.last_successful_sync),
        }
    }
}

fn validate_state(state: &MemorySyncState) -> Result<DateTime<Utc>, MemoryError> {
    if state.schema_version != SCHEMA_VERSION
        || !valid_node_id(&state.local_node_id)
        || !valid_node_id(&state.home_node_id)
        || state.local_node_id == state.home_node_id
        || !(5..=1440).contains(&state.sync_interval_minutes)
    {
        return Err(MemoryError::InvalidResponse);
    }
    DateTime::parse_from_rfc3339(&state.last_successful_sync)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| MemoryError::InvalidResponse)
}

pub(super) fn persist(path: &Path, state: &MemorySyncState) -> Result<(), MemoryError> {
    validate_state(state)?;
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(MemoryError::Task);
    }
    let bytes = serde_json::to_vec(state).map_err(|_| MemoryError::Task)?;
    if bytes.len() > MAXIMUM_STATE_BYTES as usize {
        return Err(MemoryError::Task);
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| MemoryError::Task)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| MemoryError::Task)?;
    }
    file.write_all(&bytes).map_err(|_| MemoryError::Task)?;
    file.commit().map_err(|_| MemoryError::Task)
}

pub(super) fn persist_successful_response(
    path: &Path,
    config: &MemoryConfig,
    response: &MemorySyncResponse,
) -> Result<(), MemoryError> {
    let pull = response.pull.as_ref().ok_or(MemoryError::InvalidResponse)?;
    let push = response.push.as_ref().ok_or(MemoryError::InvalidResponse)?;
    let last_success = response
        .last_success
        .as_deref()
        .ok_or(MemoryError::InvalidResponse)?;
    if response.status != "ok"
        || response.error.is_some()
        || pull.status != "ok"
        || pull.operation != "pull"
        || pull.source_node_id != config.home_node_id
        || pull.target_node_id != config.local_node_id
        || push.status != "ok"
        || push.operation != "push"
        || push.source_node_id != config.local_node_id
        || push.target_node_id != config.home_node_id
        || push.last_success != last_success
    {
        return Err(MemoryError::InvalidResponse);
    }
    let state = MemorySyncState {
        schema_version: SCHEMA_VERSION,
        local_node_id: config.local_node_id.clone(),
        home_node_id: config.home_node_id.clone(),
        local_replication_cursor: push.to_cursor,
        home_replication_cursor: pull.to_cursor,
        conflict_count: pull.target_conflict_count,
        last_successful_sync: last_success.to_string(),
        sync_interval_minutes: config.sync_interval_minutes,
    };
    persist(path, &state)
}

pub(crate) fn load_status(path: &Path, now: DateTime<Utc>) -> MemorySyncStatus {
    let bytes = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return MemorySyncStatus::empty(MemorySyncFreshness::NeverSynced);
        }
        Err(_) => return MemorySyncStatus::empty(MemorySyncFreshness::Corrupt),
        Ok(_) => {
            match ProtectedFile::open(path, MAXIMUM_STATE_BYTES).and_then(|file| file.read_all()) {
                Ok(bytes) => bytes,
                Err(_) => return MemorySyncStatus::empty(MemorySyncFreshness::Corrupt),
            }
        }
    };
    let state: MemorySyncState = match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(_) => return MemorySyncStatus::empty(MemorySyncFreshness::Corrupt),
    };
    let last_success = match validate_state(&state) {
        Ok(timestamp) => timestamp,
        Err(_) => return MemorySyncStatus::empty(MemorySyncFreshness::Corrupt),
    };
    let age = now.signed_duration_since(last_success);
    if age < Duration::minutes(-MAXIMUM_CLOCK_SKEW_MINUTES) {
        return MemorySyncStatus::empty(MemorySyncFreshness::Corrupt);
    }
    let maximum_age = Duration::minutes(i64::from(state.sync_interval_minutes) * 2);
    let freshness = if age <= maximum_age {
        MemorySyncFreshness::Fresh
    } else {
        MemorySyncFreshness::Stale
    };
    MemorySyncStatus::from_state(state, freshness)
}

#[cfg(all(test, unix))]
#[path = "memory_sync_state_tests.rs"]
mod tests;
