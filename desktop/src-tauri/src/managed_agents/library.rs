//! Cross-workspace agent library — device-local, unscoped metadata (§2).
//!
//! `library.json` (`<agents-base>/library.json`, `0o600`, never published) is
//! the single source of truth for shared agent definitions, their per-scope
//! projections, and the crash-safe identity bindings that carry one npub across
//! a same-owner's workspaces. This module is the Phase-1 data model and its
//! quarantine-preserving IO; the operations that mutate it (share, edit,
//! materialize, delete, deploy) land in Phases 2-5. The scope-local journals
//! that must share a *workspace's* failure domain rather than the library's
//! (P9-I1) live in the [`journals`] submodule.
//!
//! Fault model (§2.1):
//! - Absent file = a valid empty library (version 1, no entries).
//! - Whole-document syntax failure → preserved as `library.json.invalid`
//!   (copy, not rename — the `storage.rs` loud-failure discipline) and NO
//!   library or scoped mutation proceeds; workspaces keep running from their
//!   scoped caches.
//! - Unknown/forward `version` → read-only fail: no library or scoped mutation.
//! - A single malformed, semantically invalid, or identity-colliding entry is
//!   quarantined (kept raw, surfaced as a degradation, never merged, never a
//!   winner) while its healthy siblings stay fully usable, and every healthy
//!   rewrite preserves the quarantined entry value-equivalently (P2-I2).
//!
//! Items are `pub(crate)` for the Phase 2-5 callers; suppress the dead-code
//! lint until those land (matches `team_snapshot.rs`).
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use nostr::{Keys, PublicKey, ToBech32};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::storage::{
    agent_keyring_name, atomic_write_json_restricted, backup_invalid_store, KeyStore,
};
use super::types::ManagedAgentRecord;

pub(crate) mod journals;

/// The only `library.json` schema version v1 understands. A document whose
/// `version` differs is a forward format: read-only fail, never migrated.
pub(crate) const SUPPORTED_LIBRARY_VERSION: u32 = 1;

/// Resolve the unscoped `library.json` path under the agents base directory.
/// The library is a peer of `scopes/`, never inside a scope — it is device
/// metadata shared across every workspace (§2, layout).
pub(crate) fn library_path(base_dir: &Path) -> PathBuf {
    base_dir.join("library.json")
}

// ── Document envelope ─────────────────────────────────────────────────────────

/// The versioned `library.json` envelope (§2.1). Entries are retained RAW and
/// decoded individually so one malformed entry can never reject the whole file,
/// and re-serializing only the valid ones can never silently erase a
/// quarantined sibling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LibraryDocument {
    /// `1` in v1; an unknown value is a forward format (read-only fail).
    pub version: u32,
    /// Entries retained RAW and decoded one at a time (§2.1 quarantine).
    pub entries: Vec<Value>,
    /// Orphan-key journal (§2.5): pubkeys minted for a binding whose commit may
    /// not have completed. Non-secret (pubkeys only). The ONLY journal in
    /// `library.json` — binding state is library-linked by definition; plain
    /// crash journals live scope-local (P9-I1, [`journals`]).
    #[serde(default)]
    pub orphan_keys: Vec<String>,
}

impl LibraryDocument {
    /// A valid empty library (the absent-file and freshly-initialized shape).
    pub fn empty() -> Self {
        Self {
            version: SUPPORTED_LIBRARY_VERSION,
            entries: Vec::new(),
            orphan_keys: Vec::new(),
        }
    }
}

// ── Shared content ────────────────────────────────────────────────────────────

/// Allowlisted shared agent content (§2.2). Deliberately NOT `AgentDefinition`:
/// `env_vars`, credentials, identity, `is_active`, catalog `shared`,
/// catalog/team provenance, runtime state, relay, memory, timestamps, and
/// projection metadata are structurally absent — they cannot ride a share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharedDefinition {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub name_pool: Vec<String>,
    /// NIP-AP behavioral defaults, WIRE shape (kebab-case string / optional u32)
    /// — parsed only at the mint boundary, so an unknown future mode string
    /// round-trips byte-identically.
    pub respond_to: Option<String>,
    pub respond_to_allowlist: Vec<String>,
    pub parallelism: Option<u32>,
}

/// The ONLY writer of shared content onto a scoped keyless definition record
/// (§2.2). Assigns exactly the [`SharedDefinition`] slots, plus
/// `library_applied_revision` and `updated_at`; every other field of `local` —
/// identity, `env_vars`, `is_active`, runtime state, `library_ref` — is
/// untouched by construction. The value mapping mirrors
/// `AgentDefinition::into_agent_record` so a record populated this way is
/// byte-identical to one freshly projected.
pub(crate) fn apply_shared_definition(
    local: &mut ManagedAgentRecord,
    shared: &SharedDefinition,
    revision: u64,
) {
    local.name = shared.display_name.clone();
    local.display_name = Some(shared.display_name.clone());
    local.avatar_url = shared.avatar_url.clone();
    local.system_prompt = (!shared.system_prompt.is_empty()).then(|| shared.system_prompt.clone());
    local.runtime = shared.runtime.clone();
    local.model = shared.model.clone();
    local.provider = shared.provider.clone();
    local.name_pool = shared.name_pool.clone();
    local.definition_respond_to = shared.respond_to.clone();
    local.definition_respond_to_allowlist = shared.respond_to_allowlist.clone();
    local.definition_parallelism = shared.parallelism;
    local.library_applied_revision = Some(revision);
    local.updated_at = crate::util::now_iso();
}

// ── Library entry ─────────────────────────────────────────────────────────────

/// A shared library entry (§2.3). `library_id` routes scoped records
/// (`library_ref`/`lib-<id>`, §2.6); `origin` is the share idempotency key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LibraryEntry {
    /// UUID v4, minted once at share commit.
    pub library_id: String,
    /// `(origin scope_id, origin slug)` — share idempotency key.
    pub origin: OriginKey,
    /// Monotonic integer; wall-clock is display-only.
    pub revision: u64,
    /// Tombstone; kept forever in v1.
    pub deleted: bool,
    /// Provenance guard only — never an identity lookup key.
    pub owner_pubkey_at_share: String,
    pub shared: SharedDefinition,
    /// Owner-keyed PUBLIC identity bindings — pubkey + auth tag, NEVER an nsec.
    /// A binding exists only after its nsec is write-and-read-back verified in
    /// the device keyring (§2.5); there is no "pending binding" state.
    pub identity_bindings: BTreeMap<String, IdentityBinding>,
    /// Archive obligations deferred by a protected removal (§3.6 step 3).
    /// PERMANENT journaled retirement markers in v1 (P17-C1): no v1 path
    /// discharges them; their presence keeps `key_archive_protected(pubkey)`
    /// true. SET semantics on `(library_id, scope_id, agent_pubkey)`
    /// (`library_id` is fixed per entry). PRIVATE by construction — the ONLY
    /// access outside this module is [`upsert_deferred_archive`] (every append
    /// an upsert, P15-MINOR) and [`deferred_archive_obligations`] (reads legacy
    /// duplicate rows as one obligation), so no crate caller can `.push()` a
    /// duplicate or observe multiplicity. Serde and the child-module tests reach
    /// the field directly by module ancestry, not visibility.
    ///
    /// [`upsert_deferred_archive`]: LibraryEntry::upsert_deferred_archive
    /// [`deferred_archive_obligations`]: LibraryEntry::deferred_archive_obligations
    #[serde(default)]
    deferred_archives: Vec<DeferredArchive>,
    /// Authoritative per-scope membership — sole source for delete-confirm
    /// enumeration, tombstone completion, and key lifetime.
    pub projections: BTreeMap<String, ProjectionEntry>,
}

impl LibraryEntry {
    /// Upsert a deferred-archive obligation (§2.3, P15-MINOR). SET semantics on
    /// `(scope_id, agent_pubkey)` — `library_id` is fixed per entry — so §3.6's
    /// crash-then-re-consent retry can re-run the obligation write idempotently:
    /// a marker already present is a no-op, never a duplicate row. This is the
    /// ONLY insertion path; callers never push onto `deferred_archives`
    /// directly. Returns `true` iff a new marker was added.
    pub fn upsert_deferred_archive(&mut self, scope_id: String, agent_pubkey: String) -> bool {
        if self
            .deferred_archives
            .iter()
            .any(|d| d.scope_id == scope_id && d.agent_pubkey == agent_pubkey)
        {
            return false;
        }
        self.deferred_archives.push(DeferredArchive {
            scope_id,
            agent_pubkey,
        });
        true
    }

    /// The deferred-archive obligations as a SET (§2.3): a legacy `Vec` with
    /// duplicate rows is collapsed to one obligation per `(scope_id,
    /// agent_pubkey)`. Phase-3 callers read obligations through this view, never
    /// the raw field, so multiplicity in old on-disk data never becomes
    /// multiplicity in behavior.
    pub fn deferred_archive_obligations(&self) -> Vec<&DeferredArchive> {
        let mut seen = std::collections::HashSet::new();
        self.deferred_archives
            .iter()
            .filter(|d| seen.insert((d.scope_id.as_str(), d.agent_pubkey.as_str())))
            .collect()
    }

    /// The §3.5 retirement-due derived condition (P6-I2; permanently
    /// conservative per P15-I2/P16-I1/P17-C1). Retirement is a DERIVED condition,
    /// never a stored state: an entry is retirement-due when it has at least one
    /// projection, EVERY projection is terminal (`Excluded`/`Deleted`), and a
    /// binding key or a `deferred_archives` marker still names it. The empty-
    /// projection guard keeps a never-deployed entry (vacuously "all terminal")
    /// from being flagged — retirement means it WAS live and now every
    /// projection is terminal.
    ///
    /// Whatever write produces this condition (a cascade confirming, another
    /// scope's activation reconcile confirming, a §3.6 direct deletion completing
    /// a pending removal), the finalizer OBSERVES it at recovery points and
    /// discharges NOTHING — see [`retirement_due_entries`].
    ///
    /// [`retirement_due_entries`]: LoadedLibrary::retirement_due_entries
    pub fn is_retirement_due(&self) -> bool {
        !self.projections.is_empty()
            && self.projections.values().all(|p| p.state.is_terminal())
            && (!self.identity_bindings.is_empty()
                || !self.deferred_archive_obligations().is_empty())
    }
}

/// `(origin scope_id, origin slug)` — the share idempotency key (§2.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginKey {
    pub scope_id: String,
    pub slug: String,
}

/// A verified owner→agent identity binding (§2.5). `auth_tag` is the NIP-OA
/// `auth` tag JSON; the map key is the owner pubkey it embeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityBinding {
    pub agent_pubkey: String,
    pub auth_tag: String,
}

/// One skipped archive (§2.3): the identity `agent_pubkey`, live on `scope_id`'s
/// relay, whose archive request was skipped by a protected removal in THAT
/// scope. A permanent v1 retirement marker — never discharged by any v1 path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeferredArchive {
    pub scope_id: String,
    pub agent_pubkey: String,
}

/// This scope's projection of a library entry (§2.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionEntry {
    pub state: ProjectionState,
    /// This scope's actual definition slug, fixed at `Pending` commit: the
    /// deterministic `lib-<library_id>` for a lazily materialized scope, or the
    /// origin record's original human slug for the sharing scope. Always equals
    /// the scoped record's `slug` and its instances' `persona_id`.
    pub local_slug: String,
    /// Non-secret metadata for the ruling-4 confirm dialog, captured at this
    /// scope's own activation (or at share, for the sharing scope).
    pub relay_url: String,
    pub workspace_label: Option<String>,
}

/// The uniform journal-first projection state machine (§2.3, P2-C1/C2). Every
/// bracketed scoped-file step is flanked by library writes: intent precedes the
/// scoped mutation, confirmation follows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum ProjectionState {
    /// Journal intent: materialization not yet confirmed.
    Pending,
    Materialized {
        revision: u64,
    },
    /// Journal intent: user removed in this workspace; scoped cascade not yet
    /// confirmed (P2-C2). `manifest` journaled BEFORE any deletion (P4-C3).
    ExcludePending {
        manifest: Option<RemovalManifest>,
    },
    /// Terminal: cascade confirmed; NEVER rematerialize.
    Excluded,
    /// Library deleted; this scope's cascade not yet confirmed.
    DeletePending {
        manifest: Option<RemovalManifest>,
    },
    /// Terminal.
    Deleted,
}

impl ProjectionState {
    /// `Excluded`/`Deleted` are terminal; the identity index only conflicts on
    /// non-terminal `(scope_id, local_slug)` ownership (§2.1 index rule (c)).
    pub fn is_terminal(&self) -> bool {
        matches!(self, ProjectionState::Excluded | ProjectionState::Deleted)
    }
}

/// Everything a removal retry needs after the local records have left disk
/// (§2.3, P4-C3). Captured by the record-owning scope from its own readable
/// cache, in the same or an earlier library write than the FIRST destructive
/// scoped step. Non-secret (pubkeys/coordinates only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemovalManifest {
    /// Persona tombstone coordinate: `(owner pubkey, definition d-tag)`.
    pub definition_coordinate: (String, String),
    /// Every linked instance pubkey at capture time; each requires the full
    /// per-pubkey cleanup set (30177 tombstone + archive, key deletion only per
    /// the §2.5 entry-wide lifetime, re-evaluated against the index at execution
    /// time, never trusted stale).
    pub instances: Vec<String>,
}

// ── Quarantine-preserving load (§2.1) ─────────────────────────────────────────

/// The outcome of reading `library.json` (§2.1). The four states the
/// failure-domain rules key on.
#[derive(Debug)]
pub(crate) enum LibraryLoad {
    /// Absent file — a valid empty library (version 1, no entries).
    Empty,
    /// A healthy version-1 document, classified into healthy + quarantined.
    Loaded(LoadedLibrary),
    /// Whole-document syntax/shape failure. Preserved as `library.json.invalid`;
    /// NO library or scoped mutation may proceed.
    Corrupt,
    /// Parsed, but `version` is unknown/forward: read-only fail.
    UnknownVersion(u32),
}

/// A successfully parsed version-1 library, partitioned by the §2.1 rules. The
/// raw `quarantined` values are retained verbatim so a healthy rewrite
/// preserves them value-equivalently (P2-I2).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoadedLibrary {
    /// Individually valid, semantically sound, non-colliding entries.
    pub healthy: Vec<LibraryEntry>,
    /// Decode / semantic / identity-collision failures, kept RAW.
    pub quarantined: Vec<Value>,
    pub orphan_keys: Vec<String>,
    /// Human-readable reasons, one per quarantined entry, for the UI.
    pub degradations: Vec<String>,
}

impl LoadedLibrary {
    /// Reconstruct the on-disk document, re-serializing healthy entries and
    /// re-attaching every quarantined raw value unchanged (P2-I2 preservation).
    pub fn rebuild_document(&self) -> Result<LibraryDocument, String> {
        let mut entries = Vec::with_capacity(self.healthy.len() + self.quarantined.len());
        for entry in &self.healthy {
            entries.push(
                serde_json::to_value(entry)
                    .map_err(|e| format!("serialize library entry {}: {e}", entry.library_id))?,
            );
        }
        entries.extend(self.quarantined.iter().cloned());
        Ok(LibraryDocument {
            version: SUPPORTED_LIBRARY_VERSION,
            entries,
            orphan_keys: self.orphan_keys.clone(),
        })
    }

    /// The §3.5 binding-retirement finalizer — MARKER MAINTENANCE ONLY, the sole
    /// v1 behavior (P6-I2, permanently conservative per P15-I2/P16-I1/P17-C1).
    /// Returns the healthy entries that are retirement-due (every projection
    /// terminal while a binding key or `deferred_archives` marker still names the
    /// pubkey — [`LibraryEntry::is_retirement_due`]).
    ///
    /// It is a pure OBSERVER: it discharges nothing. No v1 path deletes an
    /// ever-bound key or archives an ever-bound identity, because library
    /// metadata alone cannot prove a process-global secret or a community-visible
    /// identity is unused — an unrelated plain carrier of the bound pubkey may
    /// live in an inactive scope no deleting authority may read (P17-C1). The
    /// keyring entry, the deferred rows, and the tombstoned entry's binding
    /// record therefore persist as PERMANENT journaled markers a future indexed
    /// version may safely consume. The recovery-point that calls this (§4)
    /// records the observation and leaves every marker in place; wiring it into
    /// the recovery sweep is Phase 4b. Quarantined entries are excluded — their
    /// markers already protect the key via [`LibraryDocument::key_archive_protected`],
    /// and a malformed entry's projection set cannot be trusted for terminality.
    pub fn retirement_due_entries(&self) -> Vec<&LibraryEntry> {
        self.healthy
            .iter()
            .filter(|entry| entry.is_retirement_due())
            .collect()
    }
}

/// Read and classify `library.json` under `base_dir` (§2.1). Never mutates the
/// file; whole-document corruption is preserved as `.invalid` as a read side
/// effect so the loud-failure evidence survives.
pub(crate) fn load_library(base_dir: &Path) -> LibraryLoad {
    let path = library_path(base_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LibraryLoad::Empty,
        Err(_) => {
            // An unreadable-but-present file is preserved and treated as
            // corrupt: no mutation may proceed against an unknown on-disk state.
            backup_invalid_store(&path);
            return LibraryLoad::Corrupt;
        }
    };
    let document: LibraryDocument = match serde_json::from_slice(&bytes) {
        Ok(document) => document,
        Err(_) => {
            backup_invalid_store(&path);
            return LibraryLoad::Corrupt;
        }
    };
    if document.version != SUPPORTED_LIBRARY_VERSION {
        return LibraryLoad::UnknownVersion(document.version);
    }
    LibraryLoad::Loaded(classify_entries(document))
}

/// Persist a document (§2.1): atomic temp-file + rename + `0o600`, same as
/// `managed-agents.json` (`storage.rs` `write_agent_store_to_path`). Creates the
/// parent agents dir if needed.
pub(crate) fn save_library_document(
    base_dir: &Path,
    document: &LibraryDocument,
) -> Result<(), String> {
    let path = library_path(base_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create agents dir {}: {e}", parent.display()))?;
    }
    let payload =
        serde_json::to_vec_pretty(document).map_err(|e| format!("serialize library.json: {e}"))?;
    atomic_write_json_restricted(&path, &payload)
}

/// Per-entry decode + semantic validation + document-wide identity index
/// (§2.1). Every failure quarantines its entry (raw-preserved, degradation);
/// identity collisions group-quarantine ALL colliders (never first-wins).
fn classify_entries(document: LibraryDocument) -> LoadedLibrary {
    let LibraryDocument {
        entries,
        orphan_keys,
        ..
    } = document;

    // Pass 1: individual decode + semantic binding validation. Survivors keep
    // their ORIGINAL raw value alongside the decoded form, so a later
    // group-quarantine preserves the exact on-disk bytes (P2-I2) rather than a
    // re-serialization that could drift or fail.
    let mut decoded: Vec<(LibraryEntry, Value)> = Vec::new();
    let mut quarantined: Vec<Value> = Vec::new();
    let mut degradations: Vec<String> = Vec::new();
    for raw in entries {
        match serde_json::from_value::<LibraryEntry>(raw.clone()) {
            Err(e) => {
                degradations.push(format!("library entry failed to decode: {e}"));
                quarantined.push(raw);
            }
            Ok(entry) => match validate_entry_bindings(&entry) {
                Err(reason) => {
                    degradations.push(format!(
                        "library entry {} quarantined: {reason}",
                        entry.library_id
                    ));
                    quarantined.push(raw);
                }
                Ok(()) => decoded.push((entry, raw)),
            },
        }
    }

    // Pass 2: document-wide identity index — collisions group-quarantine every
    // collider (§2.1 rule; P6-I3), preserved via the pass-1 original raw.
    let colliders = identity_collisions(decoded.iter().map(|(entry, _)| entry));
    let mut healthy = Vec::new();
    for (index, (entry, raw)) in decoded.into_iter().enumerate() {
        if colliders.contains(&index) {
            degradations.push(format!(
                "library entry {} quarantined: identity-index collision",
                entry.library_id
            ));
            quarantined.push(raw);
        } else {
            healthy.push(entry);
        }
    }

    LoadedLibrary {
        healthy,
        quarantined,
        orphan_keys,
        degradations,
    }
}

/// A binding pubkey must be its own canonical `to_hex()` encoding: exactly 64
/// lowercase hex chars that parse as a valid curve point (§2.5). Production
/// writers only ever emit `to_hex()` output, so a non-canonical spelling is
/// hand-edited data and fails closed into the quarantine ladder. This is what
/// makes the RAW-string identity indexes in [`identity_collisions`] sound: one
/// agent key cannot survive under two different spellings, so a cross-owner
/// alias can never evade the document-wide check by re-casing its hex. Mirrors
/// the tree's `agent_snapshot_envelope::parse_canonical_pubkey` rule — including
/// its explicit `xonly()` curve validation, since nostr's `PublicKey::from_hex`
/// only decodes 32 bytes and defers lift-x, so a non-point like `"f" * 64` would
/// otherwise pass structurally — kept library-local so the error text is
/// domain-appropriate.
fn parse_canonical_binding_pubkey(field: &str, value: &str) -> Result<PublicKey, String> {
    if value.len() != 64
        || !value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(format!(
            "binding {field} {value} is not canonical (expected 64 lowercase hex chars)"
        ));
    }
    let pubkey = PublicKey::from_hex(value)
        .map_err(|_| format!("binding {field} {value} is not a valid pubkey"))?;
    pubkey
        .xonly()
        .map_err(|_| format!("binding {field} {value} is not a curve point"))?;
    Ok(pubkey)
}

/// Semantic binding validation (§2.5 read validation): every binding's auth tag
/// must verify against its `agent_pubkey` AND embed exactly the owner pubkey it
/// is keyed under. Also rejects an entry that binds one `agent_pubkey` under two
/// different owners. Non-canonical or malformed pubkeys and bad tags fail closed;
/// canonical encoding is required so the document-wide identity index can key on
/// raw strings safely (see [`parse_canonical_binding_pubkey`]).
fn validate_entry_bindings(entry: &LibraryEntry) -> Result<(), String> {
    let mut seen_agents: HashMap<String, String> = HashMap::new();
    for (owner_hex, binding) in &entry.identity_bindings {
        let owner = parse_canonical_binding_pubkey("owner", owner_hex)?;
        let agent = parse_canonical_binding_pubkey("agent", &binding.agent_pubkey)?;
        let embedded =
            buzz_sdk_pkg::nip_oa::verify_auth_tag(&binding.auth_tag, &agent).map_err(|e| {
                format!(
                    "binding auth tag for {} failed verification: {e}",
                    binding.agent_pubkey
                )
            })?;
        if embedded != owner {
            return Err(format!(
                "binding auth tag for {} embeds {} but is keyed under {owner_hex}",
                binding.agent_pubkey,
                embedded.to_hex()
            ));
        }
        if let Some(prev) = seen_agents.insert(binding.agent_pubkey.clone(), owner_hex.clone()) {
            return Err(format!(
                "agent {} is bound under two owners ({prev} and {owner_hex})",
                binding.agent_pubkey
            ));
        }
    }
    Ok(())
}

/// Build the document-wide identity index over the individually valid entries
/// and return the indices of every collider (§2.1 rule; P6-I3). Collisions on
/// (a) `library_id`, (b) live (non-`deleted`) [`OriginKey`], (c) a non-terminal
/// `(scope_id, local_slug)` projection claim, or (d) one `agent_pubkey` bound
/// under two DIFFERENT owners anywhere in the document group-quarantine ALL
/// participants — picking a winner would silently rewrite a scope or alias one
/// global keyring identity across owner authorities.
fn identity_collisions<'a>(
    entries: impl Iterator<Item = &'a LibraryEntry>,
) -> std::collections::HashSet<usize> {
    let mut by_library_id: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut by_origin: HashMap<&OriginKey, Vec<usize>> = HashMap::new();
    let mut by_scope_slug: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    // agent_pubkey → every (entry index, owner) it is bound under document-wide.
    let mut by_bound_agent: HashMap<&str, Vec<(usize, &str)>> = HashMap::new();

    for (index, entry) in entries.enumerate() {
        by_library_id
            .entry(entry.library_id.as_str())
            .or_default()
            .push(index);
        if !entry.deleted {
            by_origin.entry(&entry.origin).or_default().push(index);
        }
        for (scope_id, projection) in &entry.projections {
            if !projection.state.is_terminal() {
                by_scope_slug
                    .entry((scope_id.as_str(), projection.local_slug.as_str()))
                    .or_default()
                    .push(index);
            }
        }
        for (owner_hex, binding) in &entry.identity_bindings {
            by_bound_agent
                .entry(binding.agent_pubkey.as_str())
                .or_default()
                .push((index, owner_hex.as_str()));
        }
    }

    let mut colliders = std::collections::HashSet::new();
    for group in by_library_id
        .values()
        .chain(by_origin.values())
        .chain(by_scope_slug.values())
    {
        if group.len() > 1 {
            colliders.extend(group.iter().copied());
        }
    }
    // (d) Cross-owner identity aliasing (§2.5, spec lines 1765-1766): the
    // keyring is process-global by pubkey, so one `agent_pubkey` bound under two
    // different owners aliases a single agent identity across owner authorities.
    // Group-quarantine every entry that binds a colliding pubkey; same-owner
    // reuse across entries is explicitly permitted and never collides. (A single
    // entry binding one pubkey under two owners is already rejected in pass 1.)
    for occurrences in by_bound_agent.values() {
        let distinct_owners: std::collections::HashSet<&str> =
            occurrences.iter().map(|(_, owner)| *owner).collect();
        if distinct_owners.len() > 1 {
            colliders.extend(occurrences.iter().map(|(index, _)| *index));
        }
    }
    colliders
}

// ── §2.5 pure identity-binding helpers ────────────────────────────────────────

impl LibraryDocument {
    /// The §2.5 deletion-protection predicate (P13-C1, widened by P17-C1):
    /// answers, purely from `library.json`, whether the key for `agent_pubkey`
    /// must NEVER be deleted or its identity archived in v1. `true` iff ANY
    /// entry — live or tombstoned, any `deleted` value, any projection state —
    /// has EVER bound this pubkey, OR any outstanding `deferred_archives` row
    /// names it. Every removal path (direct delete, persona cascade, inbound
    /// kind:5, forced delete, §3.4/§3.5 cascades, the finalizer, every recovery
    /// point) MUST consult this for the record's pubkey before `delete_agent_key`
    /// or NIP-IA archival, regardless of the record's own linkage — the keyring
    /// is process-global by pubkey, so an unrelated plain carrier of a bound
    /// pubkey would otherwise destroy the binding's one global identity.
    ///
    /// Scans the RAW entries (not the decoded healthy set) so a QUARANTINED
    /// entry that names the pubkey still protects it: mis-deleting a live key is
    /// catastrophic and irreversible, while over-protecting only leaves a secret
    /// resident — the v1 posture is conservative by design. Ever-bound is
    /// observable forever because tombstoned entries and their binding records
    /// are kept forever (P1-OQ2).
    pub fn key_archive_protected(&self, agent_pubkey: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry_names_agent(entry, agent_pubkey))
    }
}

/// Whether one raw entry `Value` binds `agent_pubkey` in `identity_bindings` or
/// carries an outstanding `deferred_archives` row for it (§2.5). Field-name
/// exact matches on the raw JSON so a quarantined entry still counts.
fn entry_names_agent(entry: &Value, agent_pubkey: &str) -> bool {
    entry_binds_agent(entry, agent_pubkey)
        || entry
            .get("deferred_archives")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|r| r.get("agent_pubkey").and_then(Value::as_str) == Some(agent_pubkey))
            })
}

/// Whether one raw entry `Value` holds a live `identity_bindings` binding to
/// `agent_pubkey` (§2.5) — a deferred-archive marker does NOT count. This is the
/// "referenced by a live binding" test that keeps a committed key out of the
/// recovery reap; [`entry_names_agent`] widens it with deferred rows for the
/// deletion-protection predicate.
fn entry_binds_agent(entry: &Value, agent_pubkey: &str) -> bool {
    entry
        .get("identity_bindings")
        .and_then(Value::as_object)
        .is_some_and(|bindings| {
            bindings
                .values()
                .any(|b| b.get("agent_pubkey").and_then(Value::as_str) == Some(agent_pubkey))
        })
}

/// One linked instance's identity coordinates for §2.5 seed selection.
pub(crate) struct SeedCandidate<'a> {
    /// ISO-8601 UTC creation timestamp — same format for every record, so
    /// lexical order equals chronological order.
    pub created_at: &'a str,
    pub pubkey: &'a str,
}

/// Deterministic binding-seed selection (§2.5, P3-I2): among the linked
/// instances that exist at share/first-deploy time, the seed is the instance
/// with the earliest `created_at`, ties broken by the lowest pubkey — the
/// longest-lived identity collaborators are most likely to know. `None` is
/// legal ONLY when zero instances exist (no identity to carry yet; the first
/// deploy anywhere seeds the binding). The chosen pubkey is what carries across
/// the owner's scopes; the others keep their identities locally, unchanged.
///
/// Pure selection only; the caller performs the §2.5 mint/verify protocol on
/// the result inside the insertion transaction (P8-C2, Phase 4b).
pub(crate) fn select_binding_seed<'a>(
    instances: impl IntoIterator<Item = SeedCandidate<'a>>,
) -> Option<&'a str> {
    instances
        .into_iter()
        .min_by(|a, b| {
            a.created_at
                .cmp(b.created_at)
                .then_with(|| a.pubkey.cmp(b.pubkey))
        })
        .map(|winner| winner.pubkey)
}

/// Construct a verified [`IdentityBinding`] at commit time (§2.5 step 2, P4-I3).
/// The inverse of [`validate_entry_bindings`]: derive `agent_pubkey` from the
/// READ-BACK nsec (never the stored record — a stored `auth_tag` proves nothing
/// about this binding, and a pre-NIP-OA seed legitimately has none), then
/// compute a FRESH auth tag with the current owner keys. Deriving the pubkey
/// from the same secret that was keyring-verified guarantees the binding's
/// keyring entry backs exactly this pubkey, and computing the tag here
/// guarantees the embedded owner equals the map key (`owner_keys.public_key()`)
/// by construction — so the entry passes [`validate_entry_bindings`] on the
/// very next read with no special case for the legacy-`None` seed.
///
/// Returns `(owner_hex, binding)` — the caller inserts it as
/// `identity_bindings[owner_hex]` inside the insertion transaction (P8-C2,
/// Phase 4b). Pure: no keyring, no library IO. The nsec parse and the NIP-OA
/// self-attestation guard (owner == agent) are the only failure modes; both are
/// fail-closed `Err`. Passes `nostr` types straight into `buzz-sdk` exactly as
/// [`validate_entry_bindings`] does on the read side, so the constructed tag
/// verifies under the same code path.
pub(crate) fn build_identity_binding(
    owner_keys: &Keys,
    read_back_nsec: &str,
) -> Result<(String, IdentityBinding), String> {
    let agent_keys = Keys::parse(read_back_nsec.trim())
        .map_err(|e| format!("read-back nsec did not parse as a keypair: {e}"))?;
    let agent_pubkey = agent_keys.public_key();
    let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(owner_keys, &agent_pubkey, "")
        .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))?;

    Ok((
        owner_keys.public_key().to_hex(),
        IdentityBinding {
            agent_pubkey: agent_pubkey.to_hex(),
            auth_tag,
        },
    ))
}

/// A freshly minted identity, its keyring secret written and read-back
/// verified, ready for the atomic binding commit (§2.5 step 4, Phase 4b).
#[derive(Debug)]
pub(crate) struct MintedIdentity {
    /// The generated keypair — the caller deploys the instance under it.
    pub keys: Keys,
    /// Owner pubkey hex; the `identity_bindings` map key for `binding`.
    pub owner_hex: String,
    /// The verified binding to insert as `identity_bindings[owner_hex]`.
    pub binding: IdentityBinding,
}

/// Crash-safe fresh-identity mint order (§2.5 "deploy minting a fresh key",
/// P5-I1). Runs the only physically coherent ordering — a pubkey exists only
/// after its keypair — with a DURABLE orphan-journal checkpoint before the
/// secret touches the keyring, so every persisted secret has a prior durable
/// coordinate and no crash can strand an un-journaled key:
///
/// 1. generate the keypair in memory (nothing persisted anywhere);
/// 2. journal its derived pubkey to `orphan_keys` and DURABLY persist the
///    document via `persist` — the coordinate outlives a crash;
/// 3. write the nsec to the keyring and confirm it is DURABLY retrievable
///    (raw OS read-back via [`KeyStore::write_and_verify`], not a cache read);
/// 4. build the verified binding from the keyring's own read-back of the nsec
///    (never the in-memory copy) — proof the keyring entry backs exactly this
///    pubkey.
///
/// On `Ok`, the orphan row is left IN `document`; the caller commits `binding`
/// and drops the row in ONE atomic write (Phase 4b), so a crash before that
/// commit leaves the orphan unreferenced and its key is reaped at the next
/// recovery point (§4, [`unreferenced_orphans`]). On any `Err`, no keyring
/// secret exists without its durable orphan row already journaled: a failure at
/// step 2's `persist` wrote nothing to the keyring; a failure at step 3 leaves
/// the durable row for the reap.
///
/// `persist` is the durable step-2 write, injected so the ordering is
/// unit-testable at every crash point without real disk IO; production passes
/// `|doc| save_library_document(base_dir, doc)`. Keyring IO goes through
/// [`KeyStore`] for the same reason. Pure of any other side effect.
pub(crate) fn mint_bound_identity(
    document: &mut LibraryDocument,
    owner_keys: &Keys,
    store: &impl KeyStore,
    persist: impl FnOnce(&LibraryDocument) -> Result<(), String>,
) -> Result<MintedIdentity, String> {
    // (1) keypair in memory — no persistence anywhere yet.
    let agent_keys = Keys::generate();
    let agent_pubkey = agent_keys.public_key().to_hex();

    // (2) durable orphan journal BEFORE any secret is written. A failed persist
    // means nothing reached the keyring, so there is no un-journaled secret.
    document.journal_orphan_pubkey(&agent_pubkey);
    persist(document)?;

    // (3) keyring write + DURABLE read-back verify (raw OS round-trip, not a
    // cache read — see KeyStore::write_and_verify). A crash at or after this
    // leaves the durable orphan row (step 2) whose keyring entry the recovery
    // sweep reaps.
    let name = agent_keyring_name(&agent_pubkey);
    let nsec = agent_keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode minted nsec: {e}"))?;
    store.write_and_verify(&name, &nsec)?;

    // (4) build the binding from the keyring's read-back — proves the entry
    // backs exactly this pubkey. Sound because step 3 already confirmed the
    // value is durably retrievable, so this load reflects the verified backend
    // state; an absent value here is still fail-closed. The caller commits the
    // binding and removes the orphan row in one atomic write (Phase 4b).
    let read_back = store.load(&name)?.ok_or_else(|| {
        "minted key absent from keyring immediately after verified write".to_string()
    })?;
    let (owner_hex, binding) = build_identity_binding(owner_keys, &read_back)?;

    Ok(MintedIdentity {
        keys: agent_keys,
        owner_hex,
        binding,
    })
}

impl LibraryDocument {
    /// Journal a freshly minted pubkey to `orphan_keys` (§2.5 step 2) before its
    /// secret is written anywhere. Idempotent — a re-journal after a crashed
    /// retry is a no-op, never a duplicate row. Every persisted secret therefore
    /// has a prior durable orphan coordinate. Pure document mutation; the caller
    /// performs the atomic `save_library_document` (Phase 4b).
    pub fn journal_orphan_pubkey(&mut self, agent_pubkey: &str) {
        if !self.orphan_keys.iter().any(|k| k == agent_pubkey) {
            self.orphan_keys.push(agent_pubkey.to_string());
        }
    }

    /// Drop a pubkey's `orphan_keys` row (§2.5 step 4 / recovery). Called in the
    /// SAME atomic library write that commits the binding (the pubkey is now
    /// referenced by a live binding) or by the recovery sweep after its keyring
    /// entry is reaped. Pure; no-op when absent.
    pub fn remove_orphan_pubkey(&mut self, agent_pubkey: &str) {
        self.orphan_keys.retain(|k| k != agent_pubkey);
    }

    /// The `orphan_keys` rows that no binding references (§2.5 step 3 recovery).
    /// A crash between the orphan journal (step 2) and the binding commit (step
    /// 4) leaves a journaled pubkey whose keyring entry, if any, must be reaped;
    /// once reaped, the caller drops the row via [`remove_orphan_pubkey`]. Scans
    /// RAW entries for a live binding to the pubkey (binding only — a
    /// `deferred_archives` marker is not a live binding and must not keep an
    /// uncommitted orphan alive). Pure selection; [`reap_unreferenced_orphans`]
    /// performs the keyring delete + journal drop at the recovery point.
    pub fn unreferenced_orphans(&self) -> Vec<&str> {
        self.orphan_keys
            .iter()
            .filter(|orphan| !self.entries.iter().any(|e| entry_binds_agent(e, orphan)))
            .map(String::as_str)
            .collect()
    }
}

/// Reap every unreferenced orphan at a §4 recovery point (§2.5 step 3): for each
/// [`LibraryDocument::unreferenced_orphans`] pubkey, delete its keyring entry
/// THEN drop its journal row, and durably `persist` the trimmed document once.
///
/// Ordering is the crash-safety guarantee: the keyring `delete` precedes the
/// journal-row drop, and the row is only dropped for a pubkey whose delete
/// SUCCEEDED. A crash after a delete but before `persist` leaves the row intact,
/// so the next recovery point re-deletes (a no-op on the already-absent entry —
/// [`KeyStore::delete`] treats absent as success) and drops the row then; the
/// reap is idempotent. A backend `delete` failure keeps that pubkey's row for a
/// later retry and surfaces as `Err` AFTER the successful drops are persisted,
/// so partial progress is never lost. Referenced (committed) orphans are never
/// touched — their key backs a live binding.
///
/// `persist` and [`KeyStore`] are injected so the ordering is unit-testable at
/// each crash point without real IO; production passes the live secret store and
/// `|doc| save_library_document(base_dir, doc)`.
pub(crate) fn reap_unreferenced_orphans(
    document: &mut LibraryDocument,
    store: &impl KeyStore,
    persist: impl FnOnce(&LibraryDocument) -> Result<(), String>,
) -> Result<(), String> {
    let targets: Vec<String> = document
        .unreferenced_orphans()
        .into_iter()
        .map(str::to_string)
        .collect();

    let mut failures = Vec::new();
    for pubkey in &targets {
        match store.delete(&agent_keyring_name(pubkey)) {
            // Key gone (or already absent) — safe to drop the coordinate.
            Ok(()) => document.remove_orphan_pubkey(pubkey),
            // Backend failure: keep the row so a later recovery point retries.
            Err(e) => failures.push(format!("{pubkey}: {e}")),
        }
    }

    // Durably record the successful drops before reporting any failure, so a
    // partial reap never repeats work it already completed.
    persist(document)?;

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "reap left {} orphan(s) for retry: {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod binding_tests;
