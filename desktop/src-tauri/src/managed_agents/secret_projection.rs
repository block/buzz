//! Immutable generation-reference protocol for persisted secrets.
//!
//! # Overview
//!
//! All secrets (agent env vars, auth tags, provider configs, global env vars,
//! definition env vars) are stored as **immutable generations** in the existing
//! [`SecretStore`] keyring blob. Each save creates a new generation entry under
//! a unique ID; the stripped JSON record carries a non-secret `*_ref` field
//! pointing at the live generation. The **atomic JSON write** is the commit
//! point.
//!
//! ## Coordinates
//!
//! ```text
//! global:env:<gen>
//! agent:<pubkey>:env:<gen>
//! agent:<pubkey>:auth_tag:<gen>
//! agent:<pubkey>:provider_config:<gen>   (entire BackendKind::Provider.config blob)
//! definition:<slug>:env:<gen>            (durable AgentDefinition.id / slug)
//! ```
//!
//! ## Save protocol
//!
//! 1. Write new generation to blob + raw read-back verify.
//! 2. Mark the new generation as GC-safe-to-candidate by REMOVING any prior
//!    candidate mark it might have inherited — not applicable here since `gen`
//!    is new.
//! 3. Atomically commit JSON carrying the new `*_ref` (the JSON write is THE
//!    commit point).
//! 4. The old generation is NOT deleted here. Eager deletion before the JSON
//!    commit could orphan a committed secret on write failure; retirement is
//!    left entirely to the two-cycle GC below.
//!
//! ## GC (two-cycle)
//!
//! - Sweep 1 (`mark_gc_candidates`): parse both raw stores, enumerate all
//!   live `*_ref` values, then mark any generation in our namespaces that is
//!   NOT referenced as a candidate (stored as `<key>_candidate` = "1" in the
//!   blob).
//! - Sweep 2 (`delete_gc_candidates`): re-parse both raw stores to confirm a
//!   candidate is still unreferenced; only then delete it.
//! - A save in flight cancels its generation's candidacy before the JSON commit.
//! - GC is a no-op when either store is absent, unreadable, or changed between
//!   reference collection and blob mutation.
//!
//! ## Empty vs unavailable
//!
//! - **No `*_ref` field** = field is intentionally empty/absent → agent runs.
//! - **`*_ref` present but blob entry missing/unreadable** = unavailable →
//!   fail closed, nsec-style refusal.
//!
//! ## Inline fallback
//!
//! When a keyring write fails (Windows TooLong / backend error), the value
//! stays inline in the `0o600` JSON with a named warning. Inline and `*_ref`
//! are mutually exclusive in a healthy record; inline is authoritative when
//! both are present (takes priority over any stale ref during hydration).

use std::collections::{BTreeMap, HashMap};

use serde_json::Value as JsonValue;

use crate::secret_store::SecretStore;

// ── GC candidate suffix ────────────────────────────────────────────────────
//
// A GC candidate key is the generation key with this suffix appended.
// Example: `agent:abc:env:gen1` → `agent:abc:env:gen1_candidate`
//
// The value is always "1"; presence is the signal.
const GC_CANDIDATE_SUFFIX: &str = "_candidate";

// ── Secret-shape namespace prefixes ───────────────────────────────────────
const NS_GLOBAL_ENV: &str = "global:env:";
const NS_AGENT_ENV: &str = "agent:";
const NS_DEFINITION_ENV: &str = "definition:";

// ── Dev-migration conflict marker ──────────────────────────────────────────
//
// The dev secrets migration copies projection generations from the source
// keyring service into the dev service. A coordinate present in BOTH with
// DIFFERENT values is a conflict it refuses to resolve. Withholding the
// completion marker only schedules a retry — it does NOT stop the destination's
// (possibly wrong) value from being hydrated and consumed during the retry
// window. To make a conflicted coordinate genuinely unavailable, the migration
// writes a `conflict:<coordinate>` marker; `load_secret` fails closed whenever
// a coordinate carries one, so hydration sets `secrets_unavailable` and every
// downstream gate (spawn, deploy, readiness, the effective-secret gate) refuses
// until a later migration clears the marker.
const NS_CONFLICT: &str = "conflict:";

// ── Per-namespace sub-part constants ──────────────────────────────────────
const PART_ENV: &str = ":env:";
const PART_AUTH_TAG: &str = ":auth_tag:";
const PART_PROVIDER_CONFIG: &str = ":provider_config:";

// ── Generation ID ─────────────────────────────────────────────────────────

/// Generate a new unique generation ID for a blob key.
///
/// Uses UUIDv4 without hyphens: compact, URL-safe, and collision-resistant.
pub fn new_gen_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ── Blob key constructors ─────────────────────────────────────────────────

pub fn global_env_key(gen: &str) -> String {
    format!("{NS_GLOBAL_ENV}{gen}")
}

pub fn agent_env_key(pubkey: &str, gen: &str) -> String {
    format!("{NS_AGENT_ENV}{pubkey}{PART_ENV}{gen}")
}

pub fn agent_auth_tag_key(pubkey: &str, gen: &str) -> String {
    format!("{NS_AGENT_ENV}{pubkey}{PART_AUTH_TAG}{gen}")
}

pub fn agent_provider_config_key(pubkey: &str, gen: &str) -> String {
    format!("{NS_AGENT_ENV}{pubkey}{PART_PROVIDER_CONFIG}{gen}")
}

pub fn definition_env_key(slug: &str, gen: &str) -> String {
    format!("{NS_DEFINITION_ENV}{slug}{PART_ENV}{gen}")
}

/// Returns true if `key` is in one of our secret-projection namespaces
/// (including GC candidate markers).
pub fn is_projection_key(key: &str) -> bool {
    key.starts_with(NS_GLOBAL_ENV)
        || (key.starts_with(NS_AGENT_ENV)
            && (key.contains(PART_ENV)
                || key.contains(PART_AUTH_TAG)
                || key.contains(PART_PROVIDER_CONFIG)))
        || (key.starts_with(NS_DEFINITION_ENV) && key.contains(PART_ENV))
}

// ── KeyStore trait extension ───────────────────────────────────────────────

/// The subset of [`SecretStore`] operations the projection logic needs.
///
/// Abstracted for unit testing (can be backed by a [`FakeSecretStore`]).
pub trait ProjectionStore {
    fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String>;
    fn load_key(&self, key: &str) -> Result<Option<String>, String>;
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String>;
    fn store_batch(&self, entries: &HashMap<String, String>) -> Result<(), String>;
    fn remove_batch(&self, keys: &[&str]) -> Result<(), String>;

    /// Commit `entries` in a single blob mutation, then confirm each key holds
    /// its value with a durable, cache-bypassing read — the batched analogue of
    /// [`write_and_verify`](Self::write_and_verify).
    ///
    /// The default verifies via [`load_key`](Self::load_key), which is exact for
    /// stores with no cache/durable split (the test fakes). [`SecretStore`]
    /// overrides it with a raw keychain read so a backend that acknowledges a
    /// write it did not persist is still caught, exactly as the per-key path
    /// does.
    fn store_batch_verified(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.store_batch(entries)?;
        for (key, value) in entries {
            match self.load_key(key)? {
                Some(ref stored) if stored == value => {}
                _ => return Err(format!("keyring read-back verify failed for {key}")),
            }
        }
        Ok(())
    }
}

impl ProjectionStore for SecretStore {
    fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String> {
        self.store(key, value)?;
        match self.verify_stored_raw(key, value) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!("keyring read-back verify failed for {key}")),
            Err(e) => Err(format!("keyring read-back verify error for {key}: {e}")),
        }
    }

    fn load_key(&self, key: &str) -> Result<Option<String>, String> {
        self.load(key)
    }

    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        self.load_all_readonly()
    }

    fn store_batch(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.store_all(entries)
    }

    fn remove_batch(&self, keys: &[&str]) -> Result<(), String> {
        for key in keys {
            self.delete(key)?;
        }
        Ok(())
    }

    /// Override the default so verification bypasses the in-process cache: after
    /// `store_all` advances the cache to the written state, a cached `load`
    /// would pass even when the OS keychain write silently failed.
    /// [`verify_stored_raw`](SecretStore::verify_stored_raw) reads the raw blob
    /// direct from the backend, proving the round-trip the same way the per-key
    /// `write_and_verify` does.
    fn store_batch_verified(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.store_all(entries)?;
        for (key, value) in entries {
            match self.verify_stored_raw(key, value) {
                Ok(true) => {}
                Ok(false) => return Err(format!("keyring read-back verify failed for {key}")),
                Err(e) => return Err(format!("keyring read-back verify error for {key}: {e}")),
            }
        }
        Ok(())
    }
}

// ── Projection outcome ─────────────────────────────────────────────────────

/// Result of writing a secret to the keyring and verifying it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Written and read-back verified. JSON `*_ref` should be set to the new gen.
    Persisted { gen: String },
    /// Keyring write failed (backend error or Windows TooLong). Value must stay
    /// inline in `0o600` JSON with a warning; ref field must be cleared.
    KeptInline { reason: String },
    /// The value was empty/None — no write attempted, no ref set.
    Nothing,
}

// ── Write-with-verify ─────────────────────────────────────────────────────

/// Attempt to write `value` to the keyring under a new generation key
/// `blob_key(new_gen_id())`.
///
/// Returns [`WriteOutcome::Persisted`] on success, [`WriteOutcome::KeptInline`]
/// on any failure, or [`WriteOutcome::Nothing`] when `value` is `None`.
pub fn write_secret<S: ProjectionStore>(
    store: &S,
    coord_key_fn: impl FnOnce(&str) -> String,
    value: Option<&str>,
    context: &str,
) -> WriteOutcome {
    let Some(v) = value else {
        return WriteOutcome::Nothing;
    };
    let gen = new_gen_id();
    let key = coord_key_fn(&gen);
    match store.write_and_verify(&key, v) {
        Ok(()) => WriteOutcome::Persisted { gen },
        Err(e) => {
            eprintln!(
                "buzz-desktop: keyring write failed for {context} ({e}); \
                 keeping inline in 0o600 JSON (retry on next boot)"
            );
            WriteOutcome::KeptInline { reason: e }
        }
    }
}

// ── Batched write-with-reuse ───────────────────────────────────────────────

/// One field's contribution to a batched secret save.
///
/// The coordinate builder is `&dyn Fn` so a single [`write_secrets_batched`]
/// call can carry fields from different namespaces (env / auth_tag /
/// provider_config) without monomorphizing over each closure type.
pub struct FieldSave<'a> {
    /// Builds the full blob coordinate for a given generation id.
    pub coord_key_fn: &'a dyn Fn(&str) -> String,
    /// The serialized inline value, or `None` when the field is empty/absent.
    pub value: Option<&'a str>,
    /// The generation currently referenced on disk, if any — the reuse anchor.
    pub existing_ref: Option<&'a str>,
    /// Human-readable context for diagnostics.
    pub context: &'a str,
}

/// Persist several secret fields in a SINGLE blob mutation, reusing the live
/// generation for any field whose bytes are unchanged.
///
/// Returns one [`WriteOutcome`] per input field, in order, so the seam applies
/// the same per-field record mutation it always has:
///
/// - `value == None` → [`WriteOutcome::Nothing`] (no write).
/// - `value` byte-equal to what `existing_ref` already stores →
///   [`WriteOutcome::Persisted`] with the SAME generation and **no write**: no
///   new UUID is minted, no blob mutation happens, and the generation never
///   becomes GC-eligible, so a metadata-only save is free of churn.
/// - `value` changed (or no prior ref, or the prior value is unreadable) → a
///   fresh generation is staged and committed with every other changed field in
///   ONE [`store_batch_verified`](ProjectionStore::store_batch_verified). On
///   success each is [`WriteOutcome::Persisted`] with its new gen; if the single
///   mutation fails, ALL staged fields become [`WriteOutcome::KeptInline`]
///   together (the blob write is atomic — there is no torn partial state).
///
/// # Why the old generation is safe
///
/// Like [`write_secret`], this never deletes a prior generation: a changed field
/// mints a NEW coordinate and leaves the old one untouched, so a subsequent
/// failed JSON commit still finds the on-disk ref's generation live. Retirement
/// stays with the two-cycle GC.
///
/// # Why no `cancel_gc_candidacy`
///
/// The per-field seam cancelled candidacy after each write. That is redundant on
/// this path and is deliberately dropped so the save is exactly one mutation:
/// every save holds the cross-process transaction lock, under which GC cannot
/// run, and a freshly-minted UUID generation has never been observed by a sweep
/// (so it carries no candidate marker), while a REUSED generation is a live JSON
/// ref that `mark_gc_candidates` skips by construction. Neither a new nor a
/// reused generation can hold a candidate marker at save time, so there is
/// nothing to cancel. (The boot-migration path keeps its cancel: it is
/// contract-frozen and its semantics are proven elsewhere.)
pub fn write_secrets_batched<S: ProjectionStore>(
    store: &S,
    fields: &[FieldSave<'_>],
) -> Vec<WriteOutcome> {
    let mut outcomes: Vec<Option<WriteOutcome>> = Vec::with_capacity(fields.len());
    let mut batch: HashMap<String, String> = HashMap::new();
    // (field index, freshly-minted gen) for each field staged into `batch`,
    // finalized after the one write resolves.
    let mut pending: Vec<(usize, String)> = Vec::new();

    for (idx, field) in fields.iter().enumerate() {
        let Some(value) = field.value else {
            outcomes.push(Some(WriteOutcome::Nothing));
            continue;
        };
        // Gen-reuse: if the live ref already stores these exact bytes, keep the
        // generation and write nothing. A load error or absent value falls
        // through to a fresh write (which, on a real outage, fails closed to
        // KeptInline) — never a silent reuse of an unverified generation.
        if let Some(existing) = field.existing_ref {
            if matches!(
                store.load_key(&(field.coord_key_fn)(existing)),
                Ok(Some(ref stored)) if stored == value
            ) {
                outcomes.push(Some(WriteOutcome::Persisted {
                    gen: existing.to_string(),
                }));
                continue;
            }
        }
        let gen = new_gen_id();
        batch.insert((field.coord_key_fn)(&gen), value.to_string());
        pending.push((idx, gen));
        outcomes.push(None); // finalized after the batched write
    }

    if !batch.is_empty() {
        match store.store_batch_verified(&batch) {
            Ok(()) => {
                for (idx, gen) in pending {
                    outcomes[idx] = Some(WriteOutcome::Persisted { gen });
                }
            }
            Err(e) => {
                let contexts: Vec<&str> = pending
                    .iter()
                    .map(|(idx, _)| fields[*idx].context)
                    .collect();
                eprintln!(
                    "buzz-desktop: batched keyring write failed ({e}); keeping \
                     inline in 0o600 JSON (retry on next boot): {}",
                    contexts.join(", ")
                );
                for (idx, _) in pending {
                    outcomes[idx] = Some(WriteOutcome::KeptInline { reason: e.clone() });
                }
            }
        }
    }

    outcomes
        .into_iter()
        .map(|o| o.expect("every field assigned an outcome"))
        .collect()
}

// ── Load-with-availability ─────────────────────────────────────────────────

/// The conflict-marker key for a projection coordinate: `conflict:<coord>`.
pub fn conflict_marker_key(coord: &str) -> String {
    format!("{NS_CONFLICT}{coord}")
}

/// Load a secret from the keyring given its `ref_gen` from JSON.
///
/// Returns:
/// - `Ok(Some(value))` — entry found and loaded.
/// - `Ok(None)` — no `ref_gen` in the record (field intentionally empty).
/// - `Err(msg)` — `ref_gen` is present but the entry is unavailable → fail
///   closed. The caller must refuse agent start/save.
///
/// # Conflict marker (fail closed)
///
/// When the coordinate carries a `conflict:<coord>` marker (written by the dev
/// secrets migration for a coordinate whose source and destination values
/// disagree), the value is treated as UNAVAILABLE regardless of what the blob
/// currently holds. The destination value cannot be trusted while unresolved,
/// so hydration must set `secrets_unavailable` and every downstream consumer
/// must refuse — this is the fail-closed replacement for the marker-withhold +
/// retry behavior that used to leave the conflicted value hydratable.
pub fn load_secret<S: ProjectionStore>(
    store: &S,
    ref_gen: Option<&str>,
    coord_key_fn: impl FnOnce(&str) -> String,
    context: &str,
) -> Result<Option<String>, String> {
    let Some(gen) = ref_gen else {
        return Ok(None); // intentionally empty
    };
    let key = coord_key_fn(gen);
    // Fail closed on an unresolved dev-migration conflict for this coordinate.
    // Only a definitive `Ok(None)` (marker absent) proceeds to the value read.
    //
    // A marker-read `Err(_)` MUST also fail closed: `SecretStore::load_blob`
    // caches a successful read but never caches an error, so a transient
    // marker-read failure could be followed immediately by a *successful*
    // value read that hydrates a known-conflicted credential. Falling through
    // on `Err` would therefore re-open exactly the window the marker exists to
    // close, so a marker read we cannot complete is treated as "conflict
    // status unknown" → unavailable.
    match store.load_key(&conflict_marker_key(&key)) {
        Ok(Some(_)) => {
            return Err(format!(
                "secret unavailable: {context} ref {gen} has an unresolved \
                 dev-migration conflict at {key}; refusing to hydrate a \
                 potentially-wrong value until the conflict is resolved"
            ))
        }
        Err(e) => {
            return Err(format!(
                "secret unavailable: {context} ref {gen} conflict-marker read \
                 at {key} failed ({e}); refusing to hydrate until the marker \
                 can be checked (a later cached-success value read must not \
                 bypass an unresolved conflict)"
            ))
        }
        Ok(None) => {} // no marker — proceed to the normal load
    }
    match store.load_key(&key) {
        Ok(Some(v)) => Ok(Some(v)),
        Ok(None) => Err(format!(
            "secret unavailable: {context} ref {gen} not found in keyring \
             (keyring may be unreachable or entry was deleted)"
        )),
        Err(e) => Err(format!(
            "secret unavailable: {context} ref {gen} keyring error: {e}"
        )),
    }
}

// ── Delete helpers ─────────────────────────────────────────────────────────
//
// Old generations are NEVER deleted eagerly on save: deleting a prior
// generation before the atomic JSON commit could orphan a secret if the write
// fails, leaving disk referencing a generation that no longer exists. All
// retirement of unreferenced generations happens through the two-cycle GC
// below (`mark_gc_candidates` + `delete_gc_candidates`).

// ── GC snapshot ───────────────────────────────────────────────────────────

/// The live-reference snapshot collected from both JSON stores by
/// [`collect_live_refs`].
#[derive(Debug, Default)]
pub struct LiveRefs {
    /// All validated ref gen ids referenced by any live JSON record. Used by
    /// the sweeps to decide whether a blob key's generation is still
    /// referenced (and therefore must not be marked/deleted).
    pub gen_ids: std::collections::HashSet<String>,
    /// The full expected blob coordinate for every live ref
    /// (e.g. `agent:<pubkey>:env:<gen>`, `global:env:<gen>`). Every one of
    /// these MUST be present in the blob before GC may delete anything: a
    /// dangling live ref (its coordinate missing/unreadable) means the store
    /// is in a degraded state where an older unreferenced generation could be
    /// the only recoverable payload for that field, so BOTH sweeps no-op until
    /// the reference resolves.
    pub coords: std::collections::HashSet<String>,
}

/// Returns `true` when every live-ref coordinate is present as a key in the
/// loaded blob. A single missing coordinate means a committed reference is
/// dangling (its keyring entry was deleted or is unreadable), so GC must not
/// delete anything this cycle — an older, unreferenced generation could be the
/// only recoverable payload for that field.
fn all_live_coords_present(live: &LiveRefs, blob: &HashMap<String, String>) -> bool {
    live.coords.iter().all(|coord| blob.contains_key(coord))
}

/// Collect all live generation refs from both raw JSON stores, validating as
/// it goes.
///
/// Returns `None` — which makes the GC sweep a **no-op** for this cycle — when
/// either store is missing/unreadable OR the JSON is in an ambiguous state the
/// GC must not make a deletion decision against:
///
/// - **Malformed coordinate:** a `*_ref` that is empty or contains a `:`
///   (the gen id is the last `:`-segment of a blob key, so an embedded `:`
///   would make the reference un-matchable against the blob and could leave a
///   still-referenced generation unprotected).
/// - **Duplicate coordinate:** the same gen id referenced by two different
///   coordinates. Generation ids are fresh UUIDs, so a collision means the JSON
///   is corrupt; protecting only one of the two would let the sweep delete a
///   live secret.
/// - **Inline + ref conflict:** a record (or global) that carries BOTH a
///   non-empty inline value AND a `*_ref` for the same field. Inline is
///   authoritative on load, so the ref is being ignored — but the state is
///   ambiguous enough that the GC must not reason about which generation is
///   live. Skipping the whole sweep is the fail-safe choice.
/// - **Unidentifiable record:** a record carrying a `*_ref` whose owning
///   coordinate cannot be reconstructed (an instance with no pubkey, or a
///   definition with no slug). The full coordinate is required for the
///   blob-existence check, so an unbuildable one no-ops the sweep.
///
/// The returned [`LiveRefs`] carries every validated gen id AND the full
/// coordinate each ref points at. The sweeps additionally require every
/// coordinate to exist in the loaded blob before deleting anything.
pub fn collect_live_refs(agents_json: &str, global_json: &str) -> Option<LiveRefs> {
    let mut live = LiveRefs::default();

    // Parse agents store (array of records).
    let agents: Vec<JsonValue> = serde_json::from_str(agents_json).ok()?;
    for record in &agents {
        collect_refs_from_record(record, &mut live)?;
    }

    // Parse global config. Global carries a single `env_vars` / `env_vars_ref`
    // pair with the same inline-precedence contract as a record field.
    let global: JsonValue = serde_json::from_str(global_json).ok()?;
    collect_ref_field(
        &global,
        "env_vars_ref",
        /* inline_non_empty */ object_field_non_empty(&global, "env_vars"),
        &mut live,
        |gen| Some(global_env_key(gen)),
    )?;

    Some(live)
}

/// Validate and collect the three secret refs of one agent/definition record.
/// Returns `None` on any malformed/duplicate coordinate, inline+ref conflict,
/// or a ref on a record whose owning coordinate cannot be reconstructed.
fn collect_refs_from_record(record: &JsonValue, live: &mut LiveRefs) -> Option<()> {
    let pubkey = record
        .get("pubkey")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let slug = record.get("slug").and_then(JsonValue::as_str);
    let is_definition = pubkey.is_empty();

    // env_vars: object, non-empty inline. Instance → agent:<pubkey>:env:<gen>;
    // definition (no pubkey) → definition:<slug>:env:<gen>.
    collect_ref_field(
        record,
        "env_vars_ref",
        object_field_non_empty(record, "env_vars"),
        live,
        |gen| {
            if is_definition {
                slug.map(|s| definition_env_key(s, gen))
            } else {
                Some(agent_env_key(pubkey, gen))
            }
        },
    )?;
    // auth_tag: string, non-empty inline. Instance-only coordinate.
    collect_ref_field(
        record,
        "auth_tag_ref",
        string_field_non_empty(record, "auth_tag"),
        live,
        |gen| (!is_definition).then(|| agent_auth_tag_key(pubkey, gen)),
    )?;
    // provider config: BackendKind::Provider.config, non-null inline.
    // Instance-only coordinate.
    collect_ref_field(
        record,
        "provider_config_ref",
        provider_config_inline_present(record),
        live,
        |gen| (!is_definition).then(|| agent_provider_config_key(pubkey, gen)),
    )?;
    Some(())
}

/// Validate a single `(inline, ref)` field pair and record both the ref gen id
/// and its full blob coordinate into `live`.
///
/// Returns `None` on an inline+ref conflict, a malformed ref, a duplicate ref
/// gen id, or a ref whose `coord_fn` cannot reconstruct the owning coordinate
/// (unidentifiable record) — every one no-ops the sweep as the fail-safe.
fn collect_ref_field(
    record: &JsonValue,
    ref_field: &str,
    inline_non_empty: bool,
    live: &mut LiveRefs,
    coord_fn: impl FnOnce(&str) -> Option<String>,
) -> Option<()> {
    let ref_val = record.get(ref_field).and_then(JsonValue::as_str);
    match ref_val {
        Some(r) => {
            // Inline present alongside a ref → ambiguous; no-op the sweep.
            if inline_non_empty {
                return None;
            }
            // Malformed coordinate: empty, or an embedded `:` that would break
            // last-segment gen extraction against the blob.
            if r.is_empty() || r.contains(':') {
                return None;
            }
            // The full coordinate must be reconstructible — an instance ref
            // with no pubkey, or a definition env ref with no slug, is
            // un-checkable against the blob, so no-op the sweep.
            let coord = coord_fn(r)?;
            // Duplicate coordinate: a gen id must reference exactly one thing.
            if !live.gen_ids.insert(r.to_string()) {
                return None;
            }
            live.coords.insert(coord);
            Some(())
        }
        None => Some(()),
    }
}

/// True when `record[field]` is a JSON object with at least one entry.
fn object_field_non_empty(record: &JsonValue, field: &str) -> bool {
    record
        .get(field)
        .and_then(JsonValue::as_object)
        .is_some_and(|m| !m.is_empty())
}

/// True when `record[field]` is a non-empty JSON string.
fn string_field_non_empty(record: &JsonValue, field: &str) -> bool {
    record
        .get(field)
        .and_then(JsonValue::as_str)
        .is_some_and(|s| !s.is_empty())
}

/// True when the record's backend is a provider whose `config` is present and
/// not JSON `null` — i.e. an inline provider-config value that has not been
/// stripped into the keyring.
fn provider_config_inline_present(record: &JsonValue) -> bool {
    let Some(backend) = record.get("backend").and_then(JsonValue::as_object) else {
        return false;
    };
    if backend.get("type").and_then(JsonValue::as_str) != Some("provider") {
        return false;
    }
    matches!(backend.get("config"), Some(c) if !c.is_null())
}

// ── Two-cycle GC ──────────────────────────────────────────────────────────

/// First GC sweep: mark unreferenced projection generations as candidates.
///
/// Reads both JSON stores (raw bytes for stability) and the current blob state.
/// Any generation key in our namespaces that is NOT in the live ref set AND
/// does not have an in-flight save cancelling its candidacy is marked as a
/// candidate by writing `<key>_candidate = "1"` into the blob.
///
/// GC is a no-op when:
/// - Either JSON store is absent or unreadable.
/// - The blob is unreachable.
/// - The JSON store content changed between `collect_live_refs` call and blob
///   mutation (this is checked by comparing the read content before and after
///   — but since we can't hold a lock across the reads, we use a snapshot
///   approach: re-read JSON after acquiring the blob lock implicitly via
///   `store_batch`). The current impl re-reads JSON inside `mark_gc_candidates`
///   to ensure stability.
pub fn mark_gc_candidates<S: ProjectionStore>(
    store: &S,
    agents_json_path: &std::path::Path,
    global_json_path: &std::path::Path,
) {
    let (agents_content, global_content) =
        match read_both_json_stores(agents_json_path, global_json_path) {
            Some(pair) => pair,
            None => return,
        };

    let live_refs = match collect_live_refs(&agents_content, &global_content) {
        Some(refs) => refs,
        None => {
            eprintln!(
                "buzz-desktop: GC sweep 1: could not collect live refs (malformed JSON), skipping"
            );
            return;
        }
    };

    // Read current blob to find projection keys.
    let blob = match store.load_all() {
        Ok(Some(map)) => map,
        Ok(None) => return, // no blob yet — nothing to GC
        Err(e) => {
            eprintln!("buzz-desktop: GC sweep 1: keyring unavailable ({e}), skipping");
            return;
        }
    };

    // Fail-safe: every live ref's full coordinate MUST exist in the blob. A
    // dangling live ref means the store is degraded — an older unreferenced
    // generation could be the only recoverable payload for that field — so no
    // marking happens this cycle until the reference resolves.
    if !all_live_coords_present(&live_refs, &blob) {
        eprintln!(
            "buzz-desktop: GC sweep 1: a live ref's blob entry is missing — \
             store is degraded, skipping to protect recoverable generations"
        );
        return;
    }

    // Find projection generation keys that are:
    // 1. In our namespaces (not candidate markers themselves).
    // 2. Not referenced by any live JSON record.
    // 3. Not already a candidate marker (suffix _candidate).
    let mut to_mark: HashMap<String, String> = HashMap::new();
    for key in blob.keys() {
        if key.ends_with(GC_CANDIDATE_SUFFIX) {
            continue; // skip existing candidate markers
        }
        if !is_projection_key(key) {
            continue; // not ours
        }
        // Extract gen from the key — the gen is the last `:` segment.
        let gen = match key.rsplit(':').next() {
            Some(g) if !g.is_empty() => g,
            _ => continue,
        };
        if live_refs.gen_ids.contains(gen) {
            continue; // referenced by a live record — do NOT mark
        }
        // Unreferenced generation — mark it as a candidate.
        let candidate_key = format!("{key}{GC_CANDIDATE_SUFFIX}");
        to_mark.insert(candidate_key, "1".to_string());
    }

    if to_mark.is_empty() {
        return;
    }

    // Re-read JSON to verify it hasn't changed since our snapshot.
    // If it has changed, abort GC for this cycle.
    let (agents_after, global_after) =
        match read_both_json_stores(agents_json_path, global_json_path) {
            Some(pair) => pair,
            None => {
                eprintln!("buzz-desktop: GC sweep 1: JSON stores changed mid-sweep, skipping");
                return;
            }
        };
    if agents_after != agents_content || global_after != global_content {
        eprintln!("buzz-desktop: GC sweep 1: JSON stores changed mid-sweep, skipping");
        return;
    }

    if let Err(e) = store.store_batch(&to_mark) {
        eprintln!("buzz-desktop: GC sweep 1: could not write candidate markers ({e})");
    } else {
        eprintln!(
            "buzz-desktop: GC sweep 1: marked {} generation(s) as GC candidates",
            to_mark.len()
        );
    }
}

/// Second GC sweep: delete candidate generations that are STILL unreferenced.
///
/// Re-parses both JSON stores before deleting anything. Any candidate whose
/// generation is now referenced (i.e. a save committed between sweep 1 and 2)
/// is skipped. GC is a no-op when either store is unreadable.
pub fn delete_gc_candidates<S: ProjectionStore>(
    store: &S,
    agents_json_path: &std::path::Path,
    global_json_path: &std::path::Path,
) {
    let (agents_content, global_content) =
        match read_both_json_stores(agents_json_path, global_json_path) {
            Some(pair) => pair,
            None => return,
        };

    let live_refs = match collect_live_refs(&agents_content, &global_content) {
        Some(refs) => refs,
        None => {
            eprintln!("buzz-desktop: GC sweep 2: could not collect live refs, skipping");
            return;
        }
    };

    let blob = match store.load_all() {
        Ok(Some(map)) => map,
        Ok(None) => return,
        Err(e) => {
            eprintln!("buzz-desktop: GC sweep 2: keyring unavailable ({e}), skipping");
            return;
        }
    };

    // Same fail-safe as sweep 1: a dangling live ref blocks ALL deletion this
    // cycle so an unreferenced generation that may be the last recoverable
    // payload survives until the reference resolves.
    if !all_live_coords_present(&live_refs, &blob) {
        eprintln!(
            "buzz-desktop: GC sweep 2: a live ref's blob entry is missing — \
             store is degraded, skipping to protect recoverable generations"
        );
        return;
    }

    // Find candidate markers whose base generation is still unreferenced.
    let mut to_delete: Vec<String> = Vec::new();
    for key in blob.keys() {
        let Some(base_key) = key.strip_suffix(GC_CANDIDATE_SUFFIX) else {
            continue; // not a candidate marker
        };
        if !is_projection_key(base_key) {
            continue;
        }
        // Re-verify: extract gen from base_key.
        let gen = match base_key.rsplit(':').next() {
            Some(g) if !g.is_empty() => g,
            _ => continue,
        };
        if live_refs.gen_ids.contains(gen) {
            // A save committed between sweep 1 and 2 — keep it.
            continue;
        }
        // Still unreferenced — schedule both the generation and its candidate
        // marker for deletion.
        to_delete.push(base_key.to_string());
        to_delete.push(key.clone()); // the _candidate marker
    }

    if to_delete.is_empty() {
        return;
    }

    // Re-check JSON one last time before deleting.
    let (agents_final, global_final) =
        match read_both_json_stores(agents_json_path, global_json_path) {
            Some(pair) => pair,
            None => {
                eprintln!("buzz-desktop: GC sweep 2: JSON stores changed, skipping");
                return;
            }
        };
    if agents_final != agents_content || global_final != global_content {
        eprintln!("buzz-desktop: GC sweep 2: JSON stores changed mid-sweep, skipping");
        return;
    }

    let keys_ref: Vec<&str> = to_delete.iter().map(String::as_str).collect();
    match store.remove_batch(&keys_ref) {
        Ok(()) => {
            eprintln!(
                "buzz-desktop: GC sweep 2: deleted {} stale generation(s)",
                to_delete.len() / 2
            );
        }
        Err(e) => {
            eprintln!("buzz-desktop: GC sweep 2: delete failed ({e})");
        }
    }
}

/// Cancel GC candidacy for `gen_key` (called before the JSON commit of a save).
///
/// Removes the `<gen_key>_candidate` marker if present. Best-effort: failure
/// is logged but does not block the save.
pub fn cancel_gc_candidacy<S: ProjectionStore>(store: &S, gen_key: &str) {
    let candidate_key = format!("{gen_key}{GC_CANDIDATE_SUFFIX}");
    if let Err(e) = store.remove_batch(&[&candidate_key]) {
        eprintln!("buzz-desktop: could not cancel GC candidacy for {gen_key}: {e}");
    }
}

fn read_both_json_stores(
    agents_path: &std::path::Path,
    global_path: &std::path::Path,
) -> Option<(String, String)> {
    // Resolve symlinks before reading so concurrent atomic-write renames at the
    // real target path don't confuse us.
    let agents_resolved =
        std::fs::canonicalize(agents_path).unwrap_or_else(|_| agents_path.to_path_buf());
    let global_resolved =
        std::fs::canonicalize(global_path).unwrap_or_else(|_| global_path.to_path_buf());

    let agents = match std::fs::read_to_string(&agents_resolved) {
        Ok(s) => s,
        Err(_) => {
            // File absent on first launch is OK — treat as empty array.
            if !agents_path.exists() {
                "[]".to_string()
            } else {
                return None;
            }
        }
    };
    let global = match std::fs::read_to_string(&global_resolved) {
        Ok(s) => s,
        Err(_) => {
            if !global_path.exists() {
                "{}".to_string()
            } else {
                return None;
            }
        }
    };
    Some((agents, global))
}

// ── Env-map serialization helpers ─────────────────────────────────────────

/// Serialize an env map to a compact JSON string for keyring storage.
pub fn serialize_env_map(env: &BTreeMap<String, String>) -> Result<String, String> {
    serde_json::to_string(env).map_err(|e| format!("env_map serialize: {e}"))
}

/// Deserialize an env map from a JSON string loaded from the keyring.
pub fn deserialize_env_map(s: &str) -> Result<BTreeMap<String, String>, String> {
    serde_json::from_str(s).map_err(|e| format!("env_map deserialize: {e}"))
}

/// Serialize a provider config value for keyring storage.
pub fn serialize_provider_config(config: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string(config).map_err(|e| format!("provider_config serialize: {e}"))
}

/// Deserialize a provider config from keyring storage.
pub fn deserialize_provider_config(s: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(s).map_err(|e| format!("provider_config deserialize: {e}"))
}

#[cfg(test)]
#[path = "secret_projection_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "secret_projection_batched_tests.rs"]
mod batched_tests;
