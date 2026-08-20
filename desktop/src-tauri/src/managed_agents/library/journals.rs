//! Scope-local crash journals (§2.1, P9-I1). These share the OWNING
//! WORKSPACE's failure domain, not the library's: an unknown-version or corrupt
//! `library.json` must never block a purely workspace-local plain create,
//! import, or deploy, so `pending-agent-keys.json` and `deploy-intents.json`
//! live in `<agents-base>/scopes/<scope_id>/` (the scope's `definitions_dir`),
//! readable and writable whenever that scope is.
//!
//! `scope_id` is NOT stored in any row — it is the directory identity, exactly
//! as with `managed-agents.json`; rows can never migrate across scopes.
//!
//! Failure domains are ASYMMETRIC by operation class (P10-C3):
//! - `pending-agent-keys.json` unreadable → that scope's create/import paths
//!   degrade loudly (they cannot journal a fresh key); no other scope and no
//!   library operation is affected.
//! - `deploy-intents.json` unreadable OR semantically invalid (unknown version,
//!   syntax failure, a duplicate `agent_pubkey` row — the at-most-one mutex is
//!   VALIDATED on read and group-rejected, never first-wins — or a row whose
//!   `provider_config` fails `validate_provider_config`) → the scope's
//!   destructive removal and new-deploy paths FAIL CLOSED, because an unreadable
//!   journal may hide an intent equivalent to a live remote deployment, and an
//!   unvalidated row must never be exposed as authoritative routing.
//!
//! Unlike `library.json`, a corrupt journal is NOT copied to `.invalid`: both
//! failure modes above refuse to write, so the corrupt file is never
//! overwritten and needs no separate forensic snapshot. Absent = valid empty.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::managed_agents::storage::atomic_write_json_restricted;

/// The only journal schema version v1 understands (both files).
pub(crate) const SUPPORTED_JOURNAL_VERSION: u32 = 1;

/// `<scope>/pending-agent-keys.json` — pubkeys minted for a scoped record whose
/// keyring write may have outrun its JSON commit (§2.5 step (2), P8-I1).
pub(crate) fn pending_keys_path(definitions_dir: &Path) -> PathBuf {
    definitions_dir.join("pending-agent-keys.json")
}

/// `<scope>/deploy-intents.json` — the per-`agent_pubkey` deployment mutex and
/// durable phase machine (§3.3, P8-C1).
pub(crate) fn deploy_intents_path(definitions_dir: &Path) -> PathBuf {
    definitions_dir.join("deploy-intents.json")
}

// ── pending-agent-keys.json ───────────────────────────────────────────────────

/// Non-secret journal of agent pubkeys whose keyring entry may have no
/// committed record yet (§2.1). Reaped at the owning scope's next activation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingKeysJournal {
    pub version: u32,
    pub pending: Vec<String>,
}

impl PendingKeysJournal {
    pub fn empty() -> Self {
        Self {
            version: SUPPORTED_JOURNAL_VERSION,
            pending: Vec::new(),
        }
    }
}

/// Read outcome for `pending-agent-keys.json` (§2.1). `Unreadable` degrades the
/// owning scope's create/import paths loudly; no other scope is affected.
#[derive(Debug)]
pub(crate) enum PendingKeysLoad {
    /// Absent file — a valid empty journal.
    Empty,
    Loaded(PendingKeysJournal),
    /// Unknown version or syntax failure — degrade loudly.
    Unreadable(String),
}

/// Read and classify `pending-agent-keys.json` under `definitions_dir` (§2.1).
pub(crate) fn load_pending_keys(definitions_dir: &Path) -> PendingKeysLoad {
    let path = pending_keys_path(definitions_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PendingKeysLoad::Empty,
        Err(e) => return PendingKeysLoad::Unreadable(format!("read pending-agent-keys.json: {e}")),
    };
    let journal: PendingKeysJournal = match serde_json::from_slice(&bytes) {
        Ok(journal) => journal,
        Err(e) => {
            return PendingKeysLoad::Unreadable(format!("parse pending-agent-keys.json: {e}"))
        }
    };
    if journal.version != SUPPORTED_JOURNAL_VERSION {
        return PendingKeysLoad::Unreadable(format!(
            "pending-agent-keys.json version {} is not supported (expected {SUPPORTED_JOURNAL_VERSION})",
            journal.version
        ));
    }
    PendingKeysLoad::Loaded(journal)
}

/// Persist `pending-agent-keys.json`: atomic temp-file + rename + `0o600`.
pub(crate) fn save_pending_keys(
    definitions_dir: &Path,
    journal: &PendingKeysJournal,
) -> Result<(), String> {
    save_journal(&pending_keys_path(definitions_dir), journal)
}

// ── deploy-intents.json ───────────────────────────────────────────────────────

/// The per-`agent_pubkey` deployment-intent journal (§2.1). AT MOST ONE row per
/// pubkey — the invariant is VALIDATED on read (see [`load_deploy_intents`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeployIntentsJournal {
    pub version: u32,
    pub intents: Vec<DeployIntent>,
}

impl DeployIntentsJournal {
    pub fn empty() -> Self {
        Self {
            version: SUPPORTED_JOURNAL_VERSION,
            intents: Vec::new(),
        }
    }
}

/// One outstanding deploy attempt (§2.1). `provider_config` OWNS the routing the
/// possible remote lives under until the row is resolved (P13-I2); the canonical
/// record supplies only the fresh payload, never a reconstruction of the lost
/// attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeployIntent {
    /// Mutex key (`scope_id` is the file's directory).
    pub agent_pubkey: String,
    pub phase: DeployIntentPhase,
    /// The provider this attempt targets — the row owns routing until resolved.
    pub provider_id: String,
    /// Provider context (never secret; validated by `validate_provider_config`).
    pub provider_config: serde_json::Value,
    /// ISO timestamp, for degradation display.
    pub created_at: String,
}

/// Durable deploy phase machine (§2.1, P10-C1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum DeployIntentPhase {
    /// A deploy attempt owns this row; only the matching `attempt_id` may
    /// clear/resolve it (P9-C3). Recovery of an orphaned `Running` row is a
    /// residue clear or replay-as-recovery (§3.3 step 3).
    Running { attempt_id: String },
    /// The user explicitly accepted orphaning the possible remote (P10-C1).
    /// Journaled BEFORE any record destruction; fenced by `consent_id`.
    /// Recovery finishes the deletion — NEVER replays — from the row's own
    /// durable data plus the pre-journaled library obligation `cleanup` points
    /// to (P11-I1).
    OrphanConsentPendingRemoval {
        consent_id: String,
        cleanup: ConsentCleanup,
    },
}

/// The full per-pubkey cleanup policy a consented removal must durably produce,
/// captured inline from still-intact canonical state at consent time (§2.1,
/// P11-I1) — never re-derived from post-crash disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConsentCleanup {
    /// A 30177 tombstone must be durably retained for this pubkey.
    pub tombstone_required: bool,
    /// NIP-IA archive retained now (plain / unprotected pubkey).
    pub archive_required: bool,
    /// `library_id` of EVERY entry protecting this pubkey (§2.5 predicate) whose
    /// immediate archive was skipped — each row `DeferredArchive` journaled on
    /// its entry BEFORE this consent transition (obligation-first, universal —
    /// plain carriers included; P14-I1). Empty = no protection applied.
    pub deferred_archive_entries: Vec<String>,
    /// Exact cleanup coordinate for a projected instance; `None` for plain.
    pub projected: Option<ProjectedCleanupRef>,
}

/// The pre-journaled library obligation a projected consent points to (§2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectedCleanupRef {
    /// Owning entry.
    pub library_id: String,
    /// This scope's projection slug (the consenting scope and the projection's
    /// scope are the same scope by construction — the journal's directory).
    pub local_slug: String,
    /// Which durable library obligation this consent points to, written BEFORE
    /// the consent phase transition (§3.3 step 4 ordering) so recovery never
    /// guesses.
    pub obligation: ProjectedObligation,
}

/// The kind of projection obligation a consent owns (§2.1, P14-I1). Deferred
/// archives are NEVER named here — they live in
/// [`ConsentCleanup::deferred_archive_entries`] for plain and projected alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum ProjectedObligation {
    /// Projection is `ExcludePending`/`DeletePending`: the pubkey was merged
    /// into that `ProjectionEntry`'s `RemovalManifest` before consent.
    ManifestMembership,
    /// Projection is `Materialized` (direct forced delete): no removal manifest
    /// exists and none is owed.
    None,
}

/// Read outcome for `deploy-intents.json` (§2.1, P10-C3). `Unreadable` — unknown
/// version, syntax failure, OR a duplicate-pubkey integrity violation — fails
/// the owning scope's destructive and new-deploy paths CLOSED.
#[derive(Debug)]
pub(crate) enum DeployIntentsLoad {
    /// Absent file — a valid empty journal.
    Empty,
    /// A healthy version-1 journal with at most one row per pubkey.
    Loaded(DeployIntentsJournal),
    Unreadable(String),
}

/// Read and validate `deploy-intents.json` under `definitions_dir` (§2.1). The
/// at-most-one-per-pubkey mutex is validated on read: any duplicate pubkey
/// group-rejects the WHOLE journal (never first-wins), because an ambiguous
/// journal may hide an intent equivalent to a live remote deployment (P10-C3).
pub(crate) fn load_deploy_intents(definitions_dir: &Path) -> DeployIntentsLoad {
    let path = deploy_intents_path(definitions_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DeployIntentsLoad::Empty,
        Err(e) => return DeployIntentsLoad::Unreadable(format!("read deploy-intents.json: {e}")),
    };
    let journal: DeployIntentsJournal = match serde_json::from_slice(&bytes) {
        Ok(journal) => journal,
        Err(e) => return DeployIntentsLoad::Unreadable(format!("parse deploy-intents.json: {e}")),
    };
    if journal.version != SUPPORTED_JOURNAL_VERSION {
        return DeployIntentsLoad::Unreadable(format!(
            "deploy-intents.json version {} is not supported (expected {SUPPORTED_JOURNAL_VERSION})",
            journal.version
        ));
    }
    if let Some(pubkey) = duplicate_pubkey(&journal.intents) {
        return DeployIntentsLoad::Unreadable(format!(
            "deploy-intents.json has duplicate rows for {pubkey}: the at-most-one mutex is violated"
        ));
    }
    if let Err(reason) = validate_intent_routing(&journal.intents) {
        return DeployIntentsLoad::Unreadable(reason);
    }
    DeployIntentsLoad::Loaded(journal)
}

/// Persist `deploy-intents.json`: atomic temp-file + rename + `0o600`. Rejects
/// any row whose `provider_config` fails `validate_provider_config`, so a
/// crate-internal caller can never persist an invalid row that a later read
/// would refuse (§2.1 row contract; the read-side check would otherwise fail a
/// scope's destructive paths closed against state this process itself wrote).
pub(crate) fn save_deploy_intents(
    definitions_dir: &Path,
    journal: &DeployIntentsJournal,
) -> Result<(), String> {
    validate_intent_routing(&journal.intents)?;
    save_journal(&deploy_intents_path(definitions_dir), journal)
}

/// Validate every row's `provider_config` (§2.1: "validated by
/// `validate_provider_config`, never secret"). A malformed or secret-bearing
/// hand-edited row must never be exposed as authoritative routing.
fn validate_intent_routing(intents: &[DeployIntent]) -> Result<(), String> {
    for intent in intents {
        crate::managed_agents::backend::validate_provider_config(&intent.provider_config).map_err(
            |reason| {
                format!(
                    "deploy-intents.json row for {} has invalid provider_config: {reason}",
                    intent.agent_pubkey
                )
            },
        )?;
    }
    Ok(())
}

/// The first `agent_pubkey` that appears in more than one row, if any.
fn duplicate_pubkey(intents: &[DeployIntent]) -> Option<&str> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for intent in intents {
        *counts.entry(intent.agent_pubkey.as_str()).or_default() += 1;
    }
    intents
        .iter()
        .map(|intent| intent.agent_pubkey.as_str())
        .find(|pubkey| counts[pubkey] > 1)
}

/// Shared atomic `0o600` writer for both journals: create the parent scope dir
/// if needed, then temp-file + rename via `storage.rs`.
fn save_journal<T: Serialize>(path: &Path, journal: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create scope dir {}: {e}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(journal)
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    atomic_write_json_restricted(path, &payload)
}
