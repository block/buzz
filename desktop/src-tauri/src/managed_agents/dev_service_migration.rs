//! Dev-build keyring secrets migration.
//!
//! Extracted from `storage.rs` to keep that module under the desktop
//! file-size ratchet. Copies secret-projection keys into the dev keyring
//! service on debug boots; a no-op in release builds.

use crate::app_state::keyring_service;
use tauri::Manager;

/// Marker key for the secrets migration to the dev keyring service.
/// Versioned separately from `_dev_migration_v1` (which covers only nsec keys)
/// so this migration runs even for installs that already completed v1.
#[cfg(debug_assertions)]
const DEV_SECRETS_MIGRATION_MARKER: &str = "_dev_secrets_migration_v2";

/// One-time migration of secret-projection keys (env vars, auth tags, provider
/// configs, definition env vars) from the source keyring service to the
/// destination dev service.
///
/// Called at dev-build boot, AFTER `migrate_agent_keys_to_dev_service` and
/// AFTER `migrate_inline_secrets_to_keyring` (so projection keys are present
/// in the source before we copy them).
///
/// Source determination:
/// - `buzz-desktop-dev` → source is `buzz-desktop` (production).
/// - `buzz-desktop-dev.<slug>` (scoped) → source is `buzz-desktop-dev`
///   (canonical dev), NOT production.
///
/// `global:env` is only copied when the destination's JSON reference requires
/// it (`global-agent-config.json` is not in `SHARED_AGENT_FILES`).
///
/// A coordinate present in both source and destination with different values
/// is a conflict; the migration fails closed for that coordinate and logs an
/// error rather than silently overwriting.
///
/// Idempotent: guarded by `DEV_SECRETS_MIGRATION_MARKER`; skips any key
/// already present in the destination.
#[cfg(debug_assertions)]
pub fn migrate_agent_secrets_to_dev_service(app: &tauri::AppHandle) {
    if !cfg!(feature = "system-keyring") {
        return;
    }
    let dest_service = keyring_service();
    if dest_service == "buzz-desktop" {
        return; // never run in a release build via this path
    }
    // Determine source: scoped dev → canonical dev; canonical dev → prod.
    let is_scoped = dest_service != "buzz-desktop-dev";
    let src_service = if is_scoped {
        "buzz-desktop-dev"
    } else {
        "buzz-desktop"
    };

    let dest_store = crate::secret_store::SecretStore::shared(dest_service);

    // Read destination blob.  If the v2 migration marker is present, we ran
    // this migration already — skip entirely.
    let dest_map: std::collections::HashMap<String, String> = match dest_store.load_all_readonly() {
        Ok(Some(map)) if map.contains_key(DEV_SECRETS_MIGRATION_MARKER) => {
            return; // already done
        }
        Ok(Some(map)) => map,
        Ok(None) => std::collections::HashMap::new(),
        Err(e) => {
            eprintln!(
                "buzz-desktop: keyring-dev-secrets-migration: \
                     cannot read dest keyring ({dest_service}): {e}"
            );
            return;
        }
    };

    let src_store = crate::secret_store::SecretStore::keyring(src_service);
    let src_map: std::collections::HashMap<String, String> = match src_store.load_all_readonly() {
        Ok(Some(map)) => map,
        Ok(None) => std::collections::HashMap::new(),
        Err(e) => {
            eprintln!(
                "buzz-desktop: keyring-dev-secrets-migration: \
                 cannot read src keyring ({src_service}): {e}"
            );
            return;
        }
    };

    // Determine which projection keys to copy: those present in src, absent
    // from dest, OR where src and dest agree (idempotent).  Keys present in
    // both with DIFFERENT values are conflicts — fail closed for that key.
    //
    // Exclude `global:env:*` from the copy unless the destination JSON
    // references it (global-agent-config.json is not a shared file).
    let global_refs = collect_global_env_refs(app);

    // Full blob coordinates referenced by canonical JSON. Used to prove a
    // conflict marker is safe to clear when its coordinate is no longer live.
    // `None` when either store is unreadable/malformed — in that case liveness
    // is unknown and only value-convergence can clear a marker (fail closed).
    let live_coords = collect_live_projection_coords(app);

    let DevMigrationPlan {
        to_write,
        conflict_keys,
        write_marker,
        conflict_markers_to_clear,
    } = plan_dev_secrets_migration(&src_map, &dest_map, &global_refs, live_coords.as_ref());

    for key in &conflict_keys {
        eprintln!(
            "buzz-desktop: keyring-dev-secrets-migration: \
             conflict on key {key} between {src_service} and {dest_service}; \
             refusing to overwrite and marking it unavailable until resolved \
             (manual resolution required)"
        );
    }
    let conflict_count = conflict_keys.len();

    // Clear resolved conflict markers first (store_all only inserts, so a
    // stale marker would otherwise linger and keep a now-healthy coordinate
    // unavailable forever). Best-effort: a failed clear just retries next boot.
    if !conflict_markers_to_clear.is_empty() {
        let keys: Vec<&str> = conflict_markers_to_clear
            .iter()
            .map(String::as_str)
            .collect();
        for key in &keys {
            if let Err(e) = dest_store.delete(key) {
                eprintln!(
                    "buzz-desktop: keyring-dev-secrets-migration: \
                     could not clear resolved conflict marker {key}: {e}"
                );
            }
        }
    }

    // Nothing to persist (no copyable keys, no conflict markers, and — being
    // unclean — no completion marker): skip the write entirely so an unclean
    // boot with zero copyable keys does not touch the destination keyring.
    if to_write.is_empty() {
        if conflict_count > 0 {
            eprintln!(
                "buzz-desktop: keyring-dev-secrets-migration: \
                 {conflict_count} conflict(s), nothing copyable; will retry next boot"
            );
        }
        return;
    }

    if let Err(e) = dest_store.store_all(&to_write) {
        eprintln!(
            "buzz-desktop: keyring-dev-secrets-migration: \
             cannot write to dest keyring ({dest_service}): {e}"
        );
        return;
    }

    // Subtract the completion marker and any conflict markers from the copied
    // count so only real projection copies are reported.
    let conflict_marker_count = conflict_keys.len();
    let copied = to_write.len() - usize::from(write_marker) - conflict_marker_count;
    if copied > 0 {
        eprintln!(
            "buzz-desktop: keyring-dev-secrets-migration: \
             copied {copied} projection key(s) from {src_service} → {dest_service}"
        );
    }
    if conflict_count > 0 {
        eprintln!(
            "buzz-desktop: keyring-dev-secrets-migration: \
             {conflict_count} key(s) had conflicts and were NOT copied; \
             marker withheld — migration will retry on the next boot"
        );
    }
}

/// Output of [`plan_dev_secrets_migration`]: the batch to write to the
/// destination keyring, the conflicting coordinates, whether the completion
/// marker is included, and the conflict markers to write/clear so a conflicted
/// coordinate is made unavailable (and cleared once it resolves).
#[cfg(debug_assertions)]
struct DevMigrationPlan {
    to_write: std::collections::HashMap<String, String>,
    conflict_keys: Vec<String>,
    write_marker: bool,
    /// `conflict:<coord>` marker keys to REMOVE this run: coordinates that
    /// previously carried a conflict marker whose conflict is now provably
    /// resolved — either source and destination converged on the same value, or
    /// the coordinate is proven no longer live in canonical JSON (nothing
    /// references it). A source that merely dropped the coordinate while the
    /// destination still holds the conflicting value does NOT clear the marker.
    /// Clearing the marker lets the coordinate hydrate normally again.
    conflict_markers_to_clear: Vec<String>,
}

/// Pure decision core of [`migrate_agent_secrets_to_dev_service`]: decide which
/// projection keys to copy, whether to write the completion marker, and which
/// conflict markers to write or clear.
///
/// A projection key present in both stores with DIFFERENT values is a conflict:
/// it is NOT copied and counts toward `conflict_count`. A `conflict:<coord>`
/// marker is written into the batch so `load_secret` fails closed for that
/// coordinate — the conflicted value must not be hydrated or consumed while
/// unresolved. The COMPLETION marker (`DEV_SECRETS_MIGRATION_MARKER`) is
/// included only when `conflict_count == 0`: an unclean run still writes its
/// non-conflicting keys AND the per-coordinate conflict markers (partial
/// progress persists), but withholds the completion marker so the migration is
/// retried on the next boot after the user resolves the conflict.
///
/// A coordinate that previously carried a conflict marker and no longer
/// conflicts is added to `conflict_markers_to_clear` so the availability block
/// is lifted once the conflict resolves.
///
/// `global:env:*` keys are copied only when the gen id is in `global_refs`
/// (the destination JSON references it) — `global-agent-config.json` is not a
/// shared file, so an unreferenced global gen must not leak across services.
#[cfg(debug_assertions)]
fn plan_dev_secrets_migration(
    src_map: &std::collections::HashMap<String, String>,
    dest_map: &std::collections::HashMap<String, String>,
    global_refs: &std::collections::HashSet<String>,
    live_coords: Option<&std::collections::HashSet<String>>,
) -> DevMigrationPlan {
    use crate::managed_agents::secret_projection::{conflict_marker_key, is_projection_key};

    let mut to_write: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut conflict_keys: Vec<String> = Vec::new();
    for (key, src_val) in src_map {
        if !is_projection_key(key) {
            continue; // skip non-projection keys (nsec, identity markers, etc.)
        }
        // Skip global:env:* unless the destination JSON references this gen.
        if key.starts_with("global:env:") {
            let gen = key.trim_start_matches("global:env:");
            if !global_refs.contains(gen) {
                continue;
            }
        }
        match dest_map.get(key) {
            None => {
                to_write.insert(key.clone(), src_val.clone());
            }
            Some(dest_val) if dest_val == src_val => {
                // Already identical — idempotent, no action needed.
            }
            Some(_dest_val) => {
                // Conflict: src and dest have different values.  Fail closed:
                // withhold the copy AND write a conflict marker so the
                // coordinate cannot be hydrated until the conflict resolves.
                conflict_keys.push(key.clone());
                to_write.insert(conflict_marker_key(key), "1".to_string());
            }
        }
    }

    // Clear a conflict marker ONLY when the conflict is provably resolved.
    // Two proofs qualify:
    //   1. Source and destination both hold the coordinate AND agree — the
    //      values converged, so the destination value can be trusted again.
    //   2. The coordinate is proven no longer live in canonical JSON — no
    //      record references it, so nothing will ever hydrate it and the
    //      two-cycle GC retires the orphaned generation.
    // Every other state RETAINS the marker and keeps the coordinate
    // unavailable (fail closed). In particular, source-absent while the
    // destination still holds the (conflicting) value and canonical JSON still
    // references the coordinate is NOT a resolution — a disappearing source is
    // not evidence that the destination value won. When liveness cannot be
    // proven (`live_coords` is `None` because a store was unreadable), only
    // proof #1 can clear a marker.
    let still_conflicting: std::collections::HashSet<&str> =
        conflict_keys.iter().map(String::as_str).collect();
    let conflict_markers_to_clear: Vec<String> = dest_map
        .keys()
        .filter(|k| k.starts_with(NS_CONFLICT_PREFIX))
        .filter(|marker| {
            let coord = marker.trim_start_matches(NS_CONFLICT_PREFIX);
            // A coordinate re-flagged as conflicting this run is also being
            // (re)written as a marker above — never clear it in the same pass.
            if still_conflicting.contains(coord) {
                return false;
            }
            let converged = matches!(
                (src_map.get(coord), dest_map.get(coord)),
                (Some(s), Some(d)) if s == d
            );
            let not_live = live_coords.is_some_and(|live| !live.contains(coord));
            converged || not_live
        })
        .cloned()
        .collect();

    let write_marker = conflict_keys.is_empty();
    if write_marker {
        to_write.insert(DEV_SECRETS_MIGRATION_MARKER.to_string(), "done".to_string());
    }

    DevMigrationPlan {
        to_write,
        conflict_keys,
        write_marker,
        conflict_markers_to_clear,
    }
}

/// The `conflict:` namespace prefix, mirrored from `secret_projection` so the
/// planner can recognize existing conflict markers in the destination blob.
#[cfg(debug_assertions)]
const NS_CONFLICT_PREFIX: &str = "conflict:";

/// Collect the set of generation IDs referenced by `global-agent-config.json`
/// for the purposes of the dev secrets migration (to decide whether to copy
/// `global:env:*` keys).  Returns an empty set when the file is absent or
/// unparseable — in that case no global env refs exist in the destination.
#[cfg(debug_assertions)]
fn collect_global_env_refs(app: &tauri::AppHandle) -> std::collections::HashSet<String> {
    let path = match app.path().app_data_dir() {
        Ok(d) => d.join("agents/global-agent-config.json"),
        Err(_) => return std::collections::HashSet::new(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return std::collections::HashSet::new(),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return std::collections::HashSet::new(),
    };
    let mut refs = std::collections::HashSet::new();
    if let Some(r) = v.get("env_vars_ref").and_then(|r| r.as_str()) {
        refs.insert(r.to_string());
    }
    refs
}

/// Collect the full blob coordinates referenced by the canonical JSON stores
/// (`managed-agents.json` + `global-agent-config.json`), reusing the same
/// [`collect_live_refs`] the GC uses so the liveness notion is identical.
///
/// Returns `None` when either store is unreadable or the JSON is in a state
/// `collect_live_refs` refuses to reason about (malformed, duplicate, or
/// inline+ref ambiguity). A `None` result must be treated as "liveness
/// unknown" — the marker-cleanup caller then declines to clear a marker on
/// liveness grounds and keeps the coordinate unavailable (fail closed).
#[cfg(debug_assertions)]
fn collect_live_projection_coords(
    app: &tauri::AppHandle,
) -> Option<std::collections::HashSet<String>> {
    use crate::managed_agents::secret_projection::collect_live_refs;

    let agents_path = crate::managed_agents::storage::managed_agents_store_path(app).ok()?;
    let global_path = app
        .path()
        .app_data_dir()
        .ok()?
        .join("agents/global-agent-config.json");

    // An absent file is a *known* empty liveness set, so default it to the
    // store's empty JSON shape (`[]` for the agents array, `{}` for the global
    // object). Only an unreadable *existing* file is genuinely unknown → `None`.
    let agents_json = read_json_or_default(&agents_path, "[]")?;
    let global_json = read_json_or_default(&global_path, "{}")?;

    collect_live_refs(&agents_json, &global_json).map(|live| live.coords)
}

/// Read a JSON store to a string, substituting `default_empty` for an absent
/// file. Returns `None` only when an existing file cannot be read — a genuine
/// "unknown" state the caller must fail closed on.
#[cfg(debug_assertions)]
fn read_json_or_default(path: &std::path::Path, default_empty: &str) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(default_empty.to_string()),
        Err(_) => None,
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use crate::managed_agents::secret_projection::{agent_env_key, global_env_key};
    use std::collections::{HashMap, HashSet};

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_clean_migration_includes_marker_and_copies() {
        // One projection key present only in src, no conflicts → copy it AND
        // write the marker (migration is complete).
        let key = agent_env_key("abc", "gen1");
        let src = map(&[(&key, "val")]);
        let plan = plan_dev_secrets_migration(&src, &HashMap::new(), &HashSet::new(), None);

        assert!(plan.conflict_keys.is_empty());
        assert!(plan.write_marker, "clean migration must write the marker");
        assert_eq!(plan.to_write.get(&key).map(String::as_str), Some("val"));
        assert!(plan.to_write.contains_key(DEV_SECRETS_MIGRATION_MARKER));
    }

    #[test]
    fn test_conflict_withholds_marker_but_copies_non_conflicting() {
        // Two keys: one conflicts (differs in dest), one is new. The migration
        // must copy the new key (partial progress) but WITHHOLD the marker so
        // the conflict is retried after manual resolution.
        let conflict = agent_env_key("abc", "gen1");
        let fresh = agent_env_key("def", "gen2");
        let src = map(&[(&conflict, "src-val"), (&fresh, "fresh-val")]);
        let dest = map(&[(&conflict, "dest-val")]);

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);

        assert_eq!(plan.conflict_keys, vec![conflict.clone()]);
        assert!(
            !plan.write_marker,
            "a conflicted migration must NOT write the marker — it must retry"
        );
        assert!(
            !plan.to_write.contains_key(DEV_SECRETS_MIGRATION_MARKER),
            "marker must be absent from the write batch on conflict"
        );
        assert_eq!(
            plan.to_write.get(&fresh).map(String::as_str),
            Some("fresh-val"),
            "non-conflicting key must still be copied for partial progress"
        );
        assert!(
            !plan.to_write.contains_key(&conflict),
            "conflicting key must NOT be overwritten"
        );
    }

    #[test]
    fn test_identical_key_is_idempotent_and_clean() {
        // A key already identical in dest is not re-copied, and (no conflict)
        // the marker is written.
        let key = agent_env_key("abc", "gen1");
        let src = map(&[(&key, "same")]);
        let dest = map(&[(&key, "same")]);

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);

        assert!(plan.conflict_keys.is_empty());
        assert!(plan.write_marker);
        assert!(
            !plan.to_write.contains_key(&key),
            "identical key must not be re-written"
        );
        // Only the marker is in the batch.
        assert_eq!(plan.to_write.len(), 1);
    }

    #[test]
    fn test_empty_source_writes_only_marker() {
        // Nothing to copy, no conflict → clean: the batch is just the marker.
        let plan =
            plan_dev_secrets_migration(&HashMap::new(), &HashMap::new(), &HashSet::new(), None);
        assert!(plan.write_marker);
        assert_eq!(plan.to_write.len(), 1);
        assert!(plan.to_write.contains_key(DEV_SECRETS_MIGRATION_MARKER));
    }

    #[test]
    fn test_unreferenced_global_env_is_skipped() {
        // A global:env gen not referenced by the destination JSON must not be
        // copied across services (global-agent-config.json is not shared).
        let key = global_env_key("gen-unref");
        let src = map(&[(&key, "val")]);
        let plan = plan_dev_secrets_migration(&src, &HashMap::new(), &HashSet::new(), None);

        assert!(
            !plan.to_write.contains_key(&key),
            "unreferenced global:env gen must be skipped"
        );
        // Clean (no conflict), so only the marker is present.
        assert!(plan.write_marker);
        assert_eq!(plan.to_write.len(), 1);
    }

    #[test]
    fn test_referenced_global_env_is_copied() {
        // The same global:env gen IS copied when the destination JSON
        // references it.
        let key = global_env_key("gen-ref");
        let src = map(&[(&key, "val")]);
        let refs: HashSet<String> = ["gen-ref".to_string()].into_iter().collect();
        let plan = plan_dev_secrets_migration(&src, &HashMap::new(), &refs, None);

        assert_eq!(plan.to_write.get(&key).map(String::as_str), Some("val"));
    }

    #[test]
    fn test_non_projection_key_is_ignored() {
        // A non-projection key (e.g. an nsec or identity marker) present only
        // in src must never be copied by this migration.
        let src = map(&[("some-agent-nsec-key", "secret")]);
        let plan = plan_dev_secrets_migration(&src, &HashMap::new(), &HashSet::new(), None);

        assert!(!plan.to_write.contains_key("some-agent-nsec-key"));
        assert!(plan.write_marker); // no projection conflict
        assert_eq!(plan.to_write.len(), 1); // marker only
    }

    #[test]
    fn test_conflict_writes_conflict_marker_into_batch() {
        // A conflicting coordinate must add a `conflict:<coord>` marker to the
        // write batch so `load_secret` fails closed for it — the F4 fix that
        // makes the conflicted value unavailable, not merely un-marked.
        use crate::managed_agents::secret_projection::conflict_marker_key;
        let conflict = agent_env_key("abc", "gen1");
        let src = map(&[(&conflict, "src-val")]);
        let dest = map(&[(&conflict, "dest-val")]);

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);

        assert_eq!(plan.conflict_keys, vec![conflict.clone()]);
        assert!(
            plan.to_write.contains_key(&conflict_marker_key(&conflict)),
            "a conflict must write its conflict marker into the batch"
        );
        assert!(
            !plan.to_write.contains_key(&conflict),
            "the conflicted value itself must NOT be copied"
        );
        assert!(!plan.write_marker, "completion marker withheld on conflict");
    }

    #[test]
    fn test_resolved_conflict_marker_is_cleared() {
        // A conflict marker present in dest whose coordinate no longer
        // conflicts (values now agree) must be scheduled for clearing so the
        // availability block is lifted.
        use crate::managed_agents::secret_projection::conflict_marker_key;
        let coord = agent_env_key("abc", "gen1");
        let marker = conflict_marker_key(&coord);
        // src and dest now AGREE on the coordinate — the conflict is resolved.
        let src = map(&[(&coord, "agreed")]);
        let dest = map(&[(&coord, "agreed"), (&marker, "1")]);

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);

        assert!(
            plan.conflict_keys.is_empty(),
            "coordinate no longer conflicts"
        );
        assert!(
            plan.conflict_markers_to_clear.contains(&marker),
            "a resolved conflict's stale marker must be cleared"
        );
    }

    #[test]
    fn test_persisting_conflict_marker_is_not_cleared() {
        // A conflict that STILL conflicts must keep its marker (not clear it),
        // so the coordinate stays unavailable across the retry.
        use crate::managed_agents::secret_projection::conflict_marker_key;
        let coord = agent_env_key("abc", "gen1");
        let marker = conflict_marker_key(&coord);
        let src = map(&[(&coord, "src-val")]);
        let dest = map(&[(&coord, "dest-val"), (&marker, "1")]);

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);

        assert_eq!(plan.conflict_keys, vec![coord.clone()]);
        assert!(
            !plan.conflict_markers_to_clear.contains(&marker),
            "a still-conflicting coordinate's marker must NOT be cleared"
        );
        assert!(
            plan.to_write.contains_key(&marker),
            "the (re)written marker keeps the coordinate unavailable"
        );
    }

    #[test]
    fn test_marker_retained_when_source_gone_but_dest_coord_still_live() {
        // F4: a disappearing SOURCE coordinate is not evidence the destination
        // won the conflict. If dest still holds the (conflicting) value AND
        // canonical JSON still references the coordinate, the marker must be
        // RETAINED so `load_secret` keeps failing closed. This is the fail-open
        // arm Thufir flagged: the old code cleared any marker whose coordinate
        // left the conflict set (source disappearance counted as resolution).
        use crate::managed_agents::secret_projection::conflict_marker_key;
        let coord = agent_env_key("abc", "gen1");
        let marker = conflict_marker_key(&coord);
        // Source no longer holds the coordinate; dest still holds the old value
        // plus the marker.
        let src = map(&[]);
        let dest = map(&[(&coord, "dest-val"), (&marker, "1")]);
        // Canonical JSON STILL references the coordinate's generation → live.
        let live: HashSet<String> = [coord.clone()].into_iter().collect();

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), Some(&live));

        assert!(
            plan.conflict_keys.is_empty(),
            "source is gone, so this run detects no fresh conflict"
        );
        assert!(
            !plan.conflict_markers_to_clear.contains(&marker),
            "marker must be RETAINED: source disappearance is not resolution while \
             the destination coordinate is still live"
        );
    }

    #[test]
    fn test_marker_cleared_when_coord_no_longer_live() {
        // F4 positive bound: once the coordinate is proven no longer referenced
        // by canonical JSON, nothing can hydrate it, so the stale marker is
        // safe to clear (the two-cycle GC retires the orphaned generation).
        use crate::managed_agents::secret_projection::conflict_marker_key;
        let coord = agent_env_key("abc", "gen1");
        let marker = conflict_marker_key(&coord);
        let src = map(&[]);
        let dest = map(&[(&coord, "dest-val"), (&marker, "1")]);
        // Canonical JSON references NO coordinates → coord is not live.
        let live: HashSet<String> = HashSet::new();

        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), Some(&live));

        assert!(
            plan.conflict_markers_to_clear.contains(&marker),
            "a marker whose coordinate is proven no longer live must be cleared"
        );
    }

    #[test]
    fn test_marker_retained_when_liveness_unknown_and_values_disagree() {
        // F4: when liveness cannot be proven (`live_coords` is None because a
        // store was unreadable) and the values still disagree, only value
        // convergence (proof #1) may clear a marker — so it is RETAINED.
        use crate::managed_agents::secret_projection::conflict_marker_key;
        let coord = agent_env_key("abc", "gen1");
        let marker = conflict_marker_key(&coord);
        let src = map(&[(&coord, "src-val")]);
        let dest = map(&[(&coord, "dest-val"), (&marker, "1")]);

        // liveness unknown → None. src/dest disagree AND this coord conflicts
        // again this run, so it is re-marked and never cleared.
        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);

        assert!(
            !plan.conflict_markers_to_clear.contains(&marker),
            "with liveness unknown and values disagreeing, the marker must be retained"
        );
    }

    // ── F4 end-to-end: a PLANNER-PRODUCED marker drives hydration refusal ────
    //
    // The other F4 tests hand-seed `conflict:<coord>` and prove `load_secret`
    // fails closed on it. This one closes the loop from the ORIGIN: run the
    // real planner on a conflicting src/dest, apply its `to_write` batch
    // verbatim (marker included) into a store, then hydrate a record pointing
    // at the conflicted generation. It proves the whole chain —
    // planner → keyring batch → hydration → spawn refusal — with no
    // hand-placed marker anywhere in the test.
    #[test]
    fn test_planner_conflict_marker_drives_hydration_refusal() {
        use crate::managed_agents::secret_projection::ProjectionStore;
        use crate::managed_agents::secret_seam::hydrate_all_secrets_for_records;
        use crate::managed_agents::storage::spawn_key_refusal;
        use crate::managed_agents::ManagedAgentRecord;
        use std::cell::RefCell;

        // Minimal in-memory store; hydration only reads via `load_key`.
        struct FakeStore(RefCell<HashMap<String, String>>);
        impl ProjectionStore for FakeStore {
            fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String> {
                self.0.borrow_mut().insert(key.into(), value.into());
                Ok(())
            }
            fn load_key(&self, key: &str) -> Result<Option<String>, String> {
                Ok(self.0.borrow().get(key).cloned())
            }
            fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
                Ok(Some(self.0.borrow().clone()))
            }
            fn store_batch(&self, entries: &HashMap<String, String>) -> Result<(), String> {
                for (k, v) in entries {
                    self.0.borrow_mut().insert(k.clone(), v.clone());
                }
                Ok(())
            }
            fn remove_batch(&self, keys: &[&str]) -> Result<(), String> {
                for k in keys {
                    self.0.borrow_mut().remove(*k);
                }
                Ok(())
            }
        }

        let pubkey = "abc";
        let coord = agent_env_key(pubkey, "gen_live");
        // Source and destination disagree on the same coordinate → conflict.
        let src = map(&[(&coord, r#"{"ANTHROPIC_API_KEY":"src-value"}"#)]);
        let dest = map(&[(&coord, r#"{"ANTHROPIC_API_KEY":"dest-value"}"#)]);

        // Run the REAL planner. It flags the conflict, withholds the completion
        // marker, and emits the `conflict:<coord>` marker in its write batch.
        let plan = plan_dev_secrets_migration(&src, &dest, &HashSet::new(), None);
        assert_eq!(plan.conflict_keys, vec![coord.clone()]);
        assert!(
            !plan.write_marker,
            "a conflict withholds the completion marker"
        );

        // Apply the planner's batch verbatim (the conflict marker rides along),
        // plus the destination value the migration left untouched — the exact
        // post-migration keyring state for an unresolved conflict.
        let store = FakeStore(RefCell::new(HashMap::new()));
        store.store_batch(&plan.to_write).unwrap();
        store
            .write_and_verify(&coord, r#"{"ANTHROPIC_API_KEY":"dest-value"}"#)
            .unwrap();

        // Hydrate a record that references the conflicted generation.
        let mut records = vec![{
            let mut r: ManagedAgentRecord = serde_json::from_str(&format!(
                r#"{{
                    "pubkey": "{pubkey}",
                    "name": "test-agent",
                    "private_key_nsec": "nsec1realkey",
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
            .expect("instance record");
            r.env_vars_ref = Some("gen_live".to_string());
            r
        }];

        let unavailable = hydrate_all_secrets_for_records(&store, &mut records);

        assert_eq!(
            unavailable,
            vec![pubkey.to_string()],
            "the planner's marker must surface the instance as unavailable"
        );
        assert!(
            records[0].secrets_unavailable,
            "a planner-produced conflict marker must set secrets_unavailable"
        );
        assert!(
            records[0].env_vars.is_empty(),
            "the untrusted destination value must NOT hydrate"
        );
        assert!(
            spawn_key_refusal(&records[0]).is_some(),
            "spawn must refuse the instance while the planner's conflict stands"
        );
    }
}
