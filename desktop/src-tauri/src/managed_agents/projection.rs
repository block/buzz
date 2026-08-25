//! Canonical roster projection.
//!
//! Pure, deterministic, side-effect-free projection of the local
//! `managed-agents.json` row set onto three buckets:
//! **Local Primary**, **Remote — Cross-Host Continuity**, and
//! **Unverified copies**. Behaviour and contract are defined in
//! `PLANS/CANONICAL_ROSTER_PROJECTION_SPEC_2026_08_22.md` (spec v0.2,
//! SHA-256 `14da368f021818dfec884e693f84443b26aca330ebf9c89ed1e1bfa96551820d`).
//!
//! The module is I/O-free by construction. Config loading is an explicit
//! caller step (see [`load_canonical_projection_config`]) so the projection
//! itself is trivially testable and can never write to disk.
//!
//! `#[allow(dead_code)]` is applied module-wide because this is a preview
//! feature landed behind `canonicalRosterProjection` (default-off). The
//! frontend consumes only [`get_canonical_projection_config`] at v0.5.18 —
//! the pure projection function, its input/output types, and the passthrough
//! helper exist so tests pin the algorithm now and future runtime call sites
//! (spec §5.4, §9 marathon) can adopt it without a second review pass. Remove
//! the module-level allow when the runtime wires the projection into the
//! roster-refresh path.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One row of input to the projection. The projection deliberately consumes
/// this minimal shape — not `ManagedAgentRecord` directly — so tests and
/// future runtime call sites can construct fixtures without touching the
/// 50-field record. Fields are public so callers can build a value with
/// the struct literal syntax; no conversion helper is required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionInputRow {
    pub pubkey: String,
    pub name: String,
}

/// A single canonical identity binding, mirroring one entry of
/// `canonical-projection.json` §3.1 `canonical`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalIdentity {
    pub identity: String,
    pub host: String,
    pub pubkey: String,
}

/// Deserialised `canonical-projection.json` — spec §3.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalProjectionConfig {
    pub version: u32,
    pub issued: String,
    pub revised: String,
    #[serde(default)]
    pub authorizing_events: Vec<String>,
    pub canonical: Vec<CanonicalIdentity>,
    /// Ordered priority list for display-name resolution. The projection
    /// consults `display_name_overrides` first, then any signed kind-0 name,
    /// then falls back to the canonical identity (or row name for unmatched
    /// rows). The field is here for spec fidelity and forward compatibility;
    /// the projection follows the priority regardless of what strings are
    /// listed, so an out-of-order list cannot silently change behaviour.
    #[serde(default)]
    pub display_name_source_priority: Vec<String>,
    /// Pubkey → forced display name. Wins over every other source.
    #[serde(default)]
    pub display_name_overrides: BTreeMap<String, String>,
}

/// A single card in the projected roster view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterCard {
    pub pubkey: String,
    pub display_name: String,
    /// The canonical identity string when this pubkey is on the frozen
    /// registry, else `None`.
    pub canonical_identity: Option<String>,
    /// `true` iff the caller supplied a live PID for this pubkey.
    pub live: bool,
}

/// A named canonical identity that the input rows did not cover on the
/// attested host. Emitted separately so the caller can surface it (WARN
/// log line per §3.2 step 6) without the projection ever synthesising a
/// missing pubkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingCanonical {
    pub identity: String,
    pub pubkey: String,
}

/// Projection output — spec §3.2 `RosterView`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterView {
    /// Canonical rows whose declared host matches the attested host, in
    /// canonical order.
    pub primary_local: Vec<RosterCard>,
    /// Canonical rows for other hosts (cross-host continuity), in
    /// canonical order.
    pub remote: Vec<RosterCard>,
    /// Every other input row (pubkey not in the canonical registry),
    /// sorted by pubkey ascending.
    pub unverified: Vec<RosterCard>,
    /// Canonical identities attested to this host that no input row
    /// covered. The projection never fabricates a pubkey for them.
    pub missing: Vec<MissingCanonical>,
}

impl RosterView {
    /// Passthrough view used when the feature flag is off. Every input row
    /// is emitted in `primary_local`, in input order, with liveness from
    /// `live_pids` and the row `name` as the display name. The other buckets
    /// stay empty; nothing is dropped or synthesised.
    pub fn passthrough(rows: &[ProjectionInputRow], live_pids: &HashMap<String, u32>) -> Self {
        let primary_local = rows
            .iter()
            .map(|row| RosterCard {
                pubkey: row.pubkey.clone(),
                display_name: row.name.clone(),
                canonical_identity: None,
                live: live_pids.contains_key(&row.pubkey),
            })
            .collect();
        RosterView {
            primary_local,
            remote: Vec::new(),
            unverified: Vec::new(),
            missing: Vec::new(),
        }
    }
}

/// Project the canonical roster.
///
/// Contract:
/// * `enabled == false` returns [`RosterView::passthrough`] verbatim.
/// * `primary_local` holds canonical rows for the attested host in
///   canonical order.
/// * `remote` holds canonical rows for other hosts in canonical order.
/// * `unverified` holds every other input row, sorted by pubkey ascending.
/// * `missing` names any canonical identity for this host with no matching
///   input row. Its `pubkey` field is copied straight from the config —
///   this function never derives a pubkey from any other input.
/// * Display-name precedence:
///     1. `config.display_name_overrides.get(pubkey)`
///     2. `signed_names.get(pubkey)`
///     3. canonical identity string when matched, else `row.name`.
pub fn project_canonical_roster(
    rows: &[ProjectionInputRow],
    signed_names: &HashMap<String, String>,
    live_pids: &HashMap<String, u32>,
    attested_host: &str,
    config: &CanonicalProjectionConfig,
    enabled: bool,
) -> RosterView {
    if !enabled {
        return RosterView::passthrough(rows, live_pids);
    }

    // Index canonical entries by pubkey and preserve canonical order.
    let canonical_by_pubkey: HashMap<&str, &CanonicalIdentity> = config
        .canonical
        .iter()
        .map(|c| (c.pubkey.as_str(), c))
        .collect();
    let canonical_order: HashMap<&str, usize> = config
        .canonical
        .iter()
        .enumerate()
        .map(|(idx, c)| (c.pubkey.as_str(), idx))
        .collect();

    let mut primary_local: Vec<RosterCard> = Vec::new();
    let mut remote: Vec<RosterCard> = Vec::new();
    let mut unverified: Vec<RosterCard> = Vec::new();
    let mut covered_canonical_pubkeys: BTreeSet<String> = BTreeSet::new();

    for row in rows {
        let display_name = resolve_display_name(
            &row.pubkey,
            &row.name,
            canonical_by_pubkey.get(row.pubkey.as_str()).copied(),
            signed_names,
            &config.display_name_overrides,
        );
        let live = live_pids.contains_key(&row.pubkey);
        match canonical_by_pubkey.get(row.pubkey.as_str()).copied() {
            Some(canonical) => {
                covered_canonical_pubkeys.insert(canonical.pubkey.clone());
                let card = RosterCard {
                    pubkey: row.pubkey.clone(),
                    display_name,
                    canonical_identity: Some(canonical.identity.clone()),
                    live,
                };
                if canonical.host == attested_host {
                    primary_local.push(card);
                } else {
                    remote.push(card);
                }
            }
            None => {
                unverified.push(RosterCard {
                    pubkey: row.pubkey.clone(),
                    display_name,
                    canonical_identity: None,
                    live,
                });
            }
        }
    }

    // Canonical order, then pubkey ASC as tiebreak — although canonical
    // pubkeys are unique, the stable sort makes the ordering explicit and
    // pins fixture snapshots against future reordering.
    primary_local.sort_by(|a, b| {
        let ai = canonical_order
            .get(a.pubkey.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let bi = canonical_order
            .get(b.pubkey.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        ai.cmp(&bi).then_with(|| a.pubkey.cmp(&b.pubkey))
    });
    remote.sort_by(|a, b| {
        let ai = canonical_order
            .get(a.pubkey.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let bi = canonical_order
            .get(b.pubkey.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        ai.cmp(&bi).then_with(|| a.pubkey.cmp(&b.pubkey))
    });
    // Unverified: stable-sorted by pubkey ASC (spec §6.1 unverified_stable_sort_by_pubkey_asc).
    unverified.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));

    let missing: Vec<MissingCanonical> = config
        .canonical
        .iter()
        .filter(|c| c.host == attested_host && !covered_canonical_pubkeys.contains(&c.pubkey))
        .map(|c| MissingCanonical {
            identity: c.identity.clone(),
            pubkey: c.pubkey.clone(),
        })
        .collect();

    RosterView {
        primary_local,
        remote,
        unverified,
        missing,
    }
}

fn resolve_display_name(
    pubkey: &str,
    row_name: &str,
    canonical: Option<&CanonicalIdentity>,
    signed_names: &HashMap<String, String>,
    overrides: &BTreeMap<String, String>,
) -> String {
    if let Some(override_name) = overrides.get(pubkey) {
        return override_name.clone();
    }
    if let Some(signed) = signed_names.get(pubkey) {
        return signed.clone();
    }
    canonical
        .map(|c| c.identity.clone())
        .unwrap_or_else(|| row_name.to_string())
}

/// Read a `canonical-projection.json` file from disk. Callers own I/O;
/// this helper exists so that both the Tauri command surface and tests
/// share the same JSON contract.
///
/// Returns `Err(String)` on missing file, invalid UTF-8, or JSON that does
/// not deserialise into [`CanonicalProjectionConfig`]. Never panics.
pub fn load_canonical_projection_config(path: &Path) -> Result<CanonicalProjectionConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read projection config at {}: {e}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse projection config at {}: {e}", path.display()))
}

/// Filesystem location of the projection config, relative to Tauri's
/// resolved `app_data_dir`. Matches spec §3.1 literally so the config
/// lives at `<app_data_dir>/roster/canonical-projection.json`.
const PROJECTION_SUBDIR: &str = "roster";
const PROJECTION_FILENAME: &str = "canonical-projection.json";

/// Tauri command — return the parsed projection config, or `None` when the
/// file is not present yet. Never writes; on parse failure the error is
/// surfaced to the frontend so bad config is loud rather than silently
/// falling back to a passthrough view.
///
/// The frontend uses this to look up per-pubkey display-name overrides on
/// the roster grid (spec §3.4). Called from a React hook gated on the
/// `canonicalRosterProjection` preview flag.
#[tauri::command]
pub fn get_canonical_projection_config(
    app: tauri::AppHandle,
) -> Result<Option<CanonicalProjectionConfig>, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join(PROJECTION_SUBDIR);
    let path = dir.join(PROJECTION_FILENAME);
    if !path.exists() {
        return Ok(None);
    }
    load_canonical_projection_config(&path).map(Some)
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
