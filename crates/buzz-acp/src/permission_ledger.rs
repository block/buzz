//! Durable, host-owned lifecycle state for ACP permission requests.
//!
//! The relay observer is a best-effort presentation stream.  It cannot be the
//! source of truth for a request that can cause an adapter action.  This module
//! writes the lifecycle to the harness configuration directory before a
//! permission is shown to an owner, and every write is atomically replaced and
//! synced before the caller may advance the ACP protocol.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};

const LEDGER_VERSION: u8 = 1;
const MAX_RECORDS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRecord {
    pub key: String,
    pub channel_id: Option<String>,
    pub start_nonce: String,
    pub turn_id: String,
    pub session_id: String,
    pub request_id: serde_json::Value,
    pub action_digest: String,
    pub request: serde_json::Value,
    pub options: Vec<serde_json::Value>,
    pub expires_at: String,
    pub updated_at: String,
    pub state: PermissionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "reason")]
pub enum PermissionState {
    Pending,
    DecisionConsumed(String),
    DeliveryAttempted(String),
    Selected(String),
    Cancelled,
    Expired,
    Abandoned,
    DeliveryUnknown,
}

#[derive(Default, Deserialize, Serialize)]
struct LedgerFile {
    version: u8,
    records: BTreeMap<String, PermissionRecord>,
}

/// A process-shared ledger.  The enclosing mutex is deliberately held by the
/// caller only for a small atomic file transaction; no observer or relay work
/// runs while the ledger lock is held.
pub struct PermissionLedger {
    path: PathBuf,
    start_nonce: String,
    records: BTreeMap<String, PermissionRecord>,
    poisoned: bool,
    #[cfg(test)]
    fail_after_replace: bool,
}

static SHARED_LEDGER: OnceLock<Arc<Mutex<PermissionLedger>>> = OnceLock::new();
static SHARED_LEDGER_INIT: Mutex<()> = Mutex::new(());

struct PersistError {
    replaced: bool,
    message: String,
}

impl PermissionLedger {
    /// Return the one ledger shared by every ACP pool client in this harness
    /// generation. A second path or generation is rejected instead of opening
    /// another in-memory view of the same action authority.
    pub fn shared(path: PathBuf, start_nonce: String) -> Result<Arc<Mutex<Self>>, String> {
        // `OnceLock::set` alone does not serialize `open()`: two first callers
        // could otherwise both reconcile the on-disk file before one wins.
        let _initializing = SHARED_LEDGER_INIT
            .lock()
            .map_err(|_| "permission lifecycle initialization lock poisoned".to_string())?;
        if let Some(existing) = SHARED_LEDGER.get() {
            let ledger = existing
                .lock()
                .map_err(|_| "permission lifecycle ledger lock poisoned".to_string())?;
            if ledger.path != path || ledger.start_nonce != start_nonce {
                return Err(
                    "permission lifecycle ledger already belongs to another runtime generation"
                        .into(),
                );
            }
            return Ok(Arc::clone(existing));
        }
        let opened = Arc::new(Mutex::new(Self::open(path, start_nonce)?));
        SHARED_LEDGER
            .set(Arc::clone(&opened))
            .map_err(|_| "permission lifecycle initialization race".to_string())?;
        Ok(opened)
    }

    pub fn open(path: PathBuf, start_nonce: String) -> Result<Self, String> {
        if start_nonce.is_empty() {
            return Err("permission lifecycle ledger requires a managed-agent start nonce".into());
        }
        let mut ledger = Self {
            path,
            start_nonce,
            records: BTreeMap::new(),
            poisoned: false,
            #[cfg(test)]
            fail_after_replace: false,
        };
        if ledger.path.exists() {
            let bytes = fs::read(&ledger.path)
                .map_err(|error| format!("read permission lifecycle ledger: {error}"))?;
            let file: LedgerFile = serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse permission lifecycle ledger: {error}"))?;
            if file.version != LEDGER_VERSION {
                return Err(format!(
                    "unsupported permission lifecycle ledger version {}",
                    file.version
                ));
            }
            ledger.records = file.records;
        }
        // An ACP process cannot resume a stdout request after it has died. Do
        // not reconstruct a pending UI card that would imply otherwise. A
        // write attempt may have reached the adapter pipe before a crash, so it
        // is explicitly reported as delivery-unknown rather than completed.
        if ledger.records.values().any(|record| {
            record.start_nonce != ledger.start_nonce
                && matches!(
                    record.state,
                    PermissionState::Pending
                        | PermissionState::DecisionConsumed(_)
                        | PermissionState::DeliveryAttempted(_)
                )
        }) {
            for record in ledger.records.values_mut() {
                if record.start_nonce == ledger.start_nonce {
                    continue;
                }
                record.state = match &record.state {
                    PermissionState::Pending | PermissionState::DecisionConsumed(_) => {
                        PermissionState::Abandoned
                    }
                    PermissionState::DeliveryAttempted(_) => PermissionState::DeliveryUnknown,
                    state => state.clone(),
                };
            }
            ledger.persist().map_err(|error| error.message)?;
        }
        Ok(ledger)
    }

    pub fn record_pending(&mut self, record: PermissionRecord) -> Result<(), String> {
        self.ensure_healthy()?;
        if record.start_nonce != self.start_nonce {
            return Err("permission lifecycle record has the wrong harness generation".into());
        }
        if self.records.contains_key(&record.key) {
            return Err("permission lifecycle key already exists".into());
        }
        let previous = self.records.clone();
        self.records.insert(record.key.clone(), record);
        self.prune_terminal_history();
        if let Err(error) = self.persist() {
            self.handle_persist_error(previous, error)?;
        }
        Ok(())
    }

    pub fn transition_pending(&mut self, key: &str, state: PermissionState) -> Result<(), String> {
        self.ensure_healthy()?;
        let previous = self.records.clone();
        let record = self
            .records
            .get_mut(key)
            .ok_or_else(|| "permission lifecycle record is missing".to_string())?;
        if record.state != PermissionState::Pending {
            return Err("permission lifecycle record was already consumed".into());
        }
        record.state = state;
        record.updated_at = chrono::Utc::now().to_rfc3339();
        if let Err(error) = self.persist() {
            self.handle_persist_error(previous, error)?;
        }
        Ok(())
    }

    pub fn transition_consumed_to_delivery_attempt(&mut self, key: &str) -> Result<(), String> {
        self.ensure_healthy()?;
        let previous = self.records.clone();
        let record = self
            .records
            .get_mut(key)
            .ok_or_else(|| "permission lifecycle record is missing".to_string())?;
        let PermissionState::DecisionConsumed(option_id) = &record.state else {
            return Err("permission lifecycle record is not a consumed decision".into());
        };
        record.state = PermissionState::DeliveryAttempted(option_id.clone());
        record.updated_at = chrono::Utc::now().to_rfc3339();
        if let Err(error) = self.persist() {
            self.handle_persist_error(previous, error)?;
        }
        Ok(())
    }

    /// Record ambiguous delivery of a cancelled ACP response without reviving owner authority.
    pub fn transition_cancelled_to_delivery_unknown(&mut self, key: &str) -> Result<(), String> {
        self.ensure_healthy()?;
        let previous = self.records.clone();
        let record = self
            .records
            .get_mut(key)
            .ok_or_else(|| "permission lifecycle record is missing".to_string())?;
        if record.state != PermissionState::Cancelled {
            return Err("permission lifecycle record is not cancelled".into());
        }
        record.state = PermissionState::DeliveryUnknown;
        record.updated_at = chrono::Utc::now().to_rfc3339();
        if let Err(error) = self.persist() {
            self.handle_persist_error(previous, error)?;
        }
        Ok(())
    }

    pub fn transition_delivery_attempt_to_selected(&mut self, key: &str) -> Result<(), String> {
        self.ensure_healthy()?;
        let previous = self.records.clone();
        let record = self
            .records
            .get_mut(key)
            .ok_or_else(|| "permission lifecycle record is missing".to_string())?;
        let PermissionState::DeliveryAttempted(option_id) = &record.state else {
            return Err("permission lifecycle record is not a delivery attempt".into());
        };
        record.state = PermissionState::Selected(option_id.clone());
        record.updated_at = chrono::Utc::now().to_rfc3339();
        if let Err(error) = self.persist() {
            self.handle_persist_error(previous, error)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn get(&self, key: &str) -> Option<&PermissionRecord> {
        self.records.get(key)
    }

    pub fn start_nonce(&self) -> &str {
        &self.start_nonce
    }

    fn prune_terminal_history(&mut self) {
        while self.records.len() > MAX_RECORDS {
            let Some(key) = self.records.iter().find_map(|(key, record)| {
                (!matches!(
                    record.state,
                    PermissionState::Pending
                        | PermissionState::DecisionConsumed(_)
                        | PermissionState::DeliveryAttempted(_)
                ))
                .then(|| key.clone())
            }) else {
                break;
            };
            self.records.remove(&key);
        }
    }

    fn ensure_healthy(&self) -> Result<(), String> {
        if self.poisoned {
            Err(
                "permission lifecycle ledger needs authoritative reopen after ambiguous write"
                    .into(),
            )
        } else {
            Ok(())
        }
    }

    fn handle_persist_error(
        &mut self,
        previous: BTreeMap<String, PermissionRecord>,
        error: PersistError,
    ) -> Result<(), String> {
        if error.replaced {
            // The replacement reached disk but its directory durability is
            // unknown. Keep the new in-memory state, poison this authority,
            // and stop ACP advancement until a new generation reopens it.
            self.poisoned = true;
        } else {
            self.records = previous;
        }
        Err(error.message)
    }

    fn persist(&self) -> Result<(), PersistError> {
        let parent = self.path.parent().ok_or_else(|| PersistError {
            replaced: false,
            message: "permission lifecycle ledger path has no parent".into(),
        })?;
        fs::create_dir_all(parent).map_err(|error| PersistError {
            replaced: false,
            message: format!("create permission lifecycle directory: {error}"),
        })?;
        let body = serde_json::to_vec(&LedgerFile {
            version: LEDGER_VERSION,
            records: self.records.clone(),
        })
        .map_err(|error| PersistError {
            replaced: false,
            message: format!("serialize permission lifecycle ledger: {error}"),
        })?;
        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| PersistError {
                replaced: false,
                message: format!("open permission lifecycle temporary file: {error}"),
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| PersistError {
                    replaced: false,
                    message: format!("restrict permission lifecycle temporary file: {error}"),
                })?;
        }
        file.write_all(&body).map_err(|error| PersistError {
            replaced: false,
            message: format!("write permission lifecycle temporary file: {error}"),
        })?;
        file.sync_all().map_err(|error| PersistError {
            replaced: false,
            message: format!("sync permission lifecycle temporary file: {error}"),
        })?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(|error| PersistError {
            replaced: false,
            message: format!("replace permission lifecycle ledger: {error}"),
        })?;
        #[cfg(test)]
        if self.fail_after_replace {
            return Err(PersistError {
                replaced: true,
                message: "injected failure after permission lifecycle replacement".into(),
            });
        }
        sync_directory(parent).map_err(|message| PersistError {
            replaced: true,
            message,
        })?;
        Ok(())
    }
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync permission lifecycle directory: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn record_key(
    start_nonce: &str,
    turn_id: &str,
    session_id: &str,
    request_id: &serde_json::Value,
    action_digest: &str,
) -> String {
    let typed_id = serde_json::to_string(request_id).unwrap_or_default();
    format!("{start_nonce}:{turn_id}:{session_id}:{typed_id}:{action_digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str) -> PermissionRecord {
        PermissionRecord {
            key: key.into(),
            channel_id: Some("channel".into()),
            start_nonce: "generation-a".into(),
            turn_id: "turn".into(),
            session_id: "session".into(),
            request_id: serde_json::json!(7),
            action_digest: "digest".into(),
            request: serde_json::json!({"method": "session/request_permission"}),
            options: vec![serde_json::json!({"optionId": "opaque-allow"})],
            expires_at: "2030-01-01T00:00:00Z".into(),
            updated_at: "2029-01-01T00:00:00Z".into(),
            state: PermissionState::Pending,
        }
    }

    #[test]
    fn restart_reconciles_unfinished_request_to_abandoned() {
        let temp = std::env::temp_dir().join(format!("buzz-acp-ledger-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let path = temp.join("permission-lifecycle.json");
        let mut ledger = PermissionLedger::open(path.clone(), "generation-a".into()).unwrap();
        ledger.record_pending(record("record")).unwrap();
        drop(ledger);

        let restarted = PermissionLedger::open(path, "generation-b".into()).unwrap();
        assert_eq!(
            restarted.get("record").unwrap().state,
            PermissionState::Abandoned
        );
        if temp.exists() {
            fs::remove_dir_all(temp).unwrap();
        }
    }

    #[test]
    fn terminal_transition_is_durable_and_single_use() {
        let temp = std::env::temp_dir().join(format!("buzz-acp-ledger-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let path = temp.join("permission-lifecycle.json");
        let mut ledger = PermissionLedger::open(path.clone(), "generation-a".into()).unwrap();
        ledger.record_pending(record("record")).unwrap();
        ledger
            .transition_pending(
                "record",
                PermissionState::DecisionConsumed("opaque-allow".into()),
            )
            .unwrap();
        assert!(ledger
            .transition_pending("record", PermissionState::Cancelled)
            .is_err());
        ledger
            .transition_consumed_to_delivery_attempt("record")
            .unwrap();
        ledger
            .transition_delivery_attempt_to_selected("record")
            .unwrap();
        drop(ledger);

        let restarted = PermissionLedger::open(path, "generation-b".into()).unwrap();
        assert_eq!(
            restarted.get("record").unwrap().state,
            PermissionState::Selected("opaque-allow".into())
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn cancelled_response_delivery_unknown_is_durable_and_never_reopens() {
        let temp = std::env::temp_dir().join(format!("buzz-acp-ledger-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let path = temp.join("permission-lifecycle.json");
        let mut ledger = PermissionLedger::open(path.clone(), "generation-a".into()).unwrap();
        ledger.record_pending(record("record")).unwrap();
        assert!(ledger
            .transition_cancelled_to_delivery_unknown("record")
            .is_err());
        ledger
            .transition_pending("record", PermissionState::Cancelled)
            .unwrap();
        ledger
            .transition_cancelled_to_delivery_unknown("record")
            .unwrap();
        assert!(ledger
            .transition_cancelled_to_delivery_unknown("record")
            .is_err());
        assert!(ledger
            .transition_pending("record", PermissionState::DecisionConsumed("allow".into()))
            .is_err());
        drop(ledger);

        let restarted = PermissionLedger::open(path, "generation-b".into()).unwrap();
        assert_eq!(
            restarted.get("record").unwrap().state,
            PermissionState::DeliveryUnknown
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn restart_does_not_replay_consumed_or_uncertain_delivery() {
        let temp = std::env::temp_dir().join(format!("buzz-acp-ledger-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let consumed_path = temp.join("consumed.json");
        let mut consumed =
            PermissionLedger::open(consumed_path.clone(), "generation-a".into()).unwrap();
        consumed.record_pending(record("consumed")).unwrap();
        consumed
            .transition_pending(
                "consumed",
                PermissionState::DecisionConsumed("opaque-allow".into()),
            )
            .unwrap();
        drop(consumed);
        assert_eq!(
            PermissionLedger::open(consumed_path, "generation-b".into())
                .unwrap()
                .get("consumed")
                .unwrap()
                .state,
            PermissionState::Abandoned
        );

        let attempted_path = temp.join("attempted.json");
        let mut attempted =
            PermissionLedger::open(attempted_path.clone(), "generation-a".into()).unwrap();
        attempted.record_pending(record("attempted")).unwrap();
        attempted
            .transition_pending(
                "attempted",
                PermissionState::DecisionConsumed("opaque-allow".into()),
            )
            .unwrap();
        attempted
            .transition_consumed_to_delivery_attempt("attempted")
            .unwrap();
        drop(attempted);
        assert_eq!(
            PermissionLedger::open(attempted_path, "generation-b".into())
                .unwrap()
                .get("attempted")
                .unwrap()
                .state,
            PermissionState::DeliveryUnknown
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn shared_initialization_serializes_first_open() {
        let temp =
            std::env::temp_dir().join(format!("buzz-acp-ledger-shared-{}", uuid::Uuid::new_v4()));
        let path = temp.join("permission-lifecycle.json");
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut joins = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            joins.push(std::thread::spawn(move || {
                barrier.wait();
                PermissionLedger::shared(path, "shared-generation".into()).unwrap()
            }));
        }
        barrier.wait();
        let first = joins.remove(0).join().unwrap();
        let second = joins.remove(0).join().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        if temp.exists() {
            fs::remove_dir_all(temp).unwrap();
        }
    }

    #[test]
    fn pre_replace_rolls_back_but_post_replace_poison_stops_advancement() {
        let temp =
            std::env::temp_dir().join(format!("buzz-acp-ledger-failure-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp).unwrap();
        let mut pre_replace = PermissionLedger {
            path: temp.clone(),
            start_nonce: "generation-a".into(),
            records: BTreeMap::new(),
            poisoned: false,
            fail_after_replace: false,
        };
        assert!(pre_replace.record_pending(record("pre-replace")).is_err());
        assert!(pre_replace.get("pre-replace").is_none());

        let path = temp.join("post-replace.json");
        let mut post_replace = PermissionLedger::open(path.clone(), "generation-a".into()).unwrap();
        post_replace.fail_after_replace = true;
        assert!(post_replace.record_pending(record("post-replace")).is_err());
        assert!(post_replace.get("post-replace").is_some());
        assert!(post_replace
            .transition_pending("post-replace", PermissionState::Cancelled)
            .is_err());
        drop(post_replace);
        assert_eq!(
            PermissionLedger::open(path, "generation-b".into())
                .unwrap()
                .get("post-replace")
                .unwrap()
                .state,
            PermissionState::Abandoned
        );
        fs::remove_dir_all(temp).unwrap();
    }
}
