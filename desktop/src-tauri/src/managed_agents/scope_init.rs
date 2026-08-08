//! Scope initialization state machine for the workspace agent definition store.
//!
//! Every scope directory is created exactly one way: staged install with a
//! durable manifest, followed by idempotent migrations, followed by a `Ready`
//! marker. Consumers may only open `Ready` scopes.
//!
//! # Claim ledger (F1 canonical family claim)
//!
//! One canonical claim covers the entire agent-store family (retention rows +
//! definitions). The legacy `retention.db`'s `retention_migrations` table is
//! the single source of truth:
//!
//! - If `retention.db` exists and already carries a `legacy_global_retention_db`
//!   claim naming scope A, then only scope A may adopt legacy definitions.
//!   A different scope B initializing first gets `LegacyClaimedByOther` — an
//!   empty, marked directory.
//! - If `retention.db` exists but is unclaimed, the first `apply_workspace`
//!   writes the claim into `retention.db`, then uses it for definitions too.
//! - If no `retention.db` exists, a canonical claim file is created under
//!   `<base_dir>/agents/legacy-claim.json` as a fallback ledger.
//!
//! # Staged install protocol
//!
//! 1. Determine manifest kind: `AdoptedLegacy`, `LegacyClaimedByOther`, or
//!    `FreshNoLegacy`.
//! 2. Build a staging directory (sibling to the target: `<target>._staging`).
//!    For `AdoptedLegacy`, copy (never move) legacy files into staging. For the
//!    other kinds, staging starts empty.
//! 3. Write the manifest JSON inside staging; fsync; single atomic rename of the
//!    staging directory into the target path. After rename, the target always
//!    carries its manifest.
//! 4. Run idempotent scoped migrations against the target.
//! 5. Write the separate `ready` marker file inside the target. Consumers check
//!    this marker before reading the store.
//!
//! # Restart / crash recovery
//!
//! - Target exists + `ready` marker present → scope is `Ready`, open normally.
//! - Target exists + no `ready` marker → installation started but migrations
//!   did not complete; resume migrations and write `ready`.
//! - Target does not exist → first activation; run the full staged install.
//! - A sibling `._staging` directory is an interrupted stage 2; clean it and
//!   restart from stage 2. The staging directory is rebuilt from the legacy
//!   source, so a retry never overwrites post-crash inbound/interactive writes
//!   that may have landed in a partial target before the rename.
//! - An artifact that cannot be produced by this state machine (no manifest)
//!   is quarantined: renamed to `<target>._quarantine_<timestamp>`.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

/// The claim name inside `retention_migrations` for the legacy definitions migration.
const DEFINITIONS_MIGRATION_NAME: &str = "legacy_global_retention_db";

/// File written inside the scope directory after all migrations complete.
const READY_MARKER: &str = "_ready";

/// Version written into the `_ready` marker file.
///
/// Increment this when the initialization pipeline gains new required steps
/// (retention migration, backfill, etc.). Any scope whose `_ready` file does
/// not contain this exact version string will be forced through the corrected
/// `run_pre_ready_family` pipeline before being considered fully ready.
///
/// History:
/// - v0 (absent / "ready"): marker written before retention migration and
///   persona backfill were added to `run_pre_ready_family`. Scopes at this
///   version may have incomplete retention or missing persona snapshots.
/// - v1: `run_pre_ready_family` (retention + backfill) runs before `_ready`;
///   Option-A Mesh preflight; marker contains this version string.
const READY_MARKER_VERSION: &str = "v1";

/// File written inside the scope directory (or staging) as the initialization manifest.
const MANIFEST_FILE: &str = "_manifest.json";

/// Fallback claim file when no `retention.db` exists.
const FALLBACK_CLAIM_FILE: &str = "legacy-claim.json";

/// The initialization kind recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeInitKind {
    /// This scope holds the canonical legacy claim and copied legacy definitions.
    AdoptedLegacy,
    /// The legacy claim exists and names a different scope; this scope starts empty.
    LegacyClaimedByOther { claiming_scope_id: String },
    /// No legacy definitions exist; this scope starts empty.
    FreshNoLegacy,
}

/// The manifest written inside a scope directory after staged install.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScopeManifest {
    pub scope_id: String,
    pub init_kind: ScopeInitKind,
}

/// Check whether a scope directory is already fully initialized at the current
/// pipeline version (has the `_ready` marker with the current version string).
///
/// Returns `false` for:
/// - Missing marker (never initialized, or crash before marker was written).
/// - Marker with an older version string (written by a prior pipeline that
///   lacked required steps such as retention migration or persona backfill).
///
/// Callers treat both cases the same: re-run `run_scoped_migrations` and
/// `run_pre_ready_family`, then write the updated marker.
pub fn scope_is_ready(scope_dir: &Path) -> bool {
    let marker_path = scope_dir.join(READY_MARKER);
    match std::fs::read_to_string(&marker_path) {
        Ok(content) => content.trim() == READY_MARKER_VERSION,
        Err(_) => false,
    }
}

/// Ensure a scope directory is fully initialized and `Ready`.
///
/// Idempotent: safe to call on every `apply_workspace`, even if the scope was
/// already initialized. Returns `Ok(())` when the scope is ready to use.
///
/// `owner_pubkey` is used for the legacy retention DB migration (copying
/// retained events for this owner from the legacy global DB into the scoped
/// DB). Pass the authenticated owner's hex pubkey.
pub fn ensure_scope_ready(
    scope_id: &str,
    scope_dir: &Path,
    base_dir: &Path,
    owner_pubkey: &str,
) -> Result<(), String> {
    if scope_is_ready(scope_dir) {
        return Ok(());
    }

    // Check for a staging directory left by a previous interrupted attempt.
    let staging_dir = staging_dir_for(scope_dir);
    if staging_dir.exists() {
        // Interrupted stage 2 — clean up and restart.
        std::fs::remove_dir_all(&staging_dir).map_err(|e| {
            format!(
                "failed to remove stale staging dir {}: {e}",
                staging_dir.display()
            )
        })?;
    }

    // If the target exists but has no manifest, it cannot have been produced by
    // this state machine — quarantine it.
    if scope_dir.exists() && !scope_dir.join(MANIFEST_FILE).exists() {
        quarantine_dir(scope_dir)?;
    }

    // If the target exists and already has a manifest, the staged install
    // completed (rename fired) but migrations or the ready marker were not
    // written before a crash. Skip re-staging and go straight to migrations —
    // re-staging would overwrite post-crash inbound/interactive writes that may
    // have landed in the target after the rename.
    if scope_dir.exists() && scope_dir.join(MANIFEST_FILE).exists() {
        run_scoped_migrations(scope_dir)?;
        run_pre_ready_family(scope_dir, base_dir, scope_id, owner_pubkey)?;
        write_ready_marker(scope_dir)?;
        return Ok(());
    }

    // Determine the manifest kind using the canonical claim ledger.
    let init_kind = resolve_init_kind(scope_id, base_dir)?;

    // Build the staged directory.
    install_staged(scope_id, scope_dir, base_dir, &init_kind)?;

    // Run idempotent scoped migrations.
    run_scoped_migrations(scope_dir)?;

    // Run pre-Ready family steps: legacy retention migration + persona backfill.
    // These must complete before _ready is written so a crash leaves the scope
    // in a retry-able state rather than permanently marking incomplete data Ready.
    run_pre_ready_family(scope_dir, base_dir, scope_id, owner_pubkey)?;

    // Write the ready marker.
    write_ready_marker(scope_dir)?;

    Ok(())
}

/// Resolve whether this scope should adopt legacy data, inherit another's
/// claim, or start fresh — using the canonical retention DB claim as the
/// single authority.
fn resolve_init_kind(scope_id: &str, base_dir: &Path) -> Result<ScopeInitKind, String> {
    let legacy_definitions_exist = legacy_definitions_exist(base_dir);

    if !legacy_definitions_exist {
        return Ok(ScopeInitKind::FreshNoLegacy);
    }

    // Consult the canonical retention DB claim.
    match read_or_create_canonical_claim(scope_id, base_dir)? {
        Some(claiming_scope_id) if claiming_scope_id == scope_id => {
            Ok(ScopeInitKind::AdoptedLegacy)
        }
        Some(claiming_scope_id) => Ok(ScopeInitKind::LegacyClaimedByOther { claiming_scope_id }),
        None => {
            // No claim exists and no retention DB to write into — this means
            // the fallback claim file was used and we successfully claimed.
            Ok(ScopeInitKind::AdoptedLegacy)
        }
    }
}

/// Read the canonical claim from `retention.db` (or the fallback claim file).
///
/// Returns:
/// - `Ok(Some(claiming_scope_id))` if a claim already exists — the caller
///   checks whether it matches.
/// - `Ok(None)` when we successfully wrote the claim for `scope_id` (caller
///   gets `AdoptedLegacy`).
/// - `Err` on I/O / DB failure.
fn read_or_create_canonical_claim(
    scope_id: &str,
    base_dir: &Path,
) -> Result<Option<String>, String> {
    let retention_db_path = base_dir.join("retention.db");
    if retention_db_path.exists() {
        return read_or_create_claim_in_retention_db(&retention_db_path, scope_id);
    }

    // No retention DB yet — use the fallback JSON claim file.
    // `base_dir` is already `<app-data>/agents`; the claim file lives
    // at `<app-data>/agents/legacy-claim.json` (no extra "agents" join).
    let claim_path = base_dir.join(FALLBACK_CLAIM_FILE);
    read_or_create_fallback_claim(&claim_path, scope_id)
}

/// Read or create the claim in `retention.db`'s `retention_migrations` table.
fn read_or_create_claim_in_retention_db(
    db_path: &Path,
    scope_id: &str,
) -> Result<Option<String>, String> {
    let conn =
        Connection::open(db_path).map_err(|e| format!("failed to open retention.db: {e}"))?;
    ensure_migration_table(&conn)?;

    // INSERT OR IGNORE so a concurrent process can't double-claim.
    conn.execute(
        "INSERT OR IGNORE INTO retention_migrations (name, scope_id) VALUES (?1, ?2)",
        params![DEFINITIONS_MIGRATION_NAME, scope_id],
    )
    .map_err(|e| format!("failed to write definition claim into retention.db: {e}"))?;

    // Read back who owns the claim.
    let claimed_by: Option<String> = conn
        .query_row(
            "SELECT scope_id FROM retention_migrations WHERE name = ?1",
            params![DEFINITIONS_MIGRATION_NAME],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("failed to read definition claim from retention.db: {e}"))?;

    Ok(claimed_by)
}

/// Read or create the fallback claim file (JSON) when no retention.db exists.
#[derive(serde::Serialize, serde::Deserialize)]
struct FallbackClaim {
    scope_id: String,
}

fn read_or_create_fallback_claim(
    claim_path: &Path,
    scope_id: &str,
) -> Result<Option<String>, String> {
    if claim_path.exists() {
        // Already claimed — read who owns it.
        let content = std::fs::read_to_string(claim_path)
            .map_err(|e| format!("failed to read fallback claim file: {e}"))?;
        let claim: FallbackClaim = serde_json::from_str(&content)
            .map_err(|e| format!("failed to parse fallback claim file: {e}"))?;
        return Ok(Some(claim.scope_id));
    }

    // Create the claim file atomically.
    if let Some(parent) = claim_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create agents dir for claim: {e}"))?;
    }
    let payload = serde_json::to_vec(&FallbackClaim {
        scope_id: scope_id.to_string(),
    })
    .map_err(|e| format!("failed to serialize fallback claim: {e}"))?;

    // Atomic write so a crash mid-write doesn't leave a partial claim.
    crate::managed_agents::storage::atomic_write_json(claim_path, &payload)?;

    // Return None to signal "we just claimed it" → AdoptedLegacy.
    Ok(None)
}

/// Check whether legacy (unscoped) definition files exist that need adoption.
///
/// `base_dir` is `<app-data>/agents` (not `<app-data>`). Legacy files live
/// directly under `base_dir`; the new scoped layout puts them under
/// `base_dir/scopes/<scope_id>/`. There is no extra "agents" join here.
fn legacy_definitions_exist(base_dir: &Path) -> bool {
    // Legacy layout: files live directly in `<app-data>/agents/`.
    // New layout puts files under `<app-data>/agents/scopes/<id>/`.
    base_dir.join("managed-agents.json").exists()
        || base_dir.join("teams.json").exists()
        || base_dir.join("global-agent-config.json").exists()
        || base_dir.join("personas.json").exists()
}

/// Build and atomically install the staged scope directory.
fn install_staged(
    scope_id: &str,
    scope_dir: &Path,
    base_dir: &Path,
    init_kind: &ScopeInitKind,
) -> Result<(), String> {
    let staging = staging_dir_for(scope_dir);

    // Clean any existing staging directory.
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("failed to clean staging dir {}: {e}", staging.display()))?;
    }
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("failed to create staging dir {}: {e}", staging.display()))?;

    // For AdoptedLegacy: copy legacy files into staging.
    if matches!(init_kind, ScopeInitKind::AdoptedLegacy) {
        // `base_dir` is `<app-data>/agents`; legacy files live directly in it.
        for filename in &[
            "managed-agents.json",
            "teams.json",
            "global-agent-config.json",
            "personas.json",
        ] {
            let src = base_dir.join(filename);
            if src.exists() {
                let dst = staging.join(filename);
                std::fs::copy(&src, &dst)
                    .map_err(|e| format!("failed to copy legacy {} to staging: {e}", filename))?;
            }
        }
    }

    // Write the manifest inside staging.
    let manifest = ScopeManifest {
        scope_id: scope_id.to_string(),
        init_kind: init_kind.clone(),
    };
    let manifest_payload = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("failed to serialize scope manifest: {e}"))?;
    std::fs::write(staging.join(MANIFEST_FILE), &manifest_payload)
        .map_err(|e| format!("failed to write scope manifest to staging: {e}"))?;

    // Fsync the staging directory to ensure durability before rename.
    // Best-effort: if fsync fails we proceed anyway (the rename is the atomic
    // boundary; a crash before fsync loses at most the staging data, not the
    // target).
    let _ = fsync_dir(&staging);

    // Atomic rename: staging → target. If the target already exists (a partial
    // installation from a previous crash that passed through quarantine), remove
    // it first.
    if scope_dir.exists() {
        std::fs::remove_dir_all(scope_dir)
            .map_err(|e| format!("failed to remove partial scope dir before rename: {e}",))?;
    }

    std::fs::rename(&staging, scope_dir).map_err(|e| {
        format!(
            "failed to atomically install scope dir (rename {} → {}): {e}",
            staging.display(),
            scope_dir.display()
        )
    })?;

    Ok(())
}

/// Run idempotent scoped migrations against an installed scope directory.
///
/// Returns `Err` on the FIRST step that fails so `ensure_scope_ready` can
/// withhold the `_ready` marker and preserve the retry gate. A partial
/// migration is better than permanently marking corrupt data as Ready.
///
/// This is the per-scope migration pipeline, run after staged install completes.
/// Ordering mirrors `migration.rs::run_boot_migrations_inner` for the
/// definition-touching steps (the ordering doc comment at `migration.rs:106-121`
/// is load-bearing). Machine-level steps that stay pre-scope (dir init, dev
/// symlinks) are NOT included here; persona-provider rename (step 0) IS included
/// and runs first so the fold reads the correct `runtime` field.
///
/// # Order (must be preserved)
/// 1. `fold_personas_in_dir` — fold personas.json into managed-agents.json.
///    BEFORE all readers of the unified store (strip, backfill, materialize).
/// 2. `strip_baked_team_instructions_in_dir` — clean legacy baked team-instructions
///    suffix AFTER fold (so lifted definitions are also cleaned) and BEFORE
///    backfill (so manufactured definitions never snapshot the suffix).
/// 3. `refresh_builtin_agent_avatars_at` — refresh legacy builtin avatars.
/// 4. `backfill_standalone_agents_in_dir` — manufacture definitions for standalone
///    agents AFTER fold (slug collision checks see pre-existing definitions).
/// 5. `detach_directory_backed_teams_in_dir` — lift pack instructions, clear
///    source_dir on teams.
/// 6. `reconcile_legacy_command_names_at` — fix stale command names.
/// 7. `reconcile_provider_mcp_commands_at` — fix mcp_command values.
/// 8. `reconcile_databricks_v1_to_v2_at` — V1→V2 provider migration.
/// 9. `materialize_agent_runtimes_at` — materialize runtime onto each record.
/// 10. Validate managed-agents.json is parseable JSON before writing Ready.
fn run_scoped_migrations(scope_dir: &Path) -> Result<(), String> {
    // Step 0: rename `provider` → `runtime` in personas.json before fold so
    // the fold reads the correct `runtime` field. The pre-scope call in
    // `run_boot_migrations_inner` is removed; this step is the canonical
    // location for the persona-provider rename.
    crate::migration::migrate_persona_provider_to_runtime_at(scope_dir)
        .map_err(|e| format!("scope-init-persona-provider: {e}"))?;

    // Step 1: fold personas.json into the unified store.
    match crate::migration::fold_personas_in_dir(scope_dir) {
        Ok(None) | Ok(Some(0)) => {}
        Ok(Some(n)) => {
            eprintln!("buzz-desktop: scope-init-fold: {n} definitions folded into scoped store");
        }
        Err(e) => return Err(format!("scope-init-fold: {e}")),
    }

    // Step 2: strip baked team-instructions suffix.
    match crate::migration::strip_baked_team_instructions_in_dir(scope_dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("buzz-desktop: scope-init-strip: {n} records cleaned"),
        Err(e) => return Err(format!("scope-init-strip: {e}")),
    }

    // Step 3: refresh legacy builtin agent avatars.
    crate::migration::refresh_builtin_agent_avatars_at(scope_dir)
        .map_err(|e| format!("scope-init-avatars: {e}"))?;

    // Step 4: backfill standalone agents into definition-linked records.
    match crate::migration::backfill_standalone_agents_in_dir(scope_dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("buzz-desktop: scope-init-backfill: {n} agents backfilled"),
        Err(e) => return Err(format!("scope-init-backfill: {e}")),
    }

    // Step 5: detach directory-backed teams.
    match crate::migration::detach_directory_backed_teams_in_dir(scope_dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("buzz-desktop: scope-init-detach: {n} teams detached"),
        Err(e) => return Err(format!("scope-init-detach: {e}")),
    }

    // Step 6: reconcile legacy command names.
    crate::migration::reconcile_legacy_command_names_at(scope_dir)
        .map_err(|e| format!("scope-init-cmd-names: {e}"))?;

    // Step 7: reconcile provider mcp_command values.
    crate::migration::reconcile_provider_mcp_commands_at(scope_dir)
        .map_err(|e| format!("scope-init-mcp-cmds: {e}"))?;

    // Step 8: Databricks V1 → V2 provider migration.
    crate::migration::reconcile_databricks_v1_to_v2_at(scope_dir)
        .map_err(|e| format!("scope-init-databricks: {e}"))?;

    // Step 9: materialize runtime onto each record.
    crate::migration::materialize_agent_runtimes_at(scope_dir)
        .map_err(|e| format!("scope-init-materialize: {e}"))?;

    // Step 10: validate the final managed-agents.json is parseable JSON before
    // writing the Ready marker. The step-10 backstop ensures the file is still
    // valid JSON even after all migrations have run successfully.
    let agents_path = scope_dir.join("managed-agents.json");
    if agents_path.exists() {
        let content = std::fs::read_to_string(&agents_path)
            .map_err(|e| format!("scope-init-validate: failed to read managed-agents.json: {e}"))?;
        serde_json::from_str::<serde_json::Value>(&content).map_err(|e| {
            format!(
                "scope-init-validate: managed-agents.json is not valid JSON after migrations: {e}"
            )
        })?;
    }

    Ok(())
}

/// Run the pre-Ready family steps: legacy retention migration and persona
/// snapshot backfill. These must complete before the `_ready` marker is
/// written so a crash between migration and marker leaves the scope in a
/// retry-able state rather than permanently marking incomplete data Ready.
///
/// Both steps are idempotent: a second run after a crash is safe.
/// A failure aborts the pre-Ready sequence and propagates to `ensure_scope_ready`,
/// which withholds the `_ready` marker, enabling a clean retry on next launch.
fn run_pre_ready_family(
    scope_dir: &Path,
    base_dir: &Path,
    scope_id: &str,
    owner_pubkey: &str,
) -> Result<(), String> {
    // Step A: legacy retention migration — copy owned retained events from
    // the legacy global retention.db into this scope's scoped DB.
    let scope_db_path = base_dir.join("retention").join(format!("{scope_id}.db"));
    if let Some(parent) = scope_db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("scope-init-retention: failed to create retention dir: {e}"))?;
    }
    match crate::managed_agents::retention::migrate_legacy_retention_db(
        base_dir,
        &scope_db_path,
        owner_pubkey,
    ) {
        Ok(0) => {}
        Ok(copied) => eprintln!(
            "buzz-desktop: scope-init-retention: adopted {copied} legacy retained event(s)"
        ),
        Err(e) => return Err(format!("scope-init-retention: {e}")),
    }

    // Step B: persona snapshot backfill — pre-populate `persona_source_version`
    // on instances that link a persona but have no version pinned yet, so
    // auto-start agents boot from a valid snapshot even on first activation.
    // Runs without the store lock because the scope is not yet published as
    // _ready and no concurrent reader or writer can legally access it.
    if let Err(e) = crate::managed_agents::restore::backfill_persona_snapshots_pre_ready(scope_dir)
    {
        return Err(format!("scope-init-backfill: {e}"));
    }

    // Step C (debug builds only, non-test): copy agent keys from the prod keyring into
    // the dev service. Runs after staged copy so the scoped managed-agents.json
    // exists with valid pubkeys. Replaces the pre-scope call in
    // `run_boot_migrations_inner` which could not read the store before scope
    // activation.
    //
    // Skipped in unit tests (`#[cfg(not(test))]`) because the real keychain
    // backend can block on macOS (waiting for a Keychain access dialog).
    // The migration itself is covered by storage_tests.rs with a FakeKeyStore.
    #[cfg(all(debug_assertions, not(test)))]
    crate::managed_agents::storage::migrate_agent_keys_to_dev_service_at(scope_dir)
        .map_err(|e| format!("scope-init-dev-keys: {e}"))?;

    Ok(())
}

/// Write the `_ready` marker file inside the scope directory, signaling that
/// all migrations are complete and the scope is available for use.
///
/// Writes `READY_MARKER_VERSION` so future pipeline upgrades can detect and
/// re-run scopes initialized by an older pipeline.
///
/// Uses an atomic temp-file + rename so a crash mid-write cannot leave a
/// partial/corrupt marker that `scope_is_ready` would misread.
fn write_ready_marker(scope_dir: &Path) -> Result<(), String> {
    let marker_path = scope_dir.join(READY_MARKER);
    // Write to a sibling temp file first, then rename atomically.
    let tmp_path = {
        let mut s = marker_path.as_os_str().to_owned();
        s.push("._tmp");
        PathBuf::from(s)
    };
    std::fs::write(&tmp_path, READY_MARKER_VERSION.as_bytes()).map_err(|e| {
        format!(
            "failed to write ready marker temp at {}: {e}",
            tmp_path.display()
        )
    })?;
    std::fs::rename(&tmp_path, &marker_path).map_err(|e| {
        // Best-effort cleanup of the temp file.
        let _ = std::fs::remove_file(&tmp_path);
        format!(
            "failed to atomically install ready marker at {}: {e}",
            marker_path.display()
        )
    })
}

/// Compute the staging directory path for a given scope directory.
/// Convention: `<scope_dir>._staging` (sibling, not child, to keep rename atomic).
fn staging_dir_for(scope_dir: &Path) -> PathBuf {
    let mut s = scope_dir.as_os_str().to_owned();
    s.push("._staging");
    PathBuf::from(s)
}

/// Quarantine an unrecognized scope directory by renaming it to a timestamped
/// path. Best-effort: if the rename fails, we proceed anyway.
fn quarantine_dir(scope_dir: &Path) -> Result<(), String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut quarantine = scope_dir.as_os_str().to_owned();
    quarantine.push(format!("._quarantine_{ts}"));
    let quarantine_path = PathBuf::from(quarantine);
    std::fs::rename(scope_dir, &quarantine_path).map_err(|e| {
        format!(
            "failed to quarantine unrecognized scope dir {} → {}: {e}",
            scope_dir.display(),
            quarantine_path.display()
        )
    })
}

/// Best-effort fsync of a directory (to flush its metadata to disk).
fn fsync_dir(path: &Path) -> std::io::Result<()> {
    let f = std::fs::File::open(path)?;
    f.sync_all()
}

/// Ensure the `retention_migrations` table exists (same DDL as in
/// `retention/legacy_migration.rs`).
fn ensure_migration_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS retention_migrations (
            name TEXT PRIMARY KEY,
            scope_id TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("failed to create retention migration table: {e}"))
}

#[cfg(test)]
#[path = "scope_init_tests.rs"]
mod tests;
