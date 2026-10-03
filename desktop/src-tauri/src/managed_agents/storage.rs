use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

use crate::app_state::{keyring_service, AppState};
use crate::managed_agents::{
    ManagedAgentRecord, ManagedAgentRuntimeKey, ManagedAgentRuntimeReceipt,
};
use crate::secret_store::{KeyringProbe, SecretStore};

/// Filename prefix of the per-community agent store shards. Each community
/// (canonical relay URL) gets `managed-agents-community.<key>.json` next to
/// the legacy global store, where `<key>` comes from [`community_key_of`]. The prefix is deliberately distinct from
/// `managed-agents.json` so hand-made backup copies (e.g.
/// `managed-agents.my-community.json`) are never mistaken for a shard.
const COMMUNITY_SHARD_PREFIX: &str = "managed-agents-community.";
const COMMUNITY_SHARD_SUFFIX: &str = ".json";

/// Community identity of a relay URL, encoded as a filename-safe shard key.
///
/// The identity is the repository's canonical relay URL
/// (`buzz_core::relay::normalize_relay_url`: scheme, lowercased host, loopback
/// folded to `127.0.0.1`, non-default port, path and query all preserved), so
/// `ws://localhost:3000` and `ws://localhost:3030` are two communities, while
/// `ws://localhost:3000/` and `ws://127.0.0.1:3000` are one. The key is
/// `<host>-<port>.<hash>`: a readable host/port label (characters outside
/// `[a-z0-9.-]` folded to `-`) plus the first 16 hex chars of the SHA-256 of
/// the canonical URL, which keeps keys distinct for relays that differ only
/// by scheme, path or query.
///
/// Returns `None` when the URL is empty or not a valid relay URL. A `None`
/// means the record cannot be scoped to a community and must stay in the
/// legacy global store (fail-open to the pre-#7184 behavior) — never silently
/// dropped.
fn community_key_of(relay_url: &str) -> Option<String> {
    use sha2::{Digest, Sha256};

    let trimmed = relay_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let canonical = buzz_core_pkg::relay::normalize_relay_url(trimmed).ok()?;
    let authority = canonical
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(&canonical)
        .split(['/', '?'])
        .next()
        .unwrap_or_default();
    let mut label = String::with_capacity(authority.len());
    for c in authority.chars() {
        let c = c.to_ascii_lowercase();
        let keep = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.';
        let next = if keep { c } else { '-' };
        // Never emit ".." into a filename.
        if next == '.' && label.ends_with('.') {
            continue;
        }
        label.push(next);
    }
    let label = label.trim_matches(|c| c == '.' || c == '-');
    let digest = hex::encode(Sha256::digest(canonical.as_bytes()));
    let hash = &digest[..16];
    if label.is_empty() {
        Some(hash.to_string())
    } else {
        Some(format!("{label}.{hash}"))
    }
}

/// Shard filename for a community key (from [`community_key_of`], already
/// filename-safe).
fn community_shard_file_name(key: &str) -> String {
    format!("{COMMUNITY_SHARD_PREFIX}{key}{COMMUNITY_SHARD_SUFFIX}")
}

/// The community key encoded in a shard path, or `None` for any other file.
fn community_key_of_shard_path(path: &Path) -> Option<String> {
    path.file_name()?
        .to_str()?
        .strip_prefix(COMMUNITY_SHARD_PREFIX)?
        .strip_suffix(COMMUNITY_SHARD_SUFFIX)
        .map(str::to_string)
}

/// Enumerate existing community shard files under `base_dir`, sorted by name
/// for deterministic load order. Only files whose middle segment parses as a
/// sanitized relay host are returned; anything else (backups, hand copies,
/// `.invalid` preserves) is ignored.
pub(crate) fn community_shard_paths(base_dir: &Path) -> Vec<PathBuf> {
    let mut shards = Vec::new();
    let Ok(entries) = fs::read_dir(base_dir) else {
        return shards;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(host) = name
            .strip_prefix(COMMUNITY_SHARD_PREFIX)
            .and_then(|rest| rest.strip_suffix(COMMUNITY_SHARD_SUFFIX))
        else {
            continue;
        };
        let safe = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
        if safe {
            shards.push(entry.path());
        }
    }
    shards.sort();
    shards
}

/// Partition keyed instances by destination store: records carrying a
/// valid relay URL go to that community's shard (keyed by
/// [`community_key_of`]); everything else
/// (unpinned legacy records, and by construction all key-less definitions,
/// which callers keep separate anyway) stays in the legacy global store.
fn partition_by_community(
    instances: Vec<ManagedAgentRecord>,
) -> (
    Vec<ManagedAgentRecord>,
    BTreeMap<String, Vec<ManagedAgentRecord>>,
) {
    let mut legacy = Vec::new();
    let mut shards: BTreeMap<String, Vec<ManagedAgentRecord>> = BTreeMap::new();
    for record in instances {
        match community_key_of(&record.relay_url) {
            Some(key) => shards.entry(key).or_default().push(record),
            None => legacy.push(record),
        }
    }
    (legacy, shards)
}

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
        Some(SecretStore::shared(keyring_service()))
    } else {
        None
    }
}

pub fn managed_agents_base_dir<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("agents");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create agents dir: {error}"))?;
    Ok(dir)
}

pub(crate) fn managed_agents_store_path<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("managed-agents.json"))
}

fn managed_agents_logs_dir<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("logs");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create logs dir: {error}"))?;
    Ok(dir)
}

/// Install-log path for `runtime_id`, alongside the agent logs.
pub fn install_log_path(app: &AppHandle, runtime_id: &str) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(install_log_filename(runtime_id)?))
}

/// Filename for a runtime's install log, or an error for an id that must not
/// become one.
///
/// The id is validated rather than trusted: ids reach this from user-defined
/// custom harnesses as well as the catalog, and a `../` or a separator in one
/// would place the log outside the logs directory. Rejecting beats sanitizing —
/// a rejected id means no log, while a rewritten one could collide with another
/// runtime's.
fn install_log_filename(runtime_id: &str) -> Result<String, String> {
    if runtime_id.is_empty() || !runtime_id.chars().all(is_safe_id_char) {
        return Err(format!(
            "unsafe runtime id for a log filename: {runtime_id}"
        ));
    }
    Ok(format!("install-{runtime_id}.log"))
}

/// Characters allowed in a runtime id used as a filename. Excludes `/`, `\`,
/// `:` and `.`, so no id can traverse or escape the logs directory.
fn is_safe_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

pub fn managed_agent_log_path<R: tauri::Runtime>(
    app: &AppHandle<R>,
    pubkey: &str,
) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(format!("{pubkey}.log")))
}

/// Pair-scoped log path for a managed runtime. The relay URL never appears in
/// the filename; the suffix is a hash of the canonical URL.
pub fn managed_agent_runtime_log_path<R: tauri::Runtime>(
    app: &AppHandle<R>,
    key: &ManagedAgentRuntimeKey,
) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(format!("{}.log", key.runtime_id())))
}

/// Log path to surface for an agent whose runtime is not tracked in memory:
/// the most recently written of its pair-scoped logs, falling back to the
/// legacy single-runtime path when the agent has not run since harnesses
/// became per (agent, relay) pair.
pub fn latest_managed_agent_log_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    match newest_agent_log_in_dir(&managed_agents_logs_dir(app)?, pubkey) {
        Some(path) => Ok(path),
        None => managed_agent_log_path(app, pubkey),
    }
}

/// Newest log in `dir` belonging to `pubkey` — either a pair-scoped
/// `{pubkey}__{relay_hash}.log` or the legacy `{pubkey}.log`. Ties break
/// toward the higher filename so the choice is deterministic.
fn newest_agent_log_in_dir(dir: &Path, pubkey: &str) -> Option<PathBuf> {
    let legacy_name = format!("{pubkey}.log");
    let pair_prefix = format!("{pubkey}__");
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let matches = name.to_str().is_some_and(|name| {
                name == legacy_name || (name.starts_with(&pair_prefix) && name.ends_with(".log"))
            });
            if !matches {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, name, entry.path()))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, _, path)| path)
}

/// The keyring operations the migration chokepoint needs. Abstracted so the
/// migrate-and-strip decision logic ([`migrate_inline_key`]) can be unit-tested
/// against a fake without touching the live OS keyring.
trait KeyStore {
    fn probe(&self, name: &str) -> KeyringProbe;
    /// Read a key. `Ok(None)` is "no such entry" (absent); `Err` is a backend
    /// failure (keyring unreachable) — the caller MUST NOT collapse the two.
    fn load(&self, name: &str) -> Result<Option<String>, String>;
    /// Read the entire blob as a map without any side effects.
    /// `Ok(None)` when no blob exists yet; `Err` only on backend failure.
    /// Callers must not call `migrate_legacy_key` — this is a read-only view.
    fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String>;
    /// Write `value` and read it back to confirm before the caller strips the
    /// inline copy.
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String>;
    /// Insert all entries from `entries` in a single blob mutation.
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String>;
}

impl KeyStore for SecretStore {
    fn probe(&self, name: &str) -> KeyringProbe {
        SecretStore::probe(self, name)
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        SecretStore::load(self, name)
    }
    fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        SecretStore::load_all_readonly(self)
    }
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
        self.store(name, value)?;
        match self.load(name)? {
            Some(stored) if stored == value => Ok(()),
            _ => Err("keyring read-back verify failed".to_string()),
        }
    }
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        SecretStore::store_all(self, entries)
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

/// Read the raw unified store — keyed instances AND key-less definitions —
/// with fail-loud parse handling. Internal seam; public readers filter.
///
/// Reads the legacy global store PLUS every community shard found on disk and
/// concatenates their records (legacy first, then shards in filename order).
/// Fail-loud contract per file: a malformed shard is preserved as
/// `<shard>.invalid` and its parse error propagates exactly like the global
/// store's (a later save would rewrite it wholesale, so silent swallowing
/// would destroy a malformed hand edit).
fn read_store_file(path: &Path) -> Result<Vec<ManagedAgentRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read agent store {}: {error}", path.display()))?;
    serde_json::from_str(&content).map_err(|error| {
        backup_invalid_store(path);
        format!(
            "failed to parse agent store {} (preserved as .invalid): {error}",
            path.display()
        )
    })
}

fn load_agent_store<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ManagedAgentRecord>, String> {
    load_agent_store_in_dir(&managed_agents_base_dir(app)?)
}

/// Path-level core of [`load_agent_store`]: legacy store plus every shard,
/// with each keyed instance returned exactly once.
///
/// A pubkey can legitimately sit in two files for a moment — a save writes a
/// record's new home before it clears the old one (see
/// [`write_agent_store_in_dir`]), and stores written by earlier builds of
/// this change carried every scoped record in both the legacy store and its
/// shard. The copy kept is the one in the file the record's own `relay_url`
/// maps to (its canonical home); otherwise the last copy read wins (shards
/// are read after the legacy store, in filename order). The next save then
/// rewrites the files without the extra copy.
fn load_agent_store_in_dir(base_dir: &Path) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut sourced: Vec<(Option<String>, ManagedAgentRecord)> =
        read_store_file(&legacy_store_path(base_dir))?
            .into_iter()
            .map(|record| (None, record))
            .collect();
    for shard in community_shard_paths(base_dir) {
        let key = community_key_of_shard_path(&shard);
        sourced.extend(
            read_store_file(&shard)?
                .into_iter()
                .map(|record| (key.clone(), record)),
        );
    }

    let mut records: Vec<ManagedAgentRecord> = Vec::with_capacity(sourced.len());
    let mut home_of_kept: Vec<bool> = Vec::with_capacity(sourced.len());
    let mut index_by_pubkey: HashMap<String, usize> = HashMap::new();
    for (source, record) in sourced {
        if record.pubkey.is_empty() {
            // Key-less definitions only ever live in the legacy store.
            records.push(record);
            home_of_kept.push(true);
            continue;
        }
        let at_home = source == community_key_of(&record.relay_url);
        match index_by_pubkey.get(&record.pubkey) {
            Some(&index) => {
                if at_home || !home_of_kept[index] {
                    records[index] = record;
                    home_of_kept[index] = at_home;
                }
            }
            None => {
                index_by_pubkey.insert(record.pubkey.clone(), records.len());
                records.push(record);
                home_of_kept.push(at_home);
            }
        }
    }
    Ok(records)
}

/// Legacy (pre-sharding) store filename, still used for key-less definitions
/// and unpinned records. Path = `<base>/managed-agents.json`.
fn legacy_store_path(base_dir: &Path) -> PathBuf {
    base_dir.join("managed-agents.json")
}

/// Load the keyed agent *instances*. Key-less definitions (former personas,
/// folded into the same store) are filtered out so every pre-fold call site
/// keeps seeing exactly the records it always did.
pub fn load_managed_agents<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_store(app)?;
    records.retain(|record| !record.pubkey.is_empty());
    hydrate_keys(&mut records);
    Ok(records)
}

/// Community key ([`community_key_of`]) of the active workspace relay, or
/// `None` when it cannot be resolved.
///
/// Precedence mirrors `relay::relay_ws_url_with_override`: workspace override
/// first (community switch), then env/build vars, then the default. Resolving
/// through `relay_ws_url_with_override` (not the override alone) keeps the
/// pre-apply boot path working: before the frontend applies the first
/// workspace, no override is set and the default/env relay is the one used.
fn active_community_key<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<String> {
    let state = app.try_state::<AppState>()?;
    let url = crate::relay::relay_ws_url_with_override(&state);
    community_key_of(&url)
}

/// Keep only the records visible to the community with key `active`.
///
/// A record is visible when its `relay_url` does not resolve to a community
/// (unpinned legacy record — fail-open, matches the pre-sharding
/// shared-roster behavior) or when it resolves to exactly the active
/// community (full canonical relay identity, so two ports on one host are
/// two communities).
fn retain_visible_in_community(records: &mut Vec<ManagedAgentRecord>, active: &str) {
    records.retain(|record| {
        community_key_of(&record.relay_url)
            .map(|key| key == active)
            .unwrap_or(true) // unpinned → visible everywhere (fail-open)
    });
}

/// Filter already-loaded keyed instances down to the ones VISIBLE to the
/// active community (#7184). Read-side only: callers must never save the
/// filtered list — they keep saving the FULL roster from
/// [`load_managed_agents`], because a save
/// rewrites every community's file and clears the ones with no records.
///
/// Fail-open rule: when the active community cannot be resolved at all (no
/// state, unparseable URL) every instance is kept — isolation degrades to the
/// pre-fix behavior, never to an empty roster that would look like data loss.
pub fn retain_active_community<R: tauri::Runtime>(
    app: &AppHandle<R>,
    records: &mut Vec<ManagedAgentRecord>,
) {
    if let Some(active) = active_community_key(app) {
        retain_visible_in_community(records, &active);
    }
}

/// Load the key-less agent *definitions* (former personas) from the unified
/// store. The persona compatibility shim (`load_personas`) presents these in
/// the legacy shape via `to_definition_view`.
pub(crate) fn load_agent_definitions<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_store(app)?;
    records.retain(|record| record.pubkey.is_empty());
    Ok(records)
}

/// Preserve a malformed store file as `<name>.invalid` before the error path
/// unwinds. Copy, not rename: the original stays in place so repeated boots
/// keep failing loudly (rename would make the next launch look like a fresh
/// install and mint an empty store over the evidence). Overwrites any prior
/// `.invalid` — the newest broken content is the one worth keeping. Failure
/// here is logged and swallowed; it must never mask the parse error itself.
pub(crate) fn backup_invalid_store(path: &Path) {
    let backup = path.with_extension("json.invalid");
    if let Err(e) = fs::copy(path, &backup) {
        eprintln!(
            "buzz-desktop: failed to preserve malformed store {} as {}: {e}",
            path.display(),
            backup.display()
        );
    }
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
        // A key-less definition (no pubkey yet — unified agent model) has no
        // keyring entry by construction; keys are minted on first start.
        if record.pubkey.is_empty() {
            continue;
        }
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

/// Save the keyed agent *instances*, preserving the key-less definitions that
/// share the unified store: callers pass exactly the records they loaded via
/// [`load_managed_agents`], and this re-reads the definition half from disk
/// before the wholesale rewrite so a definition is never dropped by an
/// instance-side save (and vice versa via [`save_agent_definitions`]).
pub fn save_managed_agents<R: tauri::Runtime>(
    app: &AppHandle<R>,
    records: &[ManagedAgentRecord],
) -> Result<(), String> {
    let definitions = load_agent_definitions(app).unwrap_or_default();
    let mut sorted = records.to_vec();
    // A caller-supplied key-less record would collide with the definition
    // half re-read below; instances always carry a pubkey.
    sorted.retain(|record| !record.pubkey.is_empty());
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

    write_agent_store(app, definitions, sorted)
}

/// Save the key-less agent *definitions*, preserving the keyed instances —
/// the definition-side mirror of [`save_managed_agents`].
pub(crate) fn save_agent_definitions<R: tauri::Runtime>(
    app: &AppHandle<R>,
    definitions: &[ManagedAgentRecord],
) -> Result<(), String> {
    let mut instances = load_agent_store(app)?;
    instances.retain(|record| !record.pubkey.is_empty());
    let mut definitions = definitions.to_vec();
    definitions.retain(|record| record.pubkey.is_empty());
    write_agent_store(app, definitions, instances)
}

/// Serialize definitions + instances into the store files.
///
/// Definitions (key-less) and unpinned instances always land in the legacy
/// global store (`managed-agents.json`). Keyed instances whose `relay_url`
/// resolves to a community are written to that community's shard
/// (`managed-agents-community.<key>.json`, see [`community_key_of`]) — the
/// #7184 tenant-isolation boundary. `instances` is the WHOLE roster: every
/// record lands in exactly one file, and an existing shard with no records
/// left is cleared. The fail-open rule (unresolvable relay → legacy store)
/// means a record is never dropped by a save.
fn write_agent_store<R: tauri::Runtime>(
    app: &AppHandle<R>,
    definitions: Vec<ManagedAgentRecord>,
    instances: Vec<ManagedAgentRecord>,
) -> Result<(), String> {
    let base_dir = managed_agents_base_dir(app)?;
    write_agent_store_in_dir(&base_dir, definitions, instances, persist_agent_keys)
}

/// Path-level core of [`write_agent_store`]; `persist_keys` is the keyring
/// step ([`persist_agent_keys`] in production, a fake store in tests).
///
/// Secrets: `persist_keys` runs over EVERY record (definitions, legacy and
/// scoped instances) BEFORE anything is partitioned or serialized, and every
/// file is serialized from those stripped records, so a key that moved to the
/// keyring never reaches any file. Only a key the keyring could not take
/// stays inline (the keyringless fallback), in a `0o600` file.
///
/// One save = one recoverable commit (AGENTS.md "one user action = one
/// atomic persist"):
/// 1. every payload is serialized up front — nothing touches disk if any
///    record fails to serialize;
/// 2. each target file is snapshotted ([`snapshot_store`]) and unchanged
///    files are skipped, so a save that only touches one community rewrites
///    only that community's file;
/// 3. each file is written atomically (temp file + rename,
///    [`atomic_write_json_restricted`]) in an order where every prefix is
///    consistent: shards that gain or keep records first, then the legacy
///    store, then shards that became empty. A record moving between files is
///    written to its new home before it leaves the old one, so no prefix
///    loses it (a transient second copy is collapsed on load by
///    [`load_agent_store_in_dir`]);
/// 4. if any write fails, every file already written is restored from its
///    snapshot ([`restore_store`]), and restore failures are reported with
///    the original error — the same policy as
///    [`commit_stores_with_snapshots`].
fn write_agent_store_in_dir(
    base_dir: &Path,
    definitions: Vec<ManagedAgentRecord>,
    instances: Vec<ManagedAgentRecord>,
    persist_keys: impl FnOnce(&mut [ManagedAgentRecord]),
) -> Result<(), String> {
    // Keyring first, on every record, so nothing below can serialize a key
    // that was just moved to the keyring.
    let mut all: Vec<ManagedAgentRecord> = definitions;
    all.extend(instances);
    persist_keys(&mut all);

    let mut definitions_out: Vec<ManagedAgentRecord> = Vec::new();
    let mut keyed: Vec<ManagedAgentRecord> = Vec::new();
    let mut seen_pubkeys: HashMap<String, usize> = HashMap::new();
    for record in all {
        if record.pubkey.is_empty() {
            definitions_out.push(record);
        } else if let Some(&index) = seen_pubkeys.get(&record.pubkey) {
            // A caller holding a duplicate (e.g. loaded from a store written
            // by an earlier build) must not persist it twice; last one wins.
            keyed[index] = record;
        } else {
            seen_pubkeys.insert(record.pubkey.clone(), keyed.len());
            keyed.push(record);
        }
    }
    definitions_out.sort_by(|left, right| left.slug.cmp(&right.slug));

    // Each keyed record goes to exactly ONE file: its community shard, or the
    // legacy store when its relay does not resolve to a community.
    let (legacy_instances, shards) = partition_by_community(keyed);

    let legacy_path = legacy_store_path(base_dir);
    let legacy_payload = serde_json::to_vec_pretty(&{
        let mut combined = definitions_out;
        combined.extend(legacy_instances);
        combined
    })
    .map_err(|error| format!("failed to serialize agent store: {error}"))?;

    let mut shard_writes: Vec<(PathBuf, Vec<u8>)> = Vec::with_capacity(shards.len());
    for (key, mut shard_records) in shards {
        shard_records.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.pubkey.cmp(&right.pubkey))
        });
        let payload = serde_json::to_vec_pretty(&shard_records)
            .map_err(|error| format!("failed to serialize agent shard {key}: {error}"))?;
        shard_writes.push((base_dir.join(community_shard_file_name(&key)), payload));
    }

    // A community whose last record was deleted (or moved) has no entry in
    // the new partition, but its old shard is still enumerated on load.
    // Rewrite it as `[]` rather than deleting it — deletion would race a
    // concurrent reader, and `[]` keeps the fail-loud parse contract uniform
    // (a missing file is also valid).
    let empty_payload = serde_json::to_vec_pretty(&Vec::<ManagedAgentRecord>::new())
        .map_err(|error| format!("failed to serialize empty agent shard: {error}"))?;
    let stale_writes: Vec<(PathBuf, Vec<u8>)> = community_shard_paths(base_dir)
        .into_iter()
        .filter(|path| !shard_writes.iter().any(|(target, _)| target == path))
        .map(|path| (path, empty_payload.clone()))
        .collect();

    let mut plan = shard_writes;
    plan.push((legacy_path, legacy_payload));
    plan.extend(stale_writes);
    commit_store_plan(plan)
}

/// Apply an ordered list of `(path, payload)` atomic writes as one
/// recoverable commit: snapshot every target, skip targets whose bytes are
/// already equal, write the rest in order, and on the first failure restore
/// every file already written (newest first) from its snapshot.
fn commit_store_plan(plan: Vec<(PathBuf, Vec<u8>)>) -> Result<(), String> {
    let mut pending: Vec<(PathBuf, Vec<u8>, StoreSnapshot)> = Vec::with_capacity(plan.len());
    for (path, payload) in plan {
        let snapshot = snapshot_store(&path)?;
        if snapshot.as_deref() == Some(payload.as_slice()) {
            continue;
        }
        pending.push((path, payload, snapshot));
    }

    let mut written: Vec<(PathBuf, StoreSnapshot)> = Vec::with_capacity(pending.len());
    for (path, payload, snapshot) in pending {
        // `managed-agents*.json` can carry plaintext agent nsecs in the
        // keyringless fallback. Write owner-only (`0o600`) unconditionally —
        // harmless for the keyring-backed case and closes the umask window a
        // post-write `chmod` would leave open.
        if let Err(error) = atomic_write_json_restricted(&path, &payload) {
            let restore_errors: Vec<String> = written
                .into_iter()
                .rev()
                .filter_map(|(path, snapshot)| restore_store(&path, snapshot).err())
                .collect();
            if restore_errors.is_empty() {
                return Err(error);
            }
            return Err(format!(
                "{error} (and the agent store could not be restored: {})",
                restore_errors.join("; ")
            ));
        }
        written.push((path, snapshot));
    }
    Ok(())
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

/// One-time migration of agent keys from the production keyring service
/// (`"buzz-desktop"`) to the dev service (`"buzz-desktop-dev"`). Only runs
/// in debug builds — release builds never touch `"buzz-desktop"` from this
/// path.
///
/// Idempotent: skips any key that already exists in the dev service so
/// repeated boots after migration are no-ops. Leaves the production keyring
/// untouched — a dev build and a prod install can coexist without sharing
/// keys after this migration.
///
/// Call this at boot before `hydrate_keys` runs (i.e. before
/// `load_managed_agents` is called) so agents find their keys on first boot
/// after the service-name change.
#[cfg(debug_assertions)]
pub fn migrate_agent_keys_to_dev_service(app: &tauri::AppHandle) {
    if !cfg!(feature = "system-keyring") || keyring_service() != "buzz-desktop-dev" {
        return;
    }

    // Read the JSON store for pubkeys only — we want every instance
    // record without running hydrate_keys (which would try the dev
    // keyring that is empty, and log noisy "has no key" warnings).
    let records = match load_agent_store(app) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("buzz-desktop: keyring-dev-migration: cannot read agent store: {e}");
            return;
        }
    };

    let pubkeys: Vec<String> = records
        .into_iter()
        .filter(|r| !r.pubkey.is_empty())
        .map(|r| r.pubkey)
        .collect();
    // A fresh non-singleton store for the prod service — its own empty
    // cache so reads go to the OS keyring without polluting the dev
    // singleton's cache.
    let prod_store = crate::secret_store::SecretStore::keyring("buzz-desktop");
    let dev_store = crate::secret_store::SecretStore::shared(keyring_service());
    copy_agent_keys_between_stores(&pubkeys, &prod_store, dev_store);
}

/// Marker key stored inside the dev blob after a successful agent-key migration.
/// Its presence means all agent keys that existed in the prod service at
/// migration time have been copied; subsequent dev boots skip the migration
/// entirely (no prod keyring access).
#[cfg(debug_assertions)]
const DEV_MIGRATION_MARKER: &str = "_dev_migration_v1";

/// Testable core of [`migrate_agent_keys_to_dev_service`]: copy `agent:<pubkey>`
/// entries from `src` to `dst` for each pubkey, then write a migration-complete
/// marker so future boots skip the entire function with zero prod-keyring access.
///
/// On the first migration boot:
///   1. One `dst.load_all_readonly()` — dev blob read (1 keychain prompt)
///   2. One `src.load_all_readonly()` — prod blob read (1 keychain prompt)
///   3. One `dst.store_all()` — dev blob write (same service as #1; macOS may
///      skip the ACL prompt if the initial grant was "Always Allow")
///
/// On subsequent boots (marker already present):
///   1. One `dst.load_all_readonly()` — dev blob read (1 keychain prompt)
///      Returns immediately — prod keyring is NEVER accessed.
///
/// Idempotency: keys already present in `dst` are not overwritten (the agent
/// may have rotated their key in the dev service after initial migration).
/// New agents (pubkey not in `src`) are silently skipped — they will mint a
/// fresh key on their next onboarding run.
#[cfg(debug_assertions)]
fn copy_agent_keys_between_stores(pubkeys: &[String], src: &impl KeyStore, dst: &impl KeyStore) {
    // One read of the dev blob. If the migration-complete marker is present,
    // all prior agent keys are already in the dev service — skip entirely.
    let dst_map: HashMap<String, String> = match dst.load_all_readonly() {
        Ok(Some(map)) if map.contains_key(DEV_MIGRATION_MARKER) => {
            return; // already migrated: 0 prod keyring accesses
        }
        Ok(Some(map)) => map,
        Ok(None) => HashMap::new(),
        Err(e) => {
            eprintln!("buzz-desktop: keyring-dev-migration: cannot read dev keyring: {e}");
            return;
        }
    };
    // Skip production when a reset left no agents or onboarding created every dev key.
    let src_map: HashMap<String, String> = if pubkeys
        .iter()
        .all(|pubkey| dst_map.contains_key(&agent_keyring_name(pubkey)))
    {
        HashMap::new()
    } else {
        match src.load_all_readonly() {
            Ok(Some(map)) => map,
            Ok(None) => HashMap::new(), // prod has no blob yet — nothing to copy
            Err(e) => {
                eprintln!("buzz-desktop: keyring-dev-migration: cannot read prod keyring: {e}");
                return;
            }
        }
    };

    // Compute the set of entries to write: agent keys absent from dst, plus
    // the migration-complete marker.
    let mut to_write: HashMap<String, String> = HashMap::new();
    let mut copied = 0usize;
    for pubkey in pubkeys {
        let name = agent_keyring_name(pubkey);
        if dst_map.contains_key(&name) {
            continue; // already in dev service — do not overwrite (idempotent)
        }
        if let Some(nsec) = src_map.get(&name) {
            to_write.insert(name, nsec.clone());
            copied += 1;
        }
        // absent from src → new agent, will mint a fresh key
    }

    // Always write the marker so future boots skip the prod read entirely,
    // even when there were no keys to copy (empty dev environment).
    to_write.insert(DEV_MIGRATION_MARKER.to_string(), "done".to_string());

    if let Err(e) = dst.store_all(&to_write) {
        eprintln!("buzz-desktop: keyring-dev-migration: cannot write to dev keyring: {e}");
        return;
    }

    if copied > 0 {
        eprintln!(
            "buzz-desktop: keyring-dev-migration: copied {copied} agent key(s) from buzz-desktop"
        );
    }
}

/// Remove an agent's key from the keyring, returning an error on failure.
/// Used by the snapshot-import rollback path, which must surface cleanup
/// failures rather than swallowing them.
pub(crate) fn try_delete_agent_key(pubkey: &str) -> Result<(), String> {
    if let Some(store) = agent_secret_store() {
        store.delete(&agent_keyring_name(pubkey))
    } else {
        // No keyring backend — nothing to clean up.
        Ok(())
    }
}

/// Remove an agent's key from the keyring (best-effort). Called when an agent
/// is deleted so its secret does not linger in the OS store.
pub fn delete_agent_key(pubkey: &str) {
    if let Err(e) = try_delete_agent_key(pubkey) {
        eprintln!("buzz-desktop: failed to delete agent {pubkey} key from keyring: {e}");
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

// ── Two-store byte-level rollback ─────────────────────────────────────────
//
// Shared by `commands::teams::adopt::apply` (catalog adoption) and
// `managed_agents::teams` (adopted-team deletion). Identical rollback policy
// in both paths (I5 / I6).

/// Raw pre-write snapshot of a JSON store file.
///
/// `None` means the file did not exist at snapshot time; restoring `None`
/// removes the file (with `NotFound` treated as success — desired state
/// already reached).
pub(crate) type StoreSnapshot = Option<Vec<u8>>;

/// Snapshot the raw bytes of `path`, or `None` if the file is absent.
pub(crate) fn snapshot_store(path: &Path) -> Result<StoreSnapshot, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("failed to snapshot {}: {e}", path.display())),
    }
}

/// Restore `path` from a [`StoreSnapshot`].
///
/// `NotFound` when restoring an absent snap is treated as success — the
/// desired state is already reached (I5).
pub(crate) fn restore_store(path: &Path, snap: StoreSnapshot) -> Result<(), String> {
    match snap {
        Some(bytes) => atomic_write_json_restricted(path, &bytes),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(
                "failed to remove {} during restore: {e}",
                path.display()
            )),
        },
    }
}

/// Write both stores via the supplied callbacks, rolling back both from
/// caller-supplied snapshots on any failure.
///
/// Both restores are attempted independently, so a restore failure in one
/// store does not prevent the other; errors from both are aggregated (I5).
pub(crate) fn commit_stores_with_snapshots(
    personas_path: &Path,
    teams_path: &Path,
    personas_snap: StoreSnapshot,
    teams_snap: StoreSnapshot,
    write_personas: impl FnOnce() -> Result<(), String>,
    write_teams: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if let Err(error) = write_personas().and_then(|()| write_teams()) {
        let personas_err = restore_store(personas_path, personas_snap).err();
        let teams_err = restore_store(teams_path, teams_snap).err();
        let restore_errors: Vec<&str> = [personas_err.as_deref(), teams_err.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if !restore_errors.is_empty() {
            return Err(format!(
                "{error} (and the local stores could not be restored: {})",
                restore_errors.join("; ")
            ));
        }
        return Err(error);
    }
    Ok(())
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

/// Start a new install-log session at `path`: keep the previous run as
/// `<path>.1` and return a freshly created, empty current file.
///
/// Rotating per *run* rather than by size is what bounds this file. A run
/// writes one record per executed attempt, each capped by the log-scale
/// capture, so one run's file is bounded by steps × attempts × cap and the
/// history on disk is bounded at two runs. Size-triggered rotation could not
/// promise either: it never replaced an existing `.1`, and on Windows —
/// where rename does not replace its destination — it stopped working
/// altogether once `.1` existed, leaving the current file to grow.
///
/// The old `.1` is therefore *removed* before the rename rather than renamed
/// over. Every step is best-effort: a rotation that fails must not cost the
/// user the install, so the session continues with a truncated current file.
pub(crate) fn start_install_log_session(path: &Path) -> Result<File, String> {
    if path.exists() {
        let mut previous = path.as_os_str().to_owned();
        previous.push(".1");
        let previous = PathBuf::from(previous);
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(path, &previous);
    }
    open_install_log(path, /* truncate */ true)
}

/// Open an install log for appending one more record to the current session.
pub(crate) fn open_install_log_file(path: &Path) -> Result<File, String> {
    open_install_log(path, /* truncate */ false)
}

/// Open an install log owner-only.
///
/// The mode is set *in the create* rather than chmod'd afterwards, so the file
/// is never briefly group/world-readable. Install output can carry registry
/// tokens and proxy credentials echoed by a failing installer, so the window
/// matters even though it is short. An existing file's mode is left as-is —
/// `OpenOptions::mode` only applies on creation, and silently re-tightening a
/// file the user relaxed is not this function's call to make.
fn open_install_log(path: &Path, truncate: bool) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true);
    if truncate {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("failed to open log file {}: {error}", path.display()))
}

pub(crate) fn append_log_marker(path: &Path, message: &str) -> Result<(), String> {
    let mut file = open_log_file(path)?;
    writeln!(file, "{message}").map_err(|error| format!("failed to write log marker: {error}"))
}

fn agent_pids_dir<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("agent-pids");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create agent-pids dir: {error}"))?;
    Ok(dir)
}

/// Persist a pair-scoped runtime receipt atomically. Callers must register the
/// process in memory in the same runtime transition; on write failure they must
/// terminate the child before releasing that transition.
pub fn write_agent_runtime_receipt<R: tauri::Runtime>(
    app: &AppHandle<R>,
    receipt: &ManagedAgentRuntimeReceipt,
) -> Result<(), String> {
    let path = agent_pids_dir(app)?.join(format!("{}.json", receipt.key.runtime_id()));
    let payload = serde_json::to_vec(receipt)
        .map_err(|error| format!("failed to serialize runtime receipt: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

pub fn remove_agent_runtime_receipt<R: tauri::Runtime>(
    app: &AppHandle<R>,
    key: &ManagedAgentRuntimeKey,
) {
    if let Ok(dir) = agent_pids_dir(app) {
        let _ = fs::remove_file(dir.join(format!("{}.json", key.runtime_id())));
    }
}

pub fn remove_agent_runtime_receipt_path(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn read_all_agent_runtime_receipts<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Vec<(PathBuf, ManagedAgentRuntimeReceipt)> {
    let Ok(dir) = agent_pids_dir(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let path = entry.path();
            let bytes = fs::read(&path).ok()?;
            serde_json::from_slice(&bytes)
                .ok()
                .map(|receipt| (path, receipt))
        })
        .collect()
}

/// Remove the PID file for an agent (e.g. on normal stop).
pub fn remove_agent_pid_file<R: tauri::Runtime>(app: &AppHandle<R>, pubkey: &str) {
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

/// A meaningful error recovered from an exited agent's log tail.
pub struct AgentLogError {
    /// The full log line, wrapped as `Agent reported error…` for display.
    pub message: String,
    /// JSON-RPC error code parsed from the line's `(code N)` marker, or a
    /// synthetic code for known bare prefixes. `None` for legacy-format
    /// lines that carry no code (or when the code fails to parse as i64).
    pub code: Option<i64>,
}

pub fn meaningful_agent_error_from_log(path: &Path) -> Option<AgentLogError> {
    let tail = read_log_tail(path, 200).ok()?;
    tail.lines().rev().map(str::trim).find_map(|line| {
        // New format: "Agent reported error (code -32002): ..."
        if let Some(rest) = line.strip_prefix("Agent reported error (code ") {
            if let Some(paren_end) = rest.find("): ") {
                let code = rest[..paren_end].parse::<i64>().ok();
                return Some(AgentLogError {
                    message: line.to_string(),
                    code,
                });
            }
        }
        // Legacy format (older buzz-acp builds): "Agent reported error: ..."
        if line.starts_with("Agent reported error:") {
            return Some(AgentLogError {
                message: line.to_string(),
                code: None,
            });
        }
        // Bare prefixes emitted by older agent binaries whose Display still leaks
        // unwrapped errors. Promote these so they surface instead of the generic
        // "harness exited with status N" fallback.
        if line.starts_with("llm auth:") {
            return Some(AgentLogError {
                message: format!("Agent reported error: {line}"),
                code: Some(-32001),
            });
        }
        if line.starts_with("llm model not found:") {
            return Some(AgentLogError {
                message: format!("Agent reported error: {line}"),
                code: Some(-32002),
            });
        }
        None
    })
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
