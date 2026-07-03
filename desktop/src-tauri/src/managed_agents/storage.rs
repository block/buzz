use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

use crate::app_state::KEYRING_SERVICE;
use crate::managed_agents::ManagedAgentRecord;
use crate::secret_store::{KeyringProbe, SecretStore};

/// Keyring key name for an agent's nsec, namespaced from the human identity
/// key (`"identity"`) which shares the service.
fn agent_keyring_name(pubkey: &str) -> String {
    format!("agent:{pubkey}")
}

/// The agent secret store. `None` when the build has no keyring backend, in
/// which case agent keys stay inline in the `0o600` JSON file. Uses
/// `SecretStore::shared` so identity and agent callers share one instance —
/// and therefore one in-memory cache and one mutex — preventing last-writer-wins
/// races on concurrent blob writes.
fn agent_secret_store() -> Option<&'static SecretStore> {
    if cfg!(feature = "system-keyring") {
        Some(SecretStore::shared(KEYRING_SERVICE))
    } else {
        None
    }
}

pub fn managed_agents_base_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("agents");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create agents dir: {error}"))?;
    Ok(dir)
}

fn managed_agents_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("managed-agents.json"))
}

fn managed_agents_logs_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("logs");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create logs dir: {error}"))?;
    Ok(dir)
}

pub fn managed_agent_log_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(format!("{pubkey}.log")))
}

/// The keyring operations the migration chokepoint needs. Abstracted so the
/// migrate-and-strip decision logic ([`migrate_inline_key`]) can be unit-tested
/// against a fake without touching the live OS keyring.
trait KeyStore {
    fn probe(&self, name: &str) -> KeyringProbe;
    /// Read a key. `Ok(None)` is "no such entry" (absent); `Err` is a backend
    /// failure (keyring unreachable) — the caller MUST NOT collapse the two.
    fn load(&self, name: &str) -> Result<Option<String>, String>;
    /// Write `value` and read it back to confirm before the caller strips the
    /// inline copy.
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String>;
}

impl KeyStore for SecretStore {
    fn probe(&self, name: &str) -> KeyringProbe {
        SecretStore::probe(self, name)
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        SecretStore::load(self, name)
    }
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
        self.store(name, value)?;
        match self.load(name)? {
            Some(stored) if stored == value => Ok(()),
            _ => Err("keyring read-back verify failed".to_string()),
        }
    }
}

/// Outcome of attempting to lift a record's inline key into the keyring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyMigration {
    /// Written to the keyring and read-back verified. Safe to drop the inline
    /// copy when serializing.
    Persisted,
    /// Could not persist (keyring unreachable, or write/verify failed). The key
    /// must stay inline (0o600 file fallback); do NOT drop it.
    KeptInline,
    /// The record carried no inline key, so there was nothing to migrate. Kept
    /// distinct from [`KeyMigration::Persisted`] so an empty key is never
    /// mistaken for "verified present in the keyring" — an empty key after a
    /// keyring outage means the secret is currently unavailable, not persisted.
    Nothing,
}

/// Attempt to lift one record's inline key into the keyring with read-back
/// verify. Pure decision logic — does NOT mutate the record, so the caller
/// chooses whether to strip the inline copy based on the returned outcome.
///
/// The single source of truth for the migrate-vs-keep decision, shared by the
/// load-time opportunistic re-migrate ([`hydrate_keys`]) and the save-time
/// chokepoint ([`persist_agent_keys`]). An empty key returns
/// [`KeyMigration::Nothing`] — never [`KeyMigration::Persisted`], so a record
/// left empty by a keyring outage is not mistaken for one verified present.
fn migrate_inline_key(store: &impl KeyStore, record: &ManagedAgentRecord) -> KeyMigration {
    if record.private_key_nsec.is_empty() {
        return KeyMigration::Nothing;
    }
    let name = agent_keyring_name(&record.pubkey);
    match store.probe(&name) {
        // Keyring down this boot: keep the key inline (file fallback), do NOT
        // migrate — re-importing later could resurrect a rotated key.
        KeyringProbe::Unreachable => KeyMigration::KeptInline,
        KeyringProbe::Present | KeyringProbe::ReachableButEmpty => {
            match store.write_and_verify(&name, &record.private_key_nsec) {
                Ok(()) => KeyMigration::Persisted,
                Err(e) => {
                    eprintln!(
                        "buzz-desktop: keyring write for agent {} failed ({e}), keeping inline",
                        record.pubkey
                    );
                    KeyMigration::KeptInline
                }
            }
        }
    }
}

/// Refuse to spawn an agent whose private key is unavailable. Returns
/// `Some(error)` when `private_key_nsec` is empty — after [`hydrate_keys`] an
/// empty key means a keyring outage or a genuinely absent secret, NOT a
/// deliberately keyless agent. Spawning anyway would inject an empty
/// `BUZZ_PRIVATE_KEY`/`NOSTR_PRIVATE_KEY`, launching with no identity. Callers
/// (the spawn path) must fail closed (Wes storage.rs:158).
pub(crate) fn spawn_key_refusal(record: &ManagedAgentRecord) -> Option<String> {
    record.private_key_nsec.is_empty().then(|| {
        format!(
            "agent {} has no private key available — the OS keyring may be unreachable. \
             Refusing to start without an identity; retry once the keyring is reachable.",
            record.pubkey
        )
    })
}

pub fn load_managed_agents(app: &AppHandle) -> Result<Vec<ManagedAgentRecord>, String> {
    let path = managed_agents_store_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read agent store: {error}"))?;
    let mut records: Vec<ManagedAgentRecord> = serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse agent store: {error}"))?;

    hydrate_keys(&mut records);
    Ok(records)
}

/// Fill in each record's in-memory `private_key_nsec` from the keyring, and
/// opportunistically re-migrate any key that is still inline.
///
/// - Empty key → fetch it from the keyring (the normal keyring-backed case).
/// - Non-empty key → the JSON carried it inline because the keyring was
///   unreachable at its last save. Re-migrate it now ([`migrate_inline_key`]):
///   if the keyring is reachable this boot, write-verify-strip so the next save
///   writes clean JSON and plaintext stops lingering on disk; if still
///   unreachable, leave it inline. This makes the strip deterministic on the
///   next reachable boot rather than waiting for a non-deterministic save.
fn hydrate_keys(records: &mut [ManagedAgentRecord]) {
    let Some(store) = agent_secret_store() else {
        return;
    };
    hydrate_keys_with(store, records);
}

/// Testable core of [`hydrate_keys`], generic over the [`KeyStore`] seam.
///
/// A keyring LOAD error (`Err`) is an OUTAGE — distinct from `Ok(None)`
/// (genuinely absent). On an outage the key is left empty and the record is
/// surfaced as unavailable rather than silently swallowed: callers must refuse
/// to spawn an agent whose key could not be read (see the empty-key bail in
/// `spawn_agent_child`). Empty here never means "fine" — it means "no usable
/// key this boot."
fn hydrate_keys_with(store: &impl KeyStore, records: &mut [ManagedAgentRecord]) {
    for record in records.iter_mut() {
        if record.private_key_nsec.is_empty() {
            match store.load(&agent_keyring_name(&record.pubkey)) {
                Ok(Some(nsec)) => record.private_key_nsec = nsec,
                Ok(None) => {
                    eprintln!(
                        "buzz-desktop: agent {} has no key in JSON or keyring",
                        record.pubkey
                    );
                }
                // Outage, NOT absence: the key may exist in the keyring but is
                // unreadable this boot. Leave it empty so the spawn path
                // refuses rather than launching with no identity.
                Err(e) => {
                    eprintln!(
                        "buzz-desktop: agent {} key unavailable — keyring read failed ({e}); \
                         agent will be refused until the keyring is reachable",
                        record.pubkey
                    );
                }
            }
        } else {
            // Inline residue from a prior keyring-unreachable save. Lift it
            // into the keyring now (side effect) but KEEP it in memory — the
            // returned record must carry the key for readers. The next save
            // then strips it from JSON. Outcome is intentionally ignored:
            // on failure the key simply stays inline until a later boot.
            let _ = migrate_inline_key(store, record);
        }
    }
}

pub fn save_managed_agents(app: &AppHandle, records: &[ManagedAgentRecord]) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sorted.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.pubkey.cmp(&right.pubkey))
    });

    // Persist each key to the keyring; on success blank the inline copy so it
    // is skipped from JSON (`skip_serializing_if = "String::is_empty"`). If the
    // keyring is unreachable, the key stays inline.
    persist_agent_keys(&mut sorted);

    let path = managed_agents_store_path(app)?;
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize agent store: {error}"))?;

    // `managed-agents.json` carries plaintext agent nsecs in the keyringless
    // fallback. Write it owner-only (`0o600`) unconditionally — harmless for the
    // keyring-backed case (it is the user's own agent store) and closes the
    // umask window a post-write `chmod` would leave open.
    atomic_write_json_restricted(&path, &payload)
}

/// Write each record's in-memory key to the keyring and blank the inline copy
/// on success. Keys that cannot be persisted (keyring unreachable) stay inline
/// in the JSON. Mutates `records` (a save-local clone) — the caller's in-memory
/// records keep their keys.
fn persist_agent_keys(records: &mut [ManagedAgentRecord]) {
    let Some(store) = agent_secret_store() else {
        // No keyring backend: keys stay inline.
        return;
    };
    persist_agent_keys_with(store, records);
}

/// Testable core of [`persist_agent_keys`], generic over the [`KeyStore`] seam.
fn persist_agent_keys_with(store: &impl KeyStore, records: &mut [ManagedAgentRecord]) {
    for record in records.iter_mut() {
        // Only a verified keyring entry lets us drop the inline copy. Both
        // other outcomes keep the key inline: `KeptInline` (keyring
        // unreachable) so it is not lost, and `Nothing` (empty key) because
        // there is no verified entry to claim. This is a save-local clone, so
        // callers keep their keys regardless.
        if migrate_inline_key(store, record) == KeyMigration::Persisted {
            record.private_key_nsec.clear();
        }
    }
}
/// Remove an agent's key from the keyring (best-effort). Called when an agent
/// is deleted so its secret does not linger in the OS store.
pub fn delete_agent_key(pubkey: &str) {
    if let Some(store) = agent_secret_store() {
        if let Err(e) = store.delete(&agent_keyring_name(pubkey)) {
            eprintln!("buzz-desktop: failed to delete agent {pubkey} key from keyring: {e}");
        }
    }
}

/// Atomic, symlink-preserving JSON write.
/// Resolves symlinks so the tmp+rename happens at the real target path,
/// preserving any symlink at `path`.
pub(crate) fn atomic_write_json(path: &Path, payload: &[u8]) -> Result<(), String> {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let tmp = resolved.with_extension("json.tmp");
    std::fs::write(&tmp, payload).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &resolved)
        .map_err(|e| format!("failed to rename {}: {e}", resolved.display()))
}

/// Atomic, symlink-preserving JSON write that creates the file `0o600` BEFORE
/// any bytes hit disk — closing the umask window the post-write `chmod` left
/// open. Used for `managed-agents.json`, which carries plaintext agent nsecs in
/// the keyringless fallback. Mirrors [`crate::app_state::save_key_file`].
///
/// Canonicalizes `path` first so the write lands at the real target, preserving
/// any symlink at `path` exactly like [`atomic_write_json`].
pub(crate) fn atomic_write_json_restricted(path: &Path, payload: &[u8]) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;

    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut file = AtomicWriteFile::open(&resolved)
        .map_err(|e| format!("open {} for atomic write: {e}", resolved.display()))?;

    // Set owner-only permissions before writing the secret bytes.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set {} permissions: {e}", resolved.display()))?;
    }

    file.write_all(payload)
        .map_err(|e| format!("write {}: {e}", resolved.display()))?;
    file.commit()
        .map_err(|e| format!("commit {}: {e}", resolved.display()))
}

/// Maximum log file size before rotation (10 MB).
const MAX_LOG_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// If `path` exceeds [`MAX_LOG_FILE_SIZE`], rotate it to `<path>.1`.
fn maybe_rotate_log(path: &Path) {
    let size = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if size <= MAX_LOG_FILE_SIZE {
        return;
    }
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    let _ = fs::rename(path, &rotated);
}

pub(crate) fn open_log_file(path: &Path) -> Result<File, String> {
    maybe_rotate_log(path);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open log file {}: {error}", path.display()))
}

pub(crate) fn append_log_marker(path: &Path, message: &str) -> Result<(), String> {
    let mut file = open_log_file(path)?;
    writeln!(file, "{message}").map_err(|error| format!("failed to write log marker: {error}"))
}

fn agent_pids_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("agent-pids");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create agent-pids dir: {error}"))?;
    Ok(dir)
}

/// Write a PID file for a spawned agent. The PID equals the PGID since we
/// spawn with `process_group(0)`.
pub fn write_agent_pid_file(app: &AppHandle, pubkey: &str, pid: u32) -> Result<(), String> {
    let path = agent_pids_dir(app)?.join(format!("{pubkey}.pid"));
    fs::write(&path, pid.to_string())
        .map_err(|error| format!("failed to write PID file {}: {error}", path.display()))
}

/// Remove the PID file for an agent (e.g. on normal stop).
pub fn remove_agent_pid_file(app: &AppHandle, pubkey: &str) {
    if let Ok(dir) = agent_pids_dir(app) {
        let _ = fs::remove_file(dir.join(format!("{pubkey}.pid")));
    }
}

/// Read all PID files from `agent-pids/`, returning `(pubkey, pid)` pairs.
pub fn read_all_agent_pid_files(app: &AppHandle) -> Vec<(String, u32)> {
    let Ok(dir) = agent_pids_dir(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let pubkey = name.strip_suffix(".pid")?;
            let pid: u32 = fs::read_to_string(entry.path()).ok()?.trim().parse().ok()?;
            Some((pubkey.to_string(), pid))
        })
        .collect()
}

pub fn read_log_tail(path: &Path, max_lines: usize) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }

    let mut file = File::open(path)
        .map_err(|error| format!("failed to read log file {}: {error}", path.display()))?;

    let file_len = file
        .seek(SeekFrom::End(0))
        .map_err(|error| format!("failed to seek log file: {error}"))?;

    if file_len == 0 {
        return Ok(String::new());
    }

    // Read backward in chunks to find enough newlines.
    const CHUNK_SIZE: u64 = 8 * 1024;
    let mut buf = Vec::new();
    let mut remaining = file_len;
    let mut newline_count: usize = 0;
    // We need max_lines + 1 newlines to delimit max_lines lines (the trailing
    // newline of the last line counts as one).
    let target_newlines = max_lines + 1;

    while remaining > 0 && newline_count < target_newlines {
        let chunk = remaining.min(CHUNK_SIZE);
        remaining -= chunk;
        file.seek(SeekFrom::Start(remaining))
            .map_err(|error| format!("failed to seek log file: {error}"))?;

        let mut tmp = vec![0u8; chunk as usize];
        file.read_exact(&mut tmp)
            .map_err(|error| format!("failed to read log chunk: {error}"))?;

        // Prepend this chunk so buf always has the tail of the file.
        tmp.append(&mut buf);
        buf = tmp;

        newline_count = bytecount_newlines(&buf);
    }

    // Strip ANSI escapes here (not in the harness) so the desktop log view
    // renders cleanly while terminals and other tools still get the colors
    // buzz-acp emits.
    let cleaned = strip_ansi_escapes::strip_str(String::from_utf8_lossy(&buf));
    let lines: Vec<&str> = cleaned.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].join("\n"))
}

fn bytecount_newlines(buf: &[u8]) -> usize {
    buf.iter().filter(|&&b| b == b'\n').count()
}

pub fn meaningful_agent_error_from_log(path: &Path) -> Option<String> {
    let tail = read_log_tail(path, 200).ok()?;
    tail.lines().rev().map(str::trim).find_map(|line| {
        if line.starts_with("Agent reported error:") {
            return Some(line.to_string());
        }
        if line.starts_with("llm auth:") {
            return Some(format!("Agent reported error: {line}"));
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::{
        agent_keyring_name, hydrate_keys_with, migrate_inline_key, persist_agent_keys_with,
        KeyMigration, KeyStore, KeyringProbe, ManagedAgentRecord,
    };

    /// In-memory [`KeyStore`] for testing the migrate decision without the OS
    /// keyring. `reachable=false` simulates a backend outage; `fail_verify`
    /// simulates a write whose read-back does not confirm.
    struct FakeKeyStore {
        reachable: bool,
        fail_verify: bool,
        stored: RefCell<HashMap<String, String>>,
        write_count: RefCell<usize>,
    }

    impl FakeKeyStore {
        fn reachable() -> Self {
            Self {
                reachable: true,
                fail_verify: false,
                stored: RefCell::new(HashMap::new()),
                write_count: RefCell::new(0),
            }
        }
        fn unreachable() -> Self {
            Self {
                reachable: false,
                fail_verify: false,
                stored: RefCell::new(HashMap::new()),
                write_count: RefCell::new(0),
            }
        }
        fn verify_fails() -> Self {
            Self {
                reachable: true,
                fail_verify: true,
                stored: RefCell::new(HashMap::new()),
                write_count: RefCell::new(0),
            }
        }
        /// Seed a key as already present in the keyring.
        fn with_key(self, name: &str, value: &str) -> Self {
            self.stored
                .borrow_mut()
                .insert(name.to_string(), value.to_string());
            self
        }
    }

    impl KeyStore for FakeKeyStore {
        fn probe(&self, _name: &str) -> KeyringProbe {
            if self.reachable {
                KeyringProbe::ReachableButEmpty
            } else {
                KeyringProbe::Unreachable
            }
        }
        fn load(&self, name: &str) -> Result<Option<String>, String> {
            // An unreachable backend errors on read (outage), distinct from a
            // reachable backend returning `Ok(None)` for an absent entry.
            if !self.reachable {
                return Err("keyring backend unreachable".to_string());
            }
            Ok(self.stored.borrow().get(name).cloned())
        }
        fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
            if self.fail_verify {
                return Err("read-back verify failed".to_string());
            }
            *self.write_count.borrow_mut() += 1;
            self.stored
                .borrow_mut()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }
    }

    fn record_with_key(nsec: &str) -> ManagedAgentRecord {
        record_with_pubkey_and_key("agent-pubkey", nsec)
    }

    fn record_with_pubkey_and_key(pubkey: &str, nsec: &str) -> ManagedAgentRecord {
        serde_json::from_str(&format!(
            r#"{{
                "pubkey": "{pubkey}",
                "name": "test-agent",
                "private_key_nsec": "{nsec}",
                "relay_url": "wss://localhost:3000",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }}"#
        ))
        .expect("sample record")
    }

    #[test]
    fn migrate_persists_and_signals_stripping_when_keyring_reachable() {
        // Item 2: an inline key (residue from a prior keyring-unreachable save)
        // is written to the keyring and verified when the backend is reachable,
        // so the next save can drop it from JSON.
        let store = FakeKeyStore::reachable();
        let record = record_with_key("nsec1realkey");

        let outcome = migrate_inline_key(&store, &record);

        assert_eq!(outcome, KeyMigration::Persisted);
        assert_eq!(
            store
                .stored
                .borrow()
                .get(&agent_keyring_name("agent-pubkey"))
                .map(String::as_str),
            Some("nsec1realkey")
        );
    }

    #[test]
    fn migrate_keeps_inline_when_keyring_unreachable() {
        // No-resurrection guard: a transient outage must NOT migrate; the key
        // stays inline (file fallback) so it is not lost.
        let store = FakeKeyStore::unreachable();
        let record = record_with_key("nsec1realkey");

        let outcome = migrate_inline_key(&store, &record);

        assert_eq!(outcome, KeyMigration::KeptInline);
        assert!(store.stored.borrow().is_empty());
    }

    #[test]
    fn migrate_keeps_inline_when_verify_fails() {
        // A write whose read-back does not confirm must keep the key inline —
        // never drop plaintext on an unverified write.
        let store = FakeKeyStore::verify_fails();
        let record = record_with_key("nsec1realkey");

        assert_eq!(
            migrate_inline_key(&store, &record),
            KeyMigration::KeptInline
        );
    }

    #[test]
    fn migrate_reports_nothing_for_empty_key() {
        // A record whose key already lives in the keyring (empty inline) has
        // nothing to migrate. It must NOT be reported as `Persisted` — an
        // empty key after a keyring outage means the secret is unavailable,
        // not verified present (Wes storage.rs:158).
        let store = FakeKeyStore::reachable();
        let record = record_with_key("");

        assert_eq!(migrate_inline_key(&store, &record), KeyMigration::Nothing);
        assert!(store.stored.borrow().is_empty());
    }

    #[test]
    fn hydrate_fills_key_from_keyring_when_reachable() {
        // The normal keyring-backed case: an empty inline key is filled from
        // the keyring on load.
        let store =
            FakeKeyStore::reachable().with_key(&agent_keyring_name("agent-pubkey"), "nsec1stored");
        let mut records = vec![record_with_key("")];

        hydrate_keys_with(&store, &mut records);

        assert_eq!(records[0].private_key_nsec, "nsec1stored");
    }

    #[test]
    fn hydrate_leaves_key_empty_on_keyring_outage() {
        // Outage edge (Wes storage.rs:158): when the keyring read ERRORS, the
        // key must be left empty — never silently treated as resolved — so the
        // spawn path refuses rather than launching the agent with no identity.
        let store = FakeKeyStore::unreachable();
        let mut records = vec![record_with_key("")];

        hydrate_keys_with(&store, &mut records);

        assert!(
            records[0].private_key_nsec.is_empty(),
            "an unreadable key must stay empty, not be fabricated"
        );
    }

    #[test]
    fn spawn_refused_when_private_key_empty() {
        // The spawn path MUST refuse a record left empty by an outage/absence
        // before injecting an empty BUZZ_PRIVATE_KEY / NOSTR_PRIVATE_KEY — never
        // launch an agent with no identity (Wes storage.rs:158).
        let record = record_with_key("");
        assert!(
            super::spawn_key_refusal(&record).is_some(),
            "an agent with no private key must be refused"
        );
    }

    #[test]
    fn spawn_allowed_when_private_key_present() {
        // A record carrying a key must not be blocked by the refusal guard.
        let record = record_with_key("nsec1realkey");
        assert!(super::spawn_key_refusal(&record).is_none());
    }

    #[test]
    fn persist_agent_keys_issues_zero_writes_when_inline_keys_already_cleared() {
        // This is the dominant prompt-storm scenario: after the first successful
        // persist all inline copies are cleared, so subsequent saves (e.g. a
        // model change) must issue zero keychain writes. `migrate_inline_key`
        // returns `Nothing` for empty-key records, and `persist_agent_keys_with`
        // must propagate that guarantee — write_count stays at 0.
        let store = FakeKeyStore::reachable();
        // Records whose inline key is already blank (key lives in the keyring).
        let mut records = vec![record_with_key(""), record_with_key("")];

        persist_agent_keys_with(&store, &mut records);

        assert_eq!(
            *store.write_count.borrow(),
            0,
            "a save with no inline keys must issue zero keychain writes"
        );
    }

    #[test]
    fn persist_agent_keys_writes_once_per_record_with_inline_key() {
        // A record carrying an inline key (e.g. first save, or keyring-outage
        // residue) must trigger exactly one write_and_verify per record — and
        // once persisted the inline copy is cleared so the next save is free.
        // Records use distinct pubkeys so each maps to a distinct keyring name,
        // verifying the "per record" behaviour rather than a single-key overwrite.
        let store = FakeKeyStore::reachable();
        let mut records = vec![
            record_with_pubkey_and_key("pubkey-agent-alpha", "nsec1key_a"),
            record_with_pubkey_and_key("pubkey-agent-beta", "nsec1key_b"),
        ];

        persist_agent_keys_with(&store, &mut records);

        assert_eq!(
            *store.write_count.borrow(),
            2,
            "each record with an inline key must trigger exactly one write"
        );
        // Verify the correct keyring name was used for each agent.
        assert_eq!(
            store
                .stored
                .borrow()
                .get(&agent_keyring_name("pubkey-agent-alpha"))
                .map(String::as_str),
            Some("nsec1key_a"),
        );
        assert_eq!(
            store
                .stored
                .borrow()
                .get(&agent_keyring_name("pubkey-agent-beta"))
                .map(String::as_str),
            Some("nsec1key_b"),
        );
        // After persist the inline copies are cleared — next save is zero-write.
        assert!(records[0].private_key_nsec.is_empty());
        assert!(records[1].private_key_nsec.is_empty());
    }

    fn write_log(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp log");
        file.write_all(content.as_bytes()).expect("write log");
        file
    }

    /// The keyringless fallback write must land `0o600` from the write itself —
    /// not a post-write `chmod` — so a crash in the umask window can never leave
    /// plaintext agent nsecs world-readable (Wes storage.rs:239, SECURITY.md:90).
    #[cfg(unix)]
    #[test]
    fn restricted_write_lands_owner_only_without_post_write_chmod() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("managed-agents.json");

        super::atomic_write_json_restricted(&path, br#"[{"private_key_nsec":"nsec1secret"}]"#)
            .expect("restricted write");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "secret-bearing write must be owner-only");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            r#"[{"private_key_nsec":"nsec1secret"}]"#
        );
    }

    #[test]
    fn meaningful_agent_error_from_log_promotes_wrapped_llm_auth() {
        let file = write_log("noise\nAgent reported error: llm auth: denied\n");
        assert_eq!(
            super::meaningful_agent_error_from_log(file.path()).as_deref(),
            Some("Agent reported error: llm auth: denied")
        );
    }

    #[test]
    fn meaningful_agent_error_from_log_promotes_unwrapped_llm_auth() {
        let file = write_log("noise\nllm auth: denied\n");
        assert_eq!(
            super::meaningful_agent_error_from_log(file.path()).as_deref(),
            Some("Agent reported error: llm auth: denied")
        );
    }

    #[test]
    fn meaningful_agent_error_from_log_does_not_promote_midline_auth_text() {
        let file = write_log("noise before llm auth: denied\n");
        assert!(super::meaningful_agent_error_from_log(file.path()).is_none());
    }

    #[test]
    fn strips_ansi_from_typical_tracing_line() {
        let input = "\x1b[2m2026-05-27T15:16:32\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mbuzz_acp\x1b[0m\x1b[2m:\x1b[0m starting";
        assert_eq!(
            strip_ansi_escapes::strip_str(input),
            "2026-05-27T15:16:32  INFO buzz_acp: starting"
        );
    }
}
