use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use chrono::{DateTime, Local};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::world_monitor::{NormalizedWorldMonitorEvidence, WorldMonitorRequest};

const DAILY_LIMIT: u8 = 25;
const CACHE_TTL_SECONDS: i64 = 15 * 60;
const MAX_CACHE_ENTRIES: usize = 64;
const MAX_STATE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsagePool {
    Brief,
    Direct,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub local_date: String,
    pub brief_used: u8,
    pub direct_used: u8,
}

#[derive(Clone, Debug)]
pub enum UsageAdmission {
    Cached(NormalizedWorldMonitorEvidence),
    Reserved {
        cache_key: String,
        snapshot: UsageSnapshot,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("World Monitor daily allowance is exhausted")]
    Exhausted,
    #[error("World Monitor usage state is unavailable")]
    State,
}

#[derive(Debug)]
pub struct WorldMonitorUsageLedger {
    state_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedState {
    version: u8,
    local_date: String,
    brief_used: u8,
    direct_used: u8,
    cache: BTreeMap<String, CacheEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheEntry {
    stored_at: DateTime<chrono::Utc>,
    evidence: NormalizedWorldMonitorEvidence,
}

impl WorldMonitorUsageLedger {
    pub fn new(state_path: PathBuf) -> Self {
        Self { state_path }
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub fn admit(
        &self,
        pool: UsagePool,
        request: &WorldMonitorRequest,
        now_local: DateTime<Local>,
    ) -> Result<UsageAdmission, UsageError> {
        self.with_locked_state(now_local, |state| {
            let cache_key = cache_key(request)?;
            if let Some(entry) = state.cache.get(&cache_key) {
                let age = now_local.with_timezone(&chrono::Utc) - entry.stored_at;
                if age.num_seconds() >= 0 && age.num_seconds() < CACHE_TTL_SECONDS {
                    return Ok(UsageAdmission::Cached(entry.evidence.clone()));
                }
            }
            state.cache.remove(&cache_key);
            let used = match pool {
                UsagePool::Brief => &mut state.brief_used,
                UsagePool::Direct => &mut state.direct_used,
            };
            if *used >= DAILY_LIMIT {
                return Err(UsageError::Exhausted);
            }
            *used += 1;
            Ok(UsageAdmission::Reserved {
                cache_key,
                snapshot: snapshot_from(state),
            })
        })
    }

    pub fn store_success(
        &self,
        cache_key: &str,
        evidence: &NormalizedWorldMonitorEvidence,
        now_local: DateTime<Local>,
    ) -> Result<(), UsageError> {
        if cache_key.len() != 64 || !cache_key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(UsageError::State);
        }
        self.with_locked_state(now_local, |state| {
            state.cache.insert(
                cache_key.to_string(),
                CacheEntry {
                    stored_at: now_local.with_timezone(&chrono::Utc),
                    evidence: evidence.clone(),
                },
            );
            while state.cache.len() > MAX_CACHE_ENTRIES {
                let Some(oldest) = state
                    .cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.stored_at)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                state.cache.remove(&oldest);
            }
            Ok(())
        })
    }

    pub fn snapshot(&self, now_local: DateTime<Local>) -> Result<UsageSnapshot, UsageError> {
        self.with_locked_state(now_local, |state| Ok(snapshot_from(state)))
    }

    fn with_locked_state<T>(
        &self,
        now_local: DateTime<Local>,
        operation: impl FnOnce(&mut PersistedState) -> Result<T, UsageError>,
    ) -> Result<T, UsageError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| UsageError::State)?;
        }
        let lock_path = self.state_path.with_extension("lock");
        let lock = restricted_open(&lock_path)?;
        lock.lock_exclusive().map_err(|_| UsageError::State)?;
        let result = (|| {
            let local_date = now_local.date_naive().to_string();
            let mut state = read_state(&self.state_path, &local_date)?;
            if state.local_date != local_date {
                state = new_state(&local_date);
            }
            let output = operation(&mut state)?;
            write_state(&self.state_path, &state)?;
            Ok(output)
        })();
        let _ = FileExt::unlock(&lock);
        result
    }
}

fn new_state(local_date: &str) -> PersistedState {
    PersistedState {
        version: 1,
        local_date: local_date.to_string(),
        brief_used: 0,
        direct_used: 0,
        cache: BTreeMap::new(),
    }
}

fn snapshot_from(state: &PersistedState) -> UsageSnapshot {
    UsageSnapshot {
        local_date: state.local_date.clone(),
        brief_used: state.brief_used,
        direct_used: state.direct_used,
    }
}

fn cache_key(request: &WorldMonitorRequest) -> Result<String, UsageError> {
    let arguments = serde_jcs::to_vec(&request.arguments).map_err(|_| UsageError::State)?;
    let mut digest = Sha256::new();
    digest.update(request.tool.as_str().as_bytes());
    digest.update(b":");
    digest.update(arguments);
    Ok(hex::encode(digest.finalize()))
}

fn restricted_open(path: &Path) -> Result<File, UsageError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| UsageError::State)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| UsageError::State)?;
    }
    Ok(file)
}

fn read_state(path: &Path, local_date: &str) -> Result<PersistedState, UsageError> {
    if !path.exists() {
        return Ok(new_state(local_date));
    }
    let mut file = File::open(path).map_err(|_| UsageError::State)?;
    let metadata = file.metadata().map_err(|_| UsageError::State)?;
    if metadata.len() > MAX_STATE_BYTES {
        return Err(UsageError::State);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| UsageError::State)?;
    let state: PersistedState = serde_json::from_slice(&bytes).map_err(|_| UsageError::State)?;
    if state.version != 1
        || state.brief_used > DAILY_LIMIT
        || state.direct_used > DAILY_LIMIT
        || state.cache.len() > MAX_CACHE_ENTRIES
    {
        return Err(UsageError::State);
    }
    Ok(state)
}

fn write_state(path: &Path, state: &PersistedState) -> Result<(), UsageError> {
    let payload = serde_json::to_vec(state).map_err(|_| UsageError::State)?;
    if payload.len() as u64 > MAX_STATE_BYTES {
        return Err(UsageError::State);
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| UsageError::State)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| UsageError::State)?;
    }
    file.write_all(&payload).map_err(|_| UsageError::State)?;
    file.commit().map_err(|_| UsageError::State)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use chrono::{Local, NaiveDate, TimeZone, Utc};
    use serde_json::json;
    use tempfile::tempdir;

    use super::{UsageAdmission, UsagePool, WorldMonitorUsageLedger};
    use crate::world_monitor::{WorldMonitorRequest, WorldMonitorTool};

    fn request() -> WorldMonitorRequest {
        WorldMonitorRequest::new(WorldMonitorTool::CountryRisk, json!({"country_code":"PH"}))
            .expect("request")
    }

    fn local_time(day: u32) -> chrono::DateTime<Local> {
        let date = NaiveDate::from_ymd_opt(2026, 7, day).expect("date");
        Local
            .from_local_datetime(&date.and_hms_opt(12, 0, 0).expect("time"))
            .single()
            .expect("unambiguous local time")
    }

    #[test]
    fn brief_and_direct_have_independent_25_call_limits() {
        let directory = tempdir().expect("tempdir");
        let ledger = WorldMonitorUsageLedger::new(directory.path().join("usage.json"));
        for pool in [UsagePool::Brief, UsagePool::Direct] {
            for _ in 0..25 {
                assert!(matches!(
                    ledger.admit(pool, &request(), local_time(28)),
                    Ok(UsageAdmission::Reserved { .. })
                ));
            }
            assert!(ledger.admit(pool, &request(), local_time(28)).is_err());
        }
    }

    #[test]
    fn cache_hit_within_15_minutes_spends_no_call_and_next_day_resets() {
        let directory = tempdir().expect("tempdir");
        let ledger = WorldMonitorUsageLedger::new(directory.path().join("usage.json"));
        let admission = ledger
            .admit(UsagePool::Brief, &request(), local_time(28))
            .expect("reserved");
        let UsageAdmission::Reserved { cache_key, .. } = admission else {
            panic!("expected reservation");
        };
        let evidence = crate::world_monitor::NormalizedWorldMonitorEvidence::new(
            request(),
            json!({"timestamp":"2026-07-28T01:00:00Z"}),
            Utc::now(),
        );
        ledger
            .store_success(&cache_key, &evidence, local_time(28))
            .expect("cache");
        assert!(matches!(
            ledger
                .admit(UsagePool::Brief, &request(), local_time(28))
                .expect("cached"),
            UsageAdmission::Cached(_)
        ));
        assert_eq!(
            ledger
                .snapshot(local_time(28))
                .expect("snapshot")
                .brief_used,
            1
        );
        assert_eq!(
            ledger
                .snapshot(local_time(29))
                .expect("next day")
                .brief_used,
            0
        );
    }

    #[test]
    fn concurrent_ledgers_never_admit_call_26() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("usage.json");
        let barrier = Arc::new(Barrier::new(30));
        let admitted = std::thread::scope(|scope| {
            let handles = (0..30)
                .map(|_| {
                    let path = path.clone();
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        let ledger = WorldMonitorUsageLedger::new(path);
                        barrier.wait();
                        ledger
                            .admit(UsagePool::Direct, &request(), local_time(28))
                            .is_ok()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("thread"))
                .filter(|admitted| *admitted)
                .count()
        });
        assert_eq!(admitted, 25);
    }

    #[test]
    fn state_file_never_contains_api_key_and_corruption_fails_closed() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("usage.json");
        let ledger = WorldMonitorUsageLedger::new(path.clone());
        ledger
            .admit(UsagePool::Brief, &request(), local_time(28))
            .expect("reservation");
        let state = std::fs::read_to_string(&path).expect("state");
        assert!(!state.contains("wm_live_"));
        std::fs::write(&path, b"{invalid").expect("corrupt");
        assert!(ledger
            .admit(UsagePool::Brief, &request(), local_time(28))
            .is_err());
    }
}
