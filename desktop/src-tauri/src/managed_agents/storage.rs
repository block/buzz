use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

use crate::app_state::keyring_service;
use crate::managed_agents::{
    secret_seam::{hydrate_all_secrets_for_records, strip_and_persist_all_for_records},
    AgentDefinition, ManagedAgentRecord,
};
use crate::secret_store::{KeyringProbe, SecretStore};

mod logs;
pub use logs::*;

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

pub fn managed_agents_base_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("agents");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create agents dir: {error}"))?;
    Ok(dir)
}

pub(crate) fn managed_agents_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("managed-agents.json"))
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
/// chokepoint ([`persist_agent_keys_with`]). An empty key returns
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
///
/// Also refuses when `record.secrets_unavailable` is set — at least one secret
/// field (env_vars, auth_tag, provider_config) has a keyring ref that points to
/// a missing or unreachable entry. Spawning with silently empty secrets is the
/// exact failure mode the gen-ref protocol exists to prevent.
pub(crate) fn spawn_key_refusal(record: &ManagedAgentRecord) -> Option<String> {
    if record.private_key_nsec.is_empty() {
        return Some(format!(
            "agent {} has no private key available — the OS keyring may be unreachable. \
             Refusing to start without an identity; retry once the keyring is reachable.",
            record.pubkey
        ));
    }
    if record.secrets_unavailable {
        return Some(format!(
            "agent {} has one or more secrets (env vars, auth tag, or provider config) \
             that could not be loaded from the keyring. Refusing to start with missing \
             secrets; retry once the keyring is reachable.",
            record.pubkey
        ));
    }
    None
}

/// The linked definition's id when that definition's secrets are unavailable —
/// its `env_vars_ref` is present but could not be hydrated from the keyring.
///
/// This is the definition tier of the fail-closed spawn gate, alongside
/// [`spawn_key_refusal`]'s instance-tier `secrets_unavailable` check and the
/// global-tier check in `spawn_agent_child`. Returning the offending id (rather
/// than a bool) lets the spawn path name it in the refusal message without a
/// second lookup; status callers use `.is_some()`. An unlinked record, or one
/// whose linked definition is absent from `personas`, yields `None`.
pub(crate) fn unavailable_definition_id<'a>(
    record: &'a ManagedAgentRecord,
    personas: &[AgentDefinition],
) -> Option<&'a str> {
    let pid = record.persona_id.as_deref()?;
    personas
        .iter()
        .find(|p| p.id == pid)
        .filter(|d| d.secrets_unavailable)
        .map(|_| pid)
}

/// Read the raw unified store — keyed instances AND key-less definitions —
/// with fail-loud parse handling. Internal seam; public readers filter.
fn load_agent_store(app: &AppHandle) -> Result<Vec<ManagedAgentRecord>, String> {
    load_agent_store_at(&managed_agents_store_path(app)?)
}

/// Path-based core of [`load_agent_store`]: reads and parses the raw store at
/// `path`. Split out so the save path and tests can drive the load/split/write
/// merge against a tempdir without an `AppHandle`.
fn load_agent_store_at(path: &Path) -> Result<Vec<ManagedAgentRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content =
        fs::read_to_string(path).map_err(|error| format!("failed to read agent store: {error}"))?;
    serde_json::from_str(&content).map_err(|error| {
        // Fail loudly and preserve the evidence: a later in-app save rewrites
        // this file wholesale, which would silently destroy a malformed hand
        // edit. Best-effort file-authoring contract (see managed_agents::
        // reconcile): the broken content survives as `.invalid` for the user
        // to recover, and the parse error propagates instead of being
        // swallowed into an empty store.
        backup_invalid_store(path);
        format!("failed to parse agent store (preserved as .invalid): {error}")
    })
}

/// Load the keyed agent *instances*. Key-less definitions (former personas,
/// folded into the same store) are filtered out so every pre-fold call site
/// keeps seeing exactly the records it always did.
pub fn load_managed_agents(app: &AppHandle) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_store(app)?;
    records.retain(|record| !record.pubkey.is_empty());
    hydrate_keys(&mut records);
    // Hydrate env/auth_tag/provider_config from keyring.
    if let Some(store) = agent_secret_store() {
        let _ = hydrate_all_secrets_for_records(store, &mut records);
        // Unavailability is set directly on each record (`secrets_unavailable`);
        // the returned Vec is a secondary summary, not needed here.  The spawn
        // path consults `spawn_key_refusal`, which checks `secrets_unavailable`
        // and refuses to start any agent whose secrets could not be loaded.
    }
    Ok(records)
}

/// Load the key-less agent *definitions* (former personas) from the unified
/// store. The persona compatibility shim (`load_personas`) presents these in
/// the legacy shape via `to_definition_view`.
pub(crate) fn load_agent_definitions(app: &AppHandle) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_store(app)?;
    records.retain(|record| record.pubkey.is_empty());
    // Hydrate definition env_vars from keyring.
    if let Some(store) = agent_secret_store() {
        let _ = hydrate_all_secrets_for_records(store, &mut records);
    }
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
pub fn save_managed_agents(app: &AppHandle, records: &[ManagedAgentRecord]) -> Result<(), String> {
    save_managed_agents_locked_at(
        &managed_agents_store_path(app)?,
        agent_secret_store(),
        records,
    )
}

/// Lock-owning path-based entry point for the instance-side save: acquires the
/// cross-process secret transaction lock on the store directory, then runs the
/// definition-preserving [`save_managed_agents_at`] under it. The lock and the
/// mutation it protects live in ONE seam — the same one the interleave test
/// drives — so the wiring is provable: delete the acquisition here and the
/// concurrent-save regression stops observing exclusion. See
/// [`acquire_secret_txn_lock`] for the lock span + residual.
fn save_managed_agents_locked_at<S>(
    store_path: &Path,
    store: Option<&S>,
    records: &[ManagedAgentRecord],
) -> Result<(), String>
where
    S: KeyStore + crate::managed_agents::secret_projection::ProjectionStore,
{
    // Lock FIRST — the definition half re-read inside `save_managed_agents_at`
    // must be under the lock (Race 1).
    let _txn = crate::secret_store::transaction_lock_at(&crate::secret_store::store_txn_lock_dir(
        store_path,
    ))?;
    save_managed_agents_at(store_path, store, records)
}

/// Path-based core of [`save_managed_agents`]: merges the caller's instance
/// records with the definition half re-read from disk and commits the unified
/// store. Split out so the definition-preservation contract can be exercised
/// over a tempdir without an `AppHandle`.
///
/// The definition half is re-read RAW ([`load_agent_store_at`] + retain), NOT
/// through the hydrating `load_agent_definitions`: a hydrated definition holds
/// its `env_vars` inline, so writing it back would re-inline the definition's
/// provider secrets into plaintext JSON on every instance-side save — the exact
/// regression the projection protocol exists to prevent — and would create the
/// inline+ref conflict that freezes GC. A raw definition is already stripped on
/// disk, so it needs no strip pass. The parse error propagates with `?` instead
/// of collapsing into an empty definition half: the wholesale rewrite below
/// would otherwise delete every definition from the live store.
fn save_managed_agents_at<S>(
    store_path: &Path,
    store: Option<&S>,
    records: &[ManagedAgentRecord],
) -> Result<(), String>
where
    S: KeyStore + crate::managed_agents::secret_projection::ProjectionStore,
{
    let mut definitions = load_agent_store_at(store_path)?;
    definitions.retain(|record| record.pubkey.is_empty());

    let mut sorted = records.to_vec();
    // A caller-supplied key-less record would collide with the definition
    // half re-read above; instances always carry a pubkey.
    sorted.retain(|record| !record.pubkey.is_empty());
    sorted.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.pubkey.cmp(&right.pubkey))
    });

    if let Some(store) = store {
        // Persist each nsec to the keyring; on success blank the inline copy.
        persist_agent_keys_with(store, &mut sorted);
        // Persist env/auth_tag/provider_config to the keyring; on success blank
        // inline values and set *_ref. On failure keep inline and clear *_ref.
        strip_and_persist_all_for_records(store, &mut sorted);
    }

    write_agent_store_at(store_path, definitions, sorted)
}

/// Save the key-less agent *definitions*, preserving the keyed instances —
/// the definition-side mirror of [`save_managed_agents`].
pub(crate) fn save_agent_definitions(
    app: &AppHandle,
    definitions: &[ManagedAgentRecord],
) -> Result<(), String> {
    save_agent_definitions_locked_at(
        &managed_agents_store_path(app)?,
        agent_secret_store(),
        definitions,
    )
}

/// Lock-owning path-based entry point for the definition-side save — the mirror
/// of [`save_managed_agents_locked_at`]. Acquires the cross-process secret
/// transaction lock on the store directory, then runs [`save_agent_definitions_at`]
/// under it, so the lock and the instance-half re-read it protects are one seam
/// the interleave test drives directly.
fn save_agent_definitions_locked_at<S>(
    store_path: &Path,
    store: Option<&S>,
    definitions: &[ManagedAgentRecord],
) -> Result<(), String>
where
    S: crate::managed_agents::secret_projection::ProjectionStore,
{
    // Lock FIRST — the instance half re-read inside `save_agent_definitions_at`
    // must be under the lock (Race 1); mirror of `save_managed_agents`.
    let _txn = crate::secret_store::transaction_lock_at(&crate::secret_store::store_txn_lock_dir(
        store_path,
    ))?;
    save_agent_definitions_at(store_path, store, definitions)
}

/// Path-based core of [`save_agent_definitions`]: merges the caller's definition
/// records with the instance half re-read from disk and commits the unified
/// store — the definition-side mirror of [`save_managed_agents_at`]. Split out
/// so the instance-preservation contract can be exercised over a tempdir
/// without an `AppHandle`.
///
/// The instance half is re-read RAW ([`load_agent_store_at`] + retain): a
/// definition-side save must never drop a concurrently-committed instance, and
/// the raw records already carry their secrets as `*_ref` (keys in the keyring,
/// inline blanked), so writing them back cannot re-inline anything. The parse
/// error propagates with `?` rather than collapsing into an empty instance half
/// — the wholesale rewrite below would otherwise delete every instance.
fn save_agent_definitions_at<S>(
    store_path: &Path,
    store: Option<&S>,
    definitions: &[ManagedAgentRecord],
) -> Result<(), String>
where
    S: crate::managed_agents::secret_projection::ProjectionStore,
{
    let mut instances = load_agent_store_at(store_path)?;
    instances.retain(|record| !record.pubkey.is_empty());
    let mut definitions = definitions.to_vec();
    definitions.retain(|record| record.pubkey.is_empty());

    // Persist definition env_vars to the keyring before writing JSON.
    if let Some(store) = store {
        strip_and_persist_all_for_records(store, &mut definitions);
    }

    write_agent_store_at(store_path, definitions, instances)
}

/// Serialize definitions + instances into the single unified store file.
/// Definitions sort first (by slug) for stable diffs; instances keep the
/// name/pubkey order their save path established.
fn write_agent_store_at(
    path: &Path,
    mut definitions: Vec<ManagedAgentRecord>,
    instances: Vec<ManagedAgentRecord>,
) -> Result<(), String> {
    definitions.sort_by(|left, right| left.slug.cmp(&right.slug));
    let mut all = definitions;
    all.extend(instances);

    let payload = serde_json::to_vec_pretty(&all)
        .map_err(|error| format!("failed to serialize agent store: {error}"))?;

    // `managed-agents.json` carries plaintext agent nsecs in the keyringless
    // fallback. Write it owner-only (`0o600`) unconditionally — harmless for the
    // keyring-backed case (it is the user's own agent store) and closes the
    // umask window a post-write `chmod` would leave open.
    atomic_write_json_restricted(path, &payload)
}

/// Write each record's in-memory key to the keyring and blank the inline copy
/// on success. Keys that cannot be persisted (keyring unreachable) stay inline
/// in the JSON. Mutates `records` (a save-local clone) — the caller's in-memory
/// records keep their keys.
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

// ── Migration-seam helpers (pub(crate) for use by migration.rs) ───────────

/// Load the raw unified store (instances + definitions) without any keyring
/// hydration. Used by the boot migration to read inline secrets before
/// extracting them into the keyring. After extraction, call
/// [`write_agent_store_raw`] to persist the updated records.
pub(crate) fn load_agent_store_raw(app: &AppHandle) -> Result<Vec<ManagedAgentRecord>, String> {
    load_agent_store(app)
}

/// Write the raw unified store (instances + definitions) without any keyring
/// strip step. Used by the boot migration after inline secrets have been
/// extracted into the keyring and the `*_ref` fields set accordingly.
pub(crate) fn write_agent_store_raw(
    app: &AppHandle,
    records: &[ManagedAgentRecord],
) -> Result<(), String> {
    let path = managed_agents_store_path(app)?;
    let payload = serde_json::to_vec_pretty(records)
        .map_err(|e| format!("failed to serialize agent store: {e}"))?;
    atomic_write_json_restricted(&path, &payload)
}

/// Return the shared secret store used for agent secrets, or `None` when the
/// build has no keyring backend. Exposed as `pub(crate)` so `migration.rs`
/// can run GC sweeps and the secret-migration function against the same store
/// instance used by the normal save/load path.
pub(crate) fn agent_secret_store_pub() -> Option<&'static crate::secret_store::SecretStore> {
    agent_secret_store()
}

/// Acquire the cross-process secret transaction lock for the agent store, or
/// `Ok(None)` on a keyless build. A save and GC hold it for their full span so
/// two Desktop processes cannot interleave. Keyed by the resolved store dir
/// ([`crate::secret_store::store_txn_lock_dir`]). Residual (deferred, PR body):
/// the lock makes each `save_*` atomic cross-process but cannot make a caller's
/// pre-lock snapshot transactional — caller races stay last-writer-wins (pre-existing).
pub(crate) fn acquire_secret_txn_lock(
    app: &AppHandle,
) -> Result<Option<crate::secret_store::SecretTxnGuard>, String> {
    if agent_secret_store().is_none() {
        return Ok(None);
    }
    let dir = crate::secret_store::store_txn_lock_dir(&managed_agents_store_path(app)?);
    crate::secret_store::transaction_lock_at(&dir).map(Some)
}

/// Resolve the store-directory lock target the secret transaction lock uses,
/// from the same `managed-agents.json` path [`acquire_secret_txn_lock`] resolves.
/// Exposed so the identity-persist seam ([`crate::commands::identity::persist_identity_locked`])
/// contends on the exact directory inode every agent save takes — keeping the
/// lock-target resolution in one place.
pub(crate) fn secret_txn_lock_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(crate::secret_store::store_txn_lock_dir(
        &managed_agents_store_path(app)?,
    ))
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

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
