//! Durable channel → ACP session map, so a harness restart resumes instead of
//! forgetting.
//!
//! `AgentPool` keeps `SessionState.sessions` in memory. That is correct while
//! the process lives, but every restart — a desktop relaunch, a config-change
//! restart, or a hosted harness being moved between nodes on a deploy — emptied
//! it, and the next mention in every channel got a fresh `session/new`. For an
//! agent whose session *is* its workspace (a sandbox per session, as with the
//! `fountain acp` gateway) that discards the channel's memory and files each
//! time.
//!
//! # What a receipt records, and why
//!
//! A receipt is deliberately *not* a full configuration fingerprint. It carries
//! exactly what the harness applies at `session/new` and **cannot** apply at
//! `session/load`:
//!
//! - `model` — [`create_session_and_apply_model`] resolves the switch method
//!   from the `session/new` *response* (`configOptions` / `models`), which
//!   `session/load` does not return. A resumed session therefore keeps whatever
//!   model it was created under, so resuming across a model change would
//!   silently ignore the change.
//! - `permission_mode` — same shape: `agent_supports_mode` reads the
//!   `session/new` response before `apply_permission_mode` fires.
//!
//! Everything else the harness sends (system prompt, team instructions, MCP
//! server list, agent core memory) is delivered per *turn* or re-sent on the
//! resumed session, so a change to it does not require abandoning the
//! conversation — and abandoning it would defeat the feature, since a prompt
//! tweak would wipe every channel's history on the next restart.
//!
//! [`create_session_and_apply_model`]: crate::pool
//!
//! # Scope
//!
//! The file records the (relay, agent pubkey) pair it was written for and is
//! treated as empty when that does not match. One agent identity can serve more
//! than one community; without this, a shared state directory would let one
//! community select another's remembered session, and `session/load` carries
//! only an opaque id for the agent to reject.
//!
//! # Concurrency and durability
//!
//! Every write re-reads the file under the lock, merges the single operation
//! onto what is actually on disk, and renames a fresh temp file over it —
//! so two harnesses that overlap during a rolling deploy cannot lose each
//! other's unrelated channels to a stale in-memory snapshot. The directory is
//! created `0700` and the file `0600`, and the temp file is opened
//! `create_new` under a unique name, because the documented deployment
//! contract points `BUZZ_ACP_STATE_DIR` at operator-chosen container storage.
//!
//! Every failure is logged and swallowed — the store is an optimisation on top
//! of the in-memory map, never a reason to refuse a turn.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// On-disk format version. Bumped when a receipt gains or loses a field that
/// older harnesses would misread; an unrecognised version is treated as empty
/// rather than guessed at.
const STORE_VERSION: u32 = 1;

/// Refuse to parse anything larger than this. The file holds one short record
/// per channel; a state directory that has become something else should not
/// turn into an allocation.
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;

/// The (relay, agent) pair a store file belongs to.
///
/// A file whose scope does not match the running harness is not ours to read:
/// the same agent key can serve several communities, and channel UUIDs from one
/// must never select a session from another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreScope {
    /// Relay/community URL as configured.
    pub relay: String,
    /// Agent pubkey, hex.
    pub agent_pubkey: String,
}

/// What the harness remembers about one channel's session.
///
/// See the module docs for why `model` and `permission_mode` are here and
/// nothing else is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub session_id: String,
    /// `desired_model` at creation; `None` means the agent's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Permission mode wire string at creation (`"default"`, `"bypassPermissions"`, …).
    pub permission_mode: String,
}

impl SessionRecord {
    /// Whether a session created under this receipt can still serve `desired`.
    ///
    /// Both fields are applied only at `session/new`, so a mismatch means a
    /// resumed session would silently run the *old* configuration.
    pub fn matches(&self, model: Option<&str>, permission_mode: &str) -> bool {
        self.model.as_deref() == model && self.permission_mode == permission_mode
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    scope: StoreScope,
    sessions: BTreeMap<Uuid, SessionRecord>,
}

#[derive(Debug)]
pub struct SessionStore {
    /// `None` disables persistence: `get` is always empty, `put`/`remove` no-op.
    path: Option<PathBuf>,
    scope: StoreScope,
    /// Read cache. Writes refresh it from disk, so it never diverges from the
    /// file for longer than one operation.
    cache: Mutex<BTreeMap<Uuid, SessionRecord>>,
}

/// Distinguishes temp files written by concurrent flushes in one process.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl SessionStore {
    /// A store that remembers nothing across restarts.
    pub fn disabled() -> Self {
        Self {
            path: None,
            scope: StoreScope {
                relay: String::new(),
                agent_pubkey: String::new(),
            },
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Open (or create on first write) the store at `path`, loading whatever is
    /// there for `scope`. An unreadable, malformed, oversized, wrong-version,
    /// or differently-scoped file is treated as empty and logged.
    pub fn open(path: impl Into<PathBuf>, scope: StoreScope) -> Self {
        let path = path.into();
        let sessions = read_scoped(&path, &scope);
        tracing::info!(
            target: "session_store",
            "session store {}: {} channel session(s) remembered for relay {}",
            path.display(),
            sessions.len(),
            scope.relay,
        );
        Self {
            path: Some(path),
            scope,
            cache: Mutex::new(sessions),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// The session last recorded for `channel_id`, if any.
    pub fn get(&self, channel_id: Uuid) -> Option<SessionRecord> {
        self.cache.lock().ok()?.get(&channel_id).cloned()
    }

    /// Remember `record` for `channel_id` and flush.
    pub fn put(&self, channel_id: Uuid, record: SessionRecord) {
        self.update(|sessions| {
            sessions.insert(channel_id, record);
            true
        });
    }

    /// Forget `channel_id` (the session was invalidated, or a resume of it
    /// failed definitively) and flush.
    pub fn remove(&self, channel_id: Uuid) {
        self.update(|sessions| sessions.remove(&channel_id).is_some());
    }

    /// Apply one operation to what is *currently on disk*, then write it back.
    ///
    /// Re-reading inside the lock is what makes two overlapping harness
    /// processes safe for each other's unrelated channels: neither can restore
    /// a stale whole-map snapshot over the other's writes. The last writer for
    /// any single channel still wins, which is the correct resolution — that is
    /// the session most recently created for it.
    fn update(&self, op: impl FnOnce(&mut BTreeMap<Uuid, SessionRecord>) -> bool) {
        let Some(path) = &self.path else { return };
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        let mut sessions = read_scoped(path, &self.scope);
        if !op(&mut sessions) {
            // Nothing changed (e.g. removing a channel that is already gone);
            // still adopt what disk says so the read cache is not stale.
            *cache = sessions;
            return;
        }
        if let Err(e) = write_atomic(path, &self.scope, &sessions) {
            tracing::warn!(
                target: "session_store",
                "cannot write session store {}: {e}",
                path.display()
            );
        }
        *cache = sessions;
    }
}

/// Read `path`, returning its sessions only if it parses and is ours.
fn read_scoped(path: &Path, scope: &StoreScope) -> BTreeMap<Uuid, SessionRecord> {
    let bytes = match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_STORE_BYTES => {
            tracing::warn!(
                target: "session_store",
                "ignoring session store {} — {} bytes exceeds the {MAX_STORE_BYTES} byte cap",
                path.display(),
                meta.len()
            );
            return BTreeMap::new();
        }
        Ok(_) => match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(
                    target: "session_store",
                    "cannot read session store {}: {e}",
                    path.display()
                );
                return BTreeMap::new();
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(e) => {
            tracing::warn!(
                target: "session_store",
                "cannot stat session store {}: {e}",
                path.display()
            );
            return BTreeMap::new();
        }
    };

    let file: StoreFile = match serde_json::from_slice(&bytes) {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!(
                target: "session_store",
                "ignoring malformed session store {}: {e}",
                path.display()
            );
            return BTreeMap::new();
        }
    };
    if file.version != STORE_VERSION {
        tracing::warn!(
            target: "session_store",
            "ignoring session store {} written by version {} (this harness writes {STORE_VERSION})",
            path.display(),
            file.version
        );
        return BTreeMap::new();
    }
    if file.scope != *scope {
        tracing::warn!(
            target: "session_store",
            "ignoring session store {} — it belongs to relay {} / agent {}, not {} / {}",
            path.display(),
            file.scope.relay,
            file.scope.agent_pubkey,
            scope.relay,
            scope.agent_pubkey,
        );
        return BTreeMap::new();
    }
    file.sessions
}

/// Write the whole store through a uniquely named `0600` temp file and rename
/// it into place, fsyncing the file and its directory.
///
/// `create_new` on a unique name is what makes the temp path unusable as a
/// symlink-preplacement target, which matters because the deployment contract
/// invites operators to point `BUZZ_ACP_STATE_DIR` at shared container storage.
fn write_atomic(
    path: &Path,
    scope: &StoreScope,
    sessions: &BTreeMap<Uuid, SessionRecord>,
) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    create_dir_private(dir)?;

    let unique = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        "{}.{}.{unique}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("sessions.json"),
        std::process::id(),
    ));

    let bytes = serde_json::to_vec_pretty(&StoreFile {
        version: STORE_VERSION,
        scope: scope.clone(),
        sessions: sessions.clone(),
    })
    .map_err(std::io::Error::other)?;

    let result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        // Durability of the rename itself, not just the bytes.
        if let Ok(dir) = std::fs::File::open(dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `create_dir_all`, owner-only on the components it creates.
fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "buzz-acp-session-store-{}-{name}-{}.json",
            std::process::id(),
            Uuid::new_v4(),
        ))
    }

    fn scope() -> StoreScope {
        StoreScope {
            relay: "wss://relay.example".into(),
            agent_pubkey: "a".repeat(64),
        }
    }

    fn record(session_id: &str) -> SessionRecord {
        SessionRecord {
            session_id: session_id.into(),
            model: None,
            permission_mode: "default".into(),
        }
    }

    #[test]
    fn disabled_store_remembers_nothing() {
        let store = SessionStore::disabled();
        let ch = Uuid::new_v4();
        store.put(ch, record("sess"));
        assert_eq!(store.get(ch), None);
        assert!(!store.is_enabled());
    }

    #[test]
    fn put_survives_reopen_and_remove_forgets() {
        let path = tmp_path("roundtrip");
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();

        let store = SessionStore::open(&path, scope());
        store.put(ch_a, record("sess-a"));
        store.put(ch_b, record("sess-b"));
        drop(store);

        // "The process restarted."
        let store = SessionStore::open(&path, scope());
        assert_eq!(store.get(ch_a), Some(record("sess-a")));
        assert_eq!(store.get(ch_b), Some(record("sess-b")));

        store.remove(ch_a);
        drop(store);
        let store = SessionStore::open(&path, scope());
        assert_eq!(store.get(ch_a), None);
        assert_eq!(store.get(ch_b), Some(record("sess-b")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_file_is_treated_as_empty() {
        let path = tmp_path("malformed");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = SessionStore::open(&path, scope());
        assert_eq!(store.get(Uuid::new_v4()), None);
        // And it is writable afterwards.
        let ch = Uuid::new_v4();
        store.put(ch, record("sess"));
        assert_eq!(
            SessionStore::open(&path, scope()).get(ch),
            Some(record("sess"))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_store_written_for_another_community_is_not_read() {
        let path = tmp_path("scope");
        let ch = Uuid::new_v4();

        let theirs = SessionStore::open(
            &path,
            StoreScope {
                relay: "wss://other.example".into(),
                agent_pubkey: "a".repeat(64),
            },
        );
        theirs.put(ch, record("their-session"));
        drop(theirs);

        // Same agent key, same channel UUID, same file — different community.
        let ours = SessionStore::open(&path, scope());
        assert_eq!(
            ours.get(ch),
            None,
            "one community must never resume another's session"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_store_from_a_future_version_is_not_read() {
        let path = tmp_path("version");
        let ch = Uuid::new_v4();
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": STORE_VERSION + 1,
                "scope": scope(),
                "sessions": { ch.to_string(): record("sess") },
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(SessionStore::open(&path, scope()).get(ch), None);
        let _ = std::fs::remove_file(&path);
    }

    /// The lost update the review reproduced: two stores open the same file,
    /// each writes a different channel, and the second must not restore its
    /// stale snapshot over the first.
    #[test]
    fn overlapping_stores_keep_each_others_channels() {
        let path = tmp_path("overlap");
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();

        let a = SessionStore::open(&path, scope());
        let b = SessionStore::open(&path, scope());
        a.put(ch_a, record("sess-a"));
        b.put(ch_b, record("sess-b"));

        let reopened = SessionStore::open(&path, scope());
        assert_eq!(reopened.get(ch_a), Some(record("sess-a")));
        assert_eq!(reopened.get(ch_b), Some(record("sess-b")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn compatibility_is_model_and_permission_mode() {
        let rec = SessionRecord {
            session_id: "s".into(),
            model: Some("gpt-5".into()),
            permission_mode: "bypassPermissions".into(),
        };
        assert!(rec.matches(Some("gpt-5"), "bypassPermissions"));
        assert!(!rec.matches(Some("gpt-4"), "bypassPermissions"));
        assert!(!rec.matches(None, "bypassPermissions"));
        assert!(!rec.matches(Some("gpt-5"), "default"));
    }

    #[cfg(unix)]
    #[test]
    fn store_file_and_directory_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("buzz-acp-store-perms-{}", Uuid::new_v4()));
        let path = dir.join("sessions.json");
        let store = SessionStore::open(&path, scope());
        store.put(Uuid::new_v4(), record("sess"));

        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "receipt must not be world-readable");
        assert_eq!(dir_mode, 0o700, "state dir must not be world-readable");

        // No temp file survives a successful flush.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leaked temp files: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
