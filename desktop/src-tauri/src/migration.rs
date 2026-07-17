//! Worktree data sync and on-launch reconciliation for the Buzz desktop app.
//!
//! **Worktree sync** (`sync_shared_agent_data`): Per-launch symlink creation
//! from the current worktree data directory to the canonical dev data
//! directory (`xyz.block.buzz.app.dev`). Only runs when
//! `BUZZ_SHARE_IDENTITY=1` and `BUZZ_PRIVATE_KEY` is set. All dev
//! instances share the same physical files — edits in any worktree are
//! immediately visible to all others.
//!
//! **Command reconciliation** (`reconcile_legacy_command_names`): Per-launch
//! fix-up of persisted built-in command names from the Sprout→Buzz rename.
//!
//! **Provider reconciliation** (`reconcile_provider_mcp_commands`): Per-launch
//! fix-up of `mcp_command` values in `managed-agents.json` against the
//! discovery table. Ensures known providers always have their canonical
//! `mcp_command`; unknown/custom agents are left untouched.

use std::path::{Path, PathBuf};
use tauri::Manager;

use crate::util::replace_with_symlink;

const CANONICAL_DEV_IDENTIFIER: &str = "xyz.block.buzz.app.dev";
const LEGACY_CANONICAL_DEV_IDENTIFIER: &str = "xyz.block.sprout.app.dev";
const LEGACY_RELEASE_IDENTIFIER: &str = "xyz.block.sprout.app";

/// JSON files symlinked from worktree data directories to the canonical
/// dev data directory. Only data files — never `agent-pids/` or `logs/`.
/// `identity.key` is deliberately excluded because worktree instances
/// receive their identity via the `BUZZ_PRIVATE_KEY` env var.
const SHARED_AGENT_FILES: &[&str] = &[
    "agents/managed-agents.json",
    "agents/personas.json",
    "agents/teams.json",
];

/// Directories symlinked from worktree data directories to the canonical
/// dev data directory. Each entry becomes a single directory symlink.
const SHARED_AGENT_DIRS: &[&str] = &["agents/teams"];

/// Returns `true` when `name` is a dev data dir name — i.e. it is exactly the
/// canonical dev identifier or a worktree variant separated by a `.` (e.g.
/// `xyz.block.buzz.app.dev.my-branch`). Rejects prefix-collisions such as
/// `xyz.block.buzz.app.developer`. This is the authoritative dev/prod
/// discriminator shared by `run_boot_migrations`, `sync_shared_agent_data`,
/// and `reconcile_target_dir`.
pub(crate) fn is_dev_data_dir_name(name: &str) -> bool {
    name == CANONICAL_DEV_IDENTIFIER
        || name
            .strip_prefix(CANONICAL_DEV_IDENTIFIER)
            .is_some_and(|rest| rest.starts_with('.'))
}

fn canonical_dev_data_dir(current: &Path) -> Option<PathBuf> {
    current.parent().map(|p| p.join(CANONICAL_DEV_IDENTIFIER))
}

pub(crate) fn legacy_app_data_dir(current: &Path) -> Option<PathBuf> {
    let name = current.file_name()?.to_str()?;
    let legacy_name = if name.starts_with(CANONICAL_DEV_IDENTIFIER) {
        name.replacen(CANONICAL_DEV_IDENTIFIER, LEGACY_CANONICAL_DEV_IDENTIFIER, 1)
    } else if name.starts_with("xyz.block.buzz.app") {
        name.replacen("xyz.block.buzz.app", LEGACY_RELEASE_IDENTIFIER, 1)
    } else {
        return None;
    };
    current.parent().map(|parent| parent.join(legacy_name))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&src_path)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let target = std::fs::read_link(&src_path)?;
                if dst_path.exists() || dst_path.is_symlink() {
                    let _ = std::fs::remove_file(&dst_path);
                }
                crate::util::create_symlink(&target, &dst_path)?;
            }
            #[cfg(not(unix))]
            {
                continue;
            }
        } else if metadata.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if metadata.is_file() {
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !dst_path.exists() {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
    }
    Ok(())
}

/// Run every data migration that must complete before identity resolution and
/// agent restore. Ordering is load-bearing: `migrate_legacy_app_data_dir` must
/// precede any disk read, and `sync_shared_agent_data` must precede
/// `restore_managed_agents_on_launch` (which reads `managed-agents.json`).
/// Identity-dependent migrations (persona/team event signing) run separately in
/// boot setup after the persisted identity is resolved.
///
/// # Ordering
/// `sync_team_personas` is the sole writer of team-dir persona-runtime edits
/// into `personas.json`/`teams.json`; it MUST run before every reader of those
/// files. The pre-identity reader is `reconcile_provider_mcp_commands` (derives
/// `mcp_command` from each persona's effective harness); the post-identity
/// readers are `migrate_personas_to_events`/`migrate_teams_to_events` in
/// [`crate::event_sync::run_event_sync`]. Sync touches only JSON (no owner
/// keys, no `retention.db`), so it runs pre-identity here ahead of all
/// readers — reader-first loses a launch (stale harness/`mcp_command` until
/// the next boot).
pub fn run_boot_migrations(app: &tauri::AppHandle) {
    run_boot_migrations_inner(app, false);
}

/// Entry point when a completed reset must suppress dev-nest re-import.
pub fn run_boot_migrations_after_reset(app: &tauri::AppHandle) {
    run_boot_migrations_inner(app, true);
}

fn run_boot_migrations_inner(app: &tauri::AppHandle, reset_completed: bool) {
    // Initialize the process-lifetime nest directory before any filesystem
    // operation that calls nest_dir(). The discriminator matches the existing
    // pattern used by reconcile_target_dir: dev instances have an app-data-dir
    // name starting with CANONICAL_DEV_IDENTIFIER.
    let is_dev = if let Ok(data_dir) = app.path().app_data_dir() {
        let dev = data_dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_dev_data_dir_name);
        crate::managed_agents::init_nest_dir(dev);
        dev
    } else {
        false
    };

    // On dev builds, copy `.repos-dir` from ~/.buzz → ~/.buzz-dev BEFORE
    // control returns to lib.rs where resolve_repos_at_boot() reads it. This
    // ensures the dev nest boots with the correct workspace on its first launch,
    // matching what the prod nest had configured. Skip-if-dest-exists so it is
    // idempotent and never clobbers a value the dev nest already set explicitly.
    // Uses the composed helper so the gate + migration run through the same
    // code path that the behavioral test exercises.
    if let (Some(home), Some(dev_nest)) = (dirs::home_dir(), crate::managed_agents::nest_dir()) {
        maybe_migrate_dev_repos_dir(is_dev, reset_completed, &home, &dev_nest);
    }

    migrate_legacy_app_data_dir(app);
    sync_shared_agent_data(app);
    // Dev-build-only: copy any agent keys that exist in the production
    // keyring ("buzz-desktop") into the dev service ("buzz-desktop-dev")
    // so existing agents don't lose their keys after the service-name split.
    // Must run after sync_shared_agent_data (JSON symlinked) and before
    // any load_managed_agents call (which runs hydrate_keys against the
    // dev service and would log "has no key" for un-migrated entries).
    #[cfg(debug_assertions)]
    if is_dev {
        crate::managed_agents::migrate_agent_keys_to_dev_service(app);
    }
    migrate_persona_provider_to_runtime(app);
    reconcile_legacy_command_names(app);
    // Fold personas.json into the unified store HERE: after the JSON-level
    // personas.json migrations above (which must see the legacy file), and
    // before every consumer of the load/save_personas shims below —
    // sync_team_personas would otherwise operate on an empty definition set.
    // Post-fold readers of the runtime map (`load_persona_runtimes`) fall
    // back to the unified store's definitions.
    fold_personas_into_agent_store(app);
    // B5: manufacture definitions for standalone agents AFTER the fold (so
    // pre-existing definition slugs are present for collision checks) and
    // before event sync republishes — the backfilled link is what flips the
    // 30177 projection to its slim shape.
    backfill_standalone_agents(app);
    detach_directory_backed_teams(app);
    migrate_buzz_agent_to_goose(app);
    reconcile_provider_mcp_commands(app);
    reconcile_databricks_v1_to_v2(app);
    materialize_agent_runtimes(app);
}

/// Copy one-time app state from the legacy app identifier directory to
/// the current Buzz identifier directory. The Tauri identifier controls the app
/// data path, so without this copy a product rename would look like a fresh
/// install and users would lose their persisted identity and agent settings.
pub fn migrate_legacy_app_data_dir(app: &tauri::AppHandle) {
    let current_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("buzz-desktop: app-data-migration: cannot resolve app data dir: {e}");
            return;
        }
    };
    let Some(legacy_dir) = legacy_app_data_dir(&current_dir) else {
        return;
    };
    if !legacy_dir.exists() {
        return;
    }
    match copy_dir_all(&legacy_dir, &current_dir) {
        Ok(()) => eprintln!(
            "buzz-desktop: app-data-migration: copied legacy data from {} to {}",
            legacy_dir.display(),
            current_dir.display()
        ),
        Err(error) => eprintln!(
            "buzz-desktop: app-data-migration: failed to copy {} to {}: {error}",
            legacy_dir.display(),
            current_dir.display()
        ),
    }
}

/// Knowledge directories and files carried from the legacy nest into the live
/// nest. Deliberately excludes `REPOS/`: cloned repositories are re-clonable by
/// definition (Will's stranded `REPOS/` measured 62 GB of checkouts plus build
/// artifacts), so copying them would block desktop startup for minutes on every
/// cold launch while recovering nothing the agent "remembers". Agents re-clone
/// what they need into the live nest. The agent's accumulated knowledge — notes,
/// plans, logs — is what must survive the rename, and it totals a few hundred KB.
///
/// All entries are plain files or directories of plain files on the observed
/// disk, so `copy_dir_all`'s symlink branch is not exercised. This is a
/// content-dependent property, not a structural guarantee: `copy_dir_all`
/// recurses with `symlink_metadata`, so a symlink later dropped into one of
/// these dirs (e.g. by a skill writing into `.scratch/`) would hit that branch's
/// clobber/abort hazard. The per-entry log-and-continue below bounds the blast
/// radius of such a failure to the single offending entry.
const LEGACY_NEST_KNOWLEDGE: &[&str] = &[
    "AGENTS.md",
    "RESEARCH",
    "PLANS",
    "GUIDES",
    "WORK_LOGS",
    "OUTBOX",
    ".scratch",
];

/// Migrate the legacy agent nest (`~/.sprout`) into the current nest.
///
/// PR #960 renamed the nest directory but shipped no migration, stranding the
/// agent's accumulated knowledge in `~/.sprout` while `~/.buzz` booted empty —
/// so agents searched `$HOME` for files they "remembered", triggering macOS TCC
/// prompts. This copies only the knowledge directories (see
/// [`LEGACY_NEST_KNOWLEDGE`]), never `REPOS/`.
///
/// Non-fatal and idempotent, mirroring [`migrate_legacy_app_data_dir`]: a copy
/// error is logged and never aborts startup. There is no completion sentinel —
/// the migration re-runs on every launch while `~/.sprout` exists, which is
/// cheap because the copy is tiny and `copy_dir_all` skips files that already
/// exist in the destination. This relies on `REPOS/` being out of scope; if it
/// is ever added back, a sentinel or off-thread copy becomes mandatory.
///
/// Returns `true` when a legacy `~/.sprout` nest was present (migration ran),
/// so the caller can emit a one-time hint inviting the user to delete it. The
/// frontend dedupes the hint, so re-firing while `~/.sprout` lingers is benign.
pub fn migrate_legacy_nest() -> bool {
    let Some(home) = dirs::home_dir() else {
        eprintln!("buzz-desktop: nest-migration: cannot resolve home directory");
        return false;
    };
    // Destination is the current build's nest dir (`.buzz` or `.buzz-dev`).
    let Some(current_nest) = crate::managed_agents::nest_dir() else {
        eprintln!("buzz-desktop: nest-migration: cannot resolve nest directory");
        return false;
    };
    migrate_legacy_nest_at(&home.join(".sprout"), &current_nest)
}

/// Copy the [`LEGACY_NEST_KNOWLEDGE`] entries from `legacy` to `current`.
///
/// Each entry is copied independently with its own log-and-continue, so a
/// failure on one entry never skips the rest. No-ops cleanly when `legacy` is
/// absent or an entry does not exist. Returns `true` when `legacy` existed.
fn migrate_legacy_nest_at(legacy: &Path, current: &Path) -> bool {
    if !legacy.exists() {
        return false;
    }
    // A deliberate dev reset pre-creates this marker to opt out of every
    // production/legacy nest import. Normal first-run migration still copies
    // `.sprout` before `migrate_dev_nest()` writes the marker later in boot.
    if current.file_name().is_some_and(|name| name == ".buzz-dev")
        && current.join(DEV_NEST_MIGRATED_SENTINEL).exists()
    {
        return false;
    }
    for name in LEGACY_NEST_KNOWLEDGE {
        let src = legacy.join(name);
        if !src.exists() {
            continue;
        }
        let dst = current.join(name);
        let result = if src.is_dir() {
            copy_dir_all(&src, &dst)
        } else if *name == "AGENTS.md" {
            // `ensure_nest` writes a default `~/.buzz/AGENTS.md` before this
            // migration runs, so the plain absent-only guard would always skip
            // the legacy file and strand the user's instructions. Overwrite the
            // destination only when it is still the untouched generated default;
            // a user-edited file is left alone.
            copy_file_over_generated_default(&src, &dst)
        } else {
            copy_file_if_absent(&src, &dst)
        };
        match result {
            Ok(()) => eprintln!(
                "buzz-desktop: nest-migration: migrated {} to {}",
                src.display(),
                dst.display()
            ),
            Err(error) => eprintln!(
                "buzz-desktop: nest-migration: failed to migrate {} to {}: {error}",
                src.display(),
                dst.display()
            ),
        }
    }
    true
}

/// Filename of the completion sentinel written after a successful dev-nest
/// knowledge migration. Presence of this file means `~/.buzz` content has
/// already been copied into `~/.buzz-dev` and subsequent boots can skip the
/// copy. Using an explicit marker instead of checking for RESEARCH/PLANS
/// content decouples the dev migration from the `.sprout` migration, which
/// also copies into `~/.buzz-dev` and could otherwise set the sentinel early.
const DEV_NEST_MIGRATED_SENTINEL: &str = ".dev-nest-migrated";

/// Returns true when `migrate_dev_repos_dir` should run: dev build AND no
/// completed reset this boot (a completed reset means the dev nest was just
/// wiped — re-importing from prod would undo the sign-out).
pub(crate) fn should_migrate_dev_repos_dir(is_dev: bool, reset_completed: bool) -> bool {
    is_dev && !reset_completed
}

/// Injectable core: copy `.repos-dir` from `<home>/.buzz/` into `dev_nest`,
/// non-destructively. Extracted so tests can inject temp paths without
/// touching `dirs::home_dir()` or the global `nest_dir()` OnceLock.
pub(crate) fn migrate_dev_repos_dir_at(home: &Path, dev_nest: &Path) {
    let src = home.join(".buzz").join(".repos-dir");
    if !src.exists() {
        return;
    }
    // This is a one-time migration. The full dev-nest migration writes the
    // same sentinel, and a deliberate dev reset pre-creates it to keep the
    // next launch from importing production workspace state again.
    if dev_nest.join(DEV_NEST_MIGRATED_SENTINEL).exists() {
        return;
    }
    let dst = dev_nest.join(".repos-dir");
    // Skip if the dev nest already has its own .repos-dir.
    if dst.exists() {
        return;
    }
    // Ensure the dev nest directory itself exists — this migration runs before
    // ensure_nest() in the boot sequence, so the directory may not yet exist.
    if let Err(e) = std::fs::create_dir_all(dev_nest) {
        eprintln!(
            "buzz-desktop: dev-nest-migration: failed to create dev nest {}: {e}",
            dev_nest.display()
        );
        return;
    }
    match std::fs::copy(&src, &dst) {
        Ok(_) => eprintln!(
            "buzz-desktop: dev-nest-migration: migrated .repos-dir to {}",
            dst.display()
        ),
        Err(e) => eprintln!("buzz-desktop: dev-nest-migration: failed to migrate .repos-dir: {e}"),
    }
}

/// Composed gate + migration: applies `should_migrate_dev_repos_dir` and, if
/// the gate passes, runs the injectable migration core. This is the seam that
/// `run_boot_migrations_inner` calls with real paths — a future mis-wired flag
/// or inverted gate breaks tests at this level, not just the predicate.
pub(crate) fn maybe_migrate_dev_repos_dir(
    is_dev: bool,
    reset_completed: bool,
    home: &Path,
    dev_nest: &Path,
) {
    if should_migrate_dev_repos_dir(is_dev, reset_completed) {
        migrate_dev_repos_dir_at(home, dev_nest);
    }
}

/// One-time migration of dev-build nest contents from `~/.buzz` → `~/.buzz-dev`.
///
/// When a dev build first boots after this change ships, it switches from the
/// shared `~/.buzz` nest to a dedicated `~/.buzz-dev` nest. Without migration,
/// all accumulated knowledge (RESEARCH/, PLANS/, GUIDES/, WORK_LOGS/, mem_*
/// slugs, AGENTS.md, managed-agents.json) would be invisible to dev instances.
///
/// Migration is non-destructive: `copy_dir_all` skips files already at the
/// destination, so a partially-migrated state is safe to re-run. The source
/// `~/.buzz` is never deleted — prod builds continue to use it normally.
///
/// Completion is tracked by a [`DEV_NEST_MIGRATED_SENTINEL`] file written into
/// `~/.buzz-dev`. Using an explicit sentinel (rather than RESEARCH/PLANS file
/// presence) decouples this migration from the `.sprout` → `~/.buzz-dev`
/// migration that runs earlier in the same boot, which might otherwise populate
/// RESEARCH/PLANS and incorrectly suppress the `~/.buzz` copy.
///
/// Only runs on dev builds (checked by the caller). Returns `true` when
/// contents were copied (useful for a one-time log message, not required).
pub fn migrate_dev_nest() -> bool {
    let Some(home) = dirs::home_dir() else {
        eprintln!("buzz-desktop: dev-nest-migration: cannot resolve home directory");
        return false;
    };
    let legacy = home.join(".buzz");
    let current = home.join(".buzz-dev");
    // If legacy doesn't exist, nothing to migrate.
    if !legacy.exists() {
        return false;
    }
    // Skip if migration has already run (explicit sentinel, not content-based).
    if current.join(DEV_NEST_MIGRATED_SENTINEL).exists() {
        return false;
    }
    let copied = migrate_legacy_nest_at(&legacy, &current);
    // Write the sentinel so future boots skip the copy. Non-fatal if it fails
    // — worst case we re-run the (idempotent) migration on the next boot.
    if copied {
        let sentinel = current.join(DEV_NEST_MIGRATED_SENTINEL);
        if let Err(e) = std::fs::write(&sentinel, "") {
            eprintln!(
                "buzz-desktop: dev-nest-migration: failed to write sentinel {}: {e}",
                sentinel.display()
            );
        }
    }
    copied
}

/// Copy a single file only if the destination does not already exist, matching
/// `copy_dir_all`'s non-destructive guard for top-level files (e.g. `AGENTS.md`).
fn copy_file_if_absent(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst).map(|_| ())
}

/// Copy `src` over `dst` when `dst` is absent or still the untouched generated
/// default `AGENTS.md` (byte-equal to the embedded template). A user-edited
/// destination — or an older default left by a since-bumped template — is
/// preserved.
///
/// On a first-time migration `ensure_nest` has just written the generated
/// default, so `copy_file_if_absent` would always skip the legacy file and
/// strand the user's instructions. This lets the legacy `AGENTS.md` win over
/// that pristine default while never clobbering content a user has changed.
fn copy_file_over_generated_default(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        let current = std::fs::read_to_string(dst)?;
        if current != crate::managed_agents::AGENTS_MD {
            return Ok(());
        }
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst).map(|_| ())
}

/// Read a JSON array of objects from `path`, apply `f` to each object,
/// and write back if any mutation returned `true`.
///
/// Writes back via [`crate::managed_agents::atomic_write_json_restricted`]
/// (owner-only `0o600`): the store files this rewrites can carry plaintext
/// agent nsecs on a keyringless host, so the write must not reopen the umask
/// window SECURITY.md:90 closes.
fn patch_json_records(
    path: &Path,
    mut f: impl FnMut(&mut serde_json::Map<String, serde_json::Value>) -> bool,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut records) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
        eprintln!(
            "buzz-desktop: patch-json-records: failed to parse {}",
            path.display()
        );
        return;
    };
    let mut changed = false;
    for record in &mut records {
        if let Some(obj) = record.as_object_mut() {
            changed |= f(obj);
        }
    }
    if changed {
        if let Ok(bytes) = serde_json::to_vec_pretty(&records) {
            if let Err(e) = crate::managed_agents::atomic_write_json_restricted(path, &bytes) {
                eprintln!("buzz-desktop: patch-json-records: {e}");
            }
        }
    }
}

/// Create symlinks for shared agent data files from the current (worktree)
/// data directory to the canonical dev data directory.
///
/// Guards:
/// - `BUZZ_SHARE_IDENTITY` must be `"1"`
/// - `BUZZ_PRIVATE_KEY` must parse as valid `nostr::Keys`
/// - The canonical dir must differ from the current dir (skip if we ARE canonical)
/// - The canonical dir must exist
pub fn sync_shared_agent_data(app: &tauri::AppHandle) {
    // Guard: only runs when sharing identity with a worktree.
    let is_shared = std::env::var("BUZZ_SHARE_IDENTITY")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !is_shared {
        return;
    }

    // Guard: BUZZ_PRIVATE_KEY must be a valid nostr key.
    let has_valid_key = std::env::var("BUZZ_PRIVATE_KEY")
        .ok()
        .and_then(|k| k.parse::<nostr::Keys>().ok())
        .is_some();
    if !has_valid_key {
        eprintln!("buzz-desktop: shared-agent-sync: BUZZ_PRIVATE_KEY missing or invalid, skipping");
        return;
    }

    let current_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("buzz-desktop: shared-agent-sync: cannot resolve app data dir: {e}");
            return;
        }
    };

    // Guard: refuse to sync against a prod-identifier data directory, regardless
    // of env vars. A release build launched from an env-armed shell (e.g. macOS
    // `open` inherits the caller's env) must never overwrite real prod files with
    // symlinks. Only data dirs whose name starts with CANONICAL_DEV_IDENTIFIER
    // (the canonical dev dir and all worktree variants) are safe targets.
    let is_dev = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_dev_data_dir_name);
    if !is_dev {
        eprintln!(
            "buzz-desktop: shared-agent-sync: skipping — data dir is not a dev dir ({})",
            current_dir.display()
        );
        return;
    }

    let canonical_dir = match canonical_dev_data_dir(&current_dir) {
        Some(dir) => dir,
        None => {
            eprintln!("buzz-desktop: shared-agent-sync: cannot compute canonical dir (no parent)");
            return;
        }
    };

    // Guard: skip if we ARE the canonical instance.
    // Use canonicalize to handle case-insensitive FS and symlinks.
    let current_canonical =
        std::fs::canonicalize(&current_dir).unwrap_or_else(|_| current_dir.clone());
    let source_canonical =
        std::fs::canonicalize(&canonical_dir).unwrap_or_else(|_| canonical_dir.clone());
    if current_canonical == source_canonical {
        return;
    }

    // Guard: skip if canonical dir doesn't exist.
    if !canonical_dir.exists() {
        eprintln!(
            "buzz-desktop: shared-agent-sync: canonical dir does not exist: {}",
            canonical_dir.display()
        );
        return;
    }

    // Seed-up: if canonical is missing a shared file but a sibling instance
    // holds real (non-symlink) content, migrate it up to canonical before the
    // symlink loop runs. Mirrors the SHARED_AGENT_DIRS migration below, applied
    // to individual files. Without this, a fresh write in a worktree is never
    // promoted to canonical and gets clobbered by the symlink step.
    for rel in SHARED_AGENT_FILES {
        let canonical_file = canonical_dir.join(rel);
        if canonical_file.exists() {
            continue;
        }
        let Some(parent) = canonical_dir.parent() else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let sibling = entry.path();
            if sibling == canonical_dir {
                continue;
            }
            let sibling_file = sibling.join(rel);
            if sibling_file.is_file() && !sibling_file.is_symlink() {
                if let Some(file_parent) = canonical_file.parent() {
                    if let Err(e) = std::fs::create_dir_all(file_parent) {
                        eprintln!(
                            "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                            file_parent.display()
                        );
                        break;
                    }
                }
                let _ = std::fs::rename(&sibling_file, &canonical_file);
                eprintln!(
                    "buzz-desktop: shared-agent-sync: seeded {rel} from {}",
                    sibling.display()
                );
                break;
            }
        }
    }

    let mut synced = 0u32;
    for rel in SHARED_AGENT_FILES {
        let src = canonical_dir.join(rel);
        let dst = current_dir.join(rel);

        if !src.exists() {
            continue;
        }

        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                    parent.display()
                );
                continue;
            }
        }

        synced += replace_with_symlink(&src, &dst);
    }

    // Ensure shared directories exist in canonical before symlinking.
    // Packs may have been installed in a sibling instance (e.g., `.main`)
    // before shared-dir syncing existed — migrate them to canonical.
    for rel in SHARED_AGENT_DIRS {
        let canonical_target = canonical_dir.join(rel);
        if !canonical_target.exists() {
            if let Err(e) = std::fs::create_dir_all(&canonical_target) {
                eprintln!(
                    "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                    canonical_target.display()
                );
            }
            // Migrate from whichever sibling has real (non-symlink) content.
            if let Some(parent) = canonical_dir.parent() {
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let sibling = entry.path();
                        if sibling == canonical_dir {
                            continue;
                        }
                        let sibling_dir = sibling.join(rel);
                        if sibling_dir.is_dir() && !sibling_dir.is_symlink() {
                            if let Ok(children) = std::fs::read_dir(&sibling_dir) {
                                for child in children.flatten() {
                                    let dest = canonical_target.join(child.file_name());
                                    if !dest.exists() {
                                        let _ = std::fs::rename(child.path(), &dest);
                                    }
                                }
                            }
                            // Replace the sibling's dir with a symlink to canonical.
                            // replace_with_symlink backs up any leftover real content.
                            replace_with_symlink(&canonical_target, &sibling_dir);
                            eprintln!(
                                "buzz-desktop: shared-agent-sync: migrated {rel} from {}",
                                sibling.display()
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    for rel in SHARED_AGENT_DIRS {
        let src = canonical_dir.join(rel);
        let dst = current_dir.join(rel);

        if !src.exists() {
            continue;
        }

        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!(
                    "buzz-desktop: shared-agent-sync: failed to create {}: {e}",
                    parent.display()
                );
                continue;
            }
        }

        synced += replace_with_symlink(&src, &dst);
    }

    if synced > 0 {
        eprintln!(
            "buzz-desktop: shared-agent-sync: {synced} item(s) linked to {}",
            canonical_dir.display()
        );
    }
}

fn reconcile_mcp_commands_in_file(path: &Path) {
    // Resolve each record's EFFECTIVE harness (persona-wins, override-honored)
    // before deriving its mcp_command, so a persona-inherited harness switch
    // doesn't leave a stale persisted mcp_command. The persona runtime is read
    // from the sibling personas.json; missing entries fall back to the record's
    // own agent_command (the create-time snapshot).
    let persona_runtimes = load_persona_runtimes(path);
    patch_json_records(path, |obj| {
        let override_cmd = obj
            .get("agent_command_override")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let snapshot = obj.get("agent_command").and_then(|v| v.as_str());
        let persona_cmd = obj
            .get("persona_id")
            .and_then(|v| v.as_str())
            .and_then(|pid| persona_runtimes.get(pid))
            .map(String::as_str)
            .and_then(crate::managed_agents::known_acp_runtime_exact)
            .and_then(|r| r.commands.first().copied());
        let effective_command = match override_cmd.or(persona_cmd).or(snapshot) {
            Some(cmd) => cmd.to_string(),
            None => return false,
        };
        let Some(runtime) = crate::managed_agents::known_acp_runtime(&effective_command) else {
            return false;
        };
        let expected = runtime.mcp_command.unwrap_or("");
        let current = obj
            .get("mcp_command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if current == expected {
            return false;
        }
        // Only fix values that are clearly stale (empty or a removed binary).
        // Leave user-customized values untouched.
        if !current.is_empty() && current != "buzz-mcp-server" {
            return false;
        }
        eprintln!(
            "buzz-desktop: runtime-reconcile: {:?} ({:?}): mcp_command {:?} → {:?}",
            obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            effective_command,
            current,
            expected,
        );
        obj.insert(
            "mcp_command".to_string(),
            serde_json::Value::String(expected.to_string()),
        );
        true
    });
}

fn replace_command_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    replacement: String,
) -> bool {
    let Some(current) = obj.get(field).and_then(|v| v.as_str()) else {
        return false;
    };
    if current == replacement {
        return false;
    }
    eprintln!(
        "buzz-desktop: command-rename-reconcile: {:?}: {field} {:?} → {:?}",
        obj.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
        current,
        replacement,
    );
    obj.insert(field.to_string(), serde_json::Value::String(replacement));
    true
}

fn reconcile_legacy_command_names_in_file(path: &Path) {
    patch_json_records(path, |obj| {
        let mut changed = false;

        if let Some(acp_command) = obj
            .get("acp_command")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            if acp_command == "sprout-acp" {
                changed |= replace_command_field(obj, "acp_command", "buzz-acp".to_string());
            }
        }

        let mut agent_command = obj
            .get("agent_command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if agent_command == "sprout-agent" {
            agent_command = "buzz-agent".to_string();
            changed |= replace_command_field(obj, "agent_command", agent_command.clone());
        }

        if let Some(mcp_command) = obj
            .get("mcp_command")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            match mcp_command.as_str() {
                "sprout-dev-mcp" => {
                    changed |=
                        replace_command_field(obj, "mcp_command", "buzz-dev-mcp".to_string());
                }
                "sprout-mcp" | "sprout-mcp-server" | "buzz-mcp-server" => {
                    let replacement = if agent_command == "buzz-agent" {
                        "buzz-dev-mcp"
                    } else {
                        ""
                    };
                    changed |= replace_command_field(obj, "mcp_command", replacement.to_string());
                }
                _ => {}
            }
        }

        changed
    });
}

fn reconcile_legacy_persona_runtimes_in_file(path: &Path) {
    patch_json_records(path, |obj| {
        let Some(runtime) = obj.get("runtime").and_then(|v| v.as_str()) else {
            return false;
        };
        if runtime != "sprout-agent" {
            return false;
        }
        eprintln!(
            "buzz-desktop: command-rename-reconcile: persona {:?}: runtime {:?} → {:?}",
            obj.get("display_name")
                .or_else(|| obj.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
            runtime,
            "buzz-agent",
        );
        obj.insert(
            "runtime".to_string(),
            serde_json::Value::String("buzz-agent".to_string()),
        );
        true
    });
}

fn rewrite_legacy_persona_md_runtime(content: &str) -> Option<String> {
    let (frontmatter, body) = buzz_persona_pkg::persona::split_frontmatter(content).ok()?;
    let mut value = serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()?;
    let mapping = value.as_mapping_mut()?;
    let runtime = mapping.get_mut(serde_yaml::Value::String("runtime".to_string()))?;
    if runtime.as_str()? != "sprout-agent" {
        return None;
    }
    *runtime = serde_yaml::Value::String("buzz-agent".to_string());
    let frontmatter = serde_yaml::to_string(&value).ok()?;
    Some(format!("---\n{frontmatter}---\n{body}"))
}

fn reconcile_legacy_team_persona_runtime_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            reconcile_legacy_team_persona_runtime_files(&path);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".persona.md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(updated) = rewrite_legacy_persona_md_runtime(&content) else {
            continue;
        };
        if updated == content {
            continue;
        }
        match std::fs::write(&path, updated) {
            Ok(()) => {
                eprintln!(
                    "buzz-desktop: command-rename-reconcile: updated {}",
                    path.display()
                );
            }
            Err(error) => {
                eprintln!(
                    "buzz-desktop: command-rename-reconcile: failed to update {}: {error}",
                    path.display()
                );
            }
        }
    }
}

/// Reconcile exact built-in command values persisted before the Sprout→Buzz
/// rename. Custom commands and explicit paths are left untouched.
pub fn reconcile_legacy_command_names(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            reconcile_legacy_command_names_in_file(&path);
        }
        let personas_path = dir.join("agents/personas.json");
        if personas_path.exists() {
            reconcile_legacy_persona_runtimes_in_file(&personas_path);
        }
        let teams_dir = dir.join("agents/teams");
        if teams_dir.exists() && !teams_dir.is_symlink() {
            reconcile_legacy_team_persona_runtime_files(&teams_dir);
        }
    }
}

/// Reconcile `mcp_command` values in managed-agents.json against the
/// discovery table. Known runtimes get their canonical mcp_command;
/// unknown/custom agents are left untouched. Covers both the current
/// app data dir and the canonical dev data dir (for worktree instances).
pub fn reconcile_provider_mcp_commands(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            reconcile_mcp_commands_in_file(&path);
        }
    }
}

fn reconcile_databricks_v1_to_v2_in_file(path: &Path, rewrite_v1_provider: bool) {
    use crate::managed_agents::is_derived_provider_model_key;
    patch_json_records(path, |obj| {
        let mut changed = false;

        // Only rewrite the structured provider field when the baked build env
        // marks this as a Block build (BUZZ_AGENT_PROVIDER == "databricks_v2").
        // OSS users may intentionally select V1 (Model Serving), so we must not
        // silently migrate their provider to V2 (AI Gateway).
        if rewrite_v1_provider && obj.get("provider").and_then(|v| v.as_str()) == Some("databricks")
        {
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            eprintln!(
                "buzz-desktop: databricks-v1-to-v2: {name:?}: provider \"databricks\" → \"databricks_v2\"",
            );
            obj.insert(
                "provider".to_string(),
                serde_json::Value::String("databricks_v2".to_string()),
            );
            // Also clear the model field — a V1 model name (e.g. "dbrx-instruct")
            // on a V2 provider would shadow the baked DATABRICKS_MODEL at spawn time
            // (BUZZ_AGENT_MODEL from runtime_metadata_env_vars takes priority in
            // buzz-agent config.rs). Clearing it lets the baked V2 default win.
            if obj.remove("model").is_some() {
                eprintln!(
                    "buzz-desktop: databricks-v1-to-v2: {name:?}: cleared stale V1 model field",
                );
            }
            changed = true;
        }

        // Strip derived provider/model keys from env_vars on ALL records,
        // regardless of rewrite_v1_provider. These keys are re-derived from
        // structured fields at spawn time; stale copies in env_vars silently
        // override the structured fields (last-write-wins in Command::env) and
        // can cause V1 routing even when the provider dropdown shows V2.
        //
        // The check is case-insensitive (matching the established helper)
        // to cover any case-variant that may have been written historically.
        if let Some(serde_json::Value::Object(env_vars)) = obj.get_mut("env_vars") {
            let stale_keys: Vec<String> = env_vars
                .keys()
                .filter(|k| is_derived_provider_model_key(k))
                .cloned()
                .collect();
            for key in stale_keys {
                env_vars.remove(key.as_str());
                eprintln!("buzz-desktop: databricks-v1-to-v2: removed stale env_vars[\"{key}\"]",);
                changed = true;
            }
        }

        changed
    });
}

/// Strip stale derived provider/model keys from `env_vars` in all
/// managed-agent records, and — on Block builds — also migrate any persisted
/// `provider: "databricks"` to `"databricks_v2"`.
///
/// **Block builds** (where `baked_build_env()` contains
/// `BUZZ_AGENT_PROVIDER=databricks_v2`): the structured `provider` field is
/// rewritten V1→V2 because the baked release targets V2 exclusively. Records
/// that were saved before this migration would otherwise silently override the
/// baked value at spawn time (last-write-wins in `Command::env`).
///
/// **OSS builds** (baked env empty): the `provider` field is left alone —
/// V1 (`databricks`) is a valid Model Serving choice for OSS users.
///
/// In both cases, stale `BUZZ_AGENT_PROVIDER` / `BUZZ_AGENT_MODEL` /
/// `GOOSE_PROVIDER` / `GOOSE_MODEL` are stripped from `env_vars`. These keys
/// are always re-derived from structured fields at spawn time; persisted copies
/// silence UI edits and cause stale routing.
///
/// Covers both the current app data dir and the canonical dev data dir
/// (for worktree instances) — same dual-dir pattern as
/// `reconcile_legacy_command_names` and `reconcile_provider_mcp_commands`.
pub fn reconcile_databricks_v1_to_v2(app: &tauri::AppHandle) {
    use crate::managed_agents::baked_build_env;
    // On Block builds, the baked env contains BUZZ_AGENT_PROVIDER=databricks_v2.
    // Use that as a reliable signal that this is a Block build and the V1
    // provider should be migrated. OSS builds have an empty baked env, so
    // rewrite_v1_provider is false and the structured provider is preserved.
    let rewrite_v1_provider = baked_build_env()
        .get("BUZZ_AGENT_PROVIDER")
        .map(|v| v == "databricks_v2")
        .unwrap_or(false);
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            reconcile_databricks_v1_to_v2_in_file(&path, rewrite_v1_provider);
        }
    }
}

fn rename_provider_to_runtime_in_personas(path: &Path) {
    patch_json_records(path, |obj| {
        if obj.contains_key("runtime") {
            return false;
        }
        if let Some(value) = obj.remove("provider") {
            obj.insert("runtime".to_string(), value);
            true
        } else {
            false
        }
    });
}

pub fn migrate_persona_provider_to_runtime(app: &tauri::AppHandle) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let path = dir.join("agents/personas.json");
    if !path.exists() {
        return;
    }
    rename_provider_to_runtime_in_personas(&path);
}

mod materialize;
pub use materialize::materialize_agent_runtimes;
mod fold;
pub use fold::fold_personas_into_agent_store;
use fold::load_persona_runtimes;
mod backfill;
pub use backfill::backfill_standalone_agents;
mod detach;
pub use detach::detach_directory_backed_teams;
mod goose_default;
pub use goose_default::migrate_buzz_agent_to_goose;

#[cfg(test)]
#[path = "migration_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "migration_command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "migration_databricks_tests.rs"]
mod databricks_tests;

#[cfg(test)]
#[path = "migration_team_dir_tests.rs"]
mod team_dir_tests;

#[cfg(test)]
#[path = "migration_sync_guard_tests.rs"]
mod sync_guard_tests;
