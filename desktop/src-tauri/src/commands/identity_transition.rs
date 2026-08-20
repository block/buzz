//! Identity-transition coordinator (P25-C1): the shared sink both runtime
//! identity-swap callers (normal import and phone recovery) route through, split
//! from `identity.rs` (file-size guard). Owns lock-ordering, the journaled
//! managed-agent runtime drain, the fenced pre-commit gate, and scope clear.

use crate::app_state::AppState;
use crate::models::IdentityInfo;
use tauri::Manager;

/// Drain live managed-agent runtimes for identity import (Layer 2 protocol).
/// Caller must hold `managed_agent_runtime_transition`. Returns stopped entries
/// or `Err((stopped, msg))` on failure.
fn drain_managed_agent_runtimes_for_import<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
) -> Result<
    Vec<crate::managed_agents::DrainJournalEntry>,
    (Vec<crate::managed_agents::DrainJournalEntry>, String),
> {
    let (stopped, _remaining, drain_error) =
        crate::managed_agents::drain_scope_runtimes(app, state);
    match drain_error {
        None => Ok(stopped),
        Some(e) => Err((stopped, e)),
    }
}

/// Compensate a drained runtime set with NO durable identity write — the shared
/// unwind for every barrier abort (drain failure, fenced-gate/journal failure,
/// `DefinitelyUnchanged`). The store guard is dropped FIRST because
/// [`compensate_drain`](crate::managed_agents::compensate_drain) re-acquires it,
/// and the runtime-transition guard is passed BY VALUE so compensation runs
/// without any interleave window. `scope`/`rt_guard` are `None` on the no-scope
/// path (nothing drained); either being `None` skips compensation. Returns a
/// combined diagnostic string when compensation itself fails.
fn compensate_import_drain<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
    stopped: &[crate::managed_agents::DrainJournalEntry],
    scope: Option<&crate::managed_agents::scope::WorkspaceAgentScope>,
    rt_guard: Option<std::sync::MutexGuard<'_, ()>>,
    store_guard: Option<std::sync::MutexGuard<'_, ()>>,
    reason: &str,
) -> String {
    // Drop the store lock before compensating (compensate_drain re-acquires it).
    drop(store_guard);
    let comp_err = match (scope, rt_guard) {
        (Some(scope), Some(rt_guard)) => {
            crate::managed_agents::compensate_drain(app_handle, stopped, scope, rt_guard)
        }
        (_, leftover_guard) => {
            drop(leftover_guard);
            None
        }
    };
    match comp_err {
        Some(comp) => format!("{reason}; compensation failed: {comp}"),
        None => reason.to_string(),
    }
}

/// The shared identity-transition coordinator (P25-C1): the ONE sink both
/// runtime identity-swap callers route through. Acquires the transition locks
/// in ONE unconditional order — `identity_mutation` → `workspace_transition`
/// (ALWAYS, incl. Mesh preflight when the feature is active) — then runs the
/// journaled drain + fenced commit in [`import_identity_blocking`], which
/// decides the active/no-scope branch ONLY from a scope snapshot taken UNDER
/// the held transition guard (P26-C1). Taking `workspace_transition` even on
/// the no-scope path closes the `None → Some` activation race: a concurrent
/// `apply_workspace` cannot commit a new active scope between this
/// coordinator's scope sample and its durable commit, because both serialize
/// on `workspace_transition`. No deadlock: `apply_workspace` takes only
/// `workspace_transition` (never `identity_mutation`), so the global order
/// `identity_mutation` → `workspace_transition` has no cycle.
///
/// `commit_fence` + `late_validity_check` are threaded to the pre-commit
/// boundary: normal import supplies `None` + always-`Ok`; the phone-recovery
/// continuation supplies the pairing `generation_fence` + its generation-current
/// check, so a superseded recovery compensates the drain and commits NO identity
/// while a recovery that wins the fence commits durably (P26-C1).
///
/// # Two validity boundaries (P26-C1)
///
/// The transition is gated at TWO points, each closing a distinct supersession
/// window; a single check cannot cover both because they straddle the drain:
///
/// - **`early_validity_check`** runs under the held `workspace_transition`
///   guard, immediately after the locks are acquired and BEFORE the Mesh
///   preflight and drain. Its job is to reject a task that was already
///   superseded/cancelled while it queued on the locks, doing ZERO disruptive
///   work — no drain, no Mesh state change, no egress-barrier bump, nothing to
///   compensate. Normal import supplies always-`Ok`; pairing supplies the
///   task-currency check.
/// - **`late_validity_check`** runs inside [`import_identity_blocking`] at the
///   fence-held pre-commit boundary — after a successful drain, immediately
///   before the durable dispatch — so a supersession that lands during the
///   drain window is caught before any identity is committed (the drained
///   runtimes are then compensated). This is the boundary the `commit_fence`
///   makes indivisible against a racing invalidation.
///
/// A cancellation that lands AFTER the early gate but DURING the drain reaches
/// the runtime revoke before `late_validity_check` rejects it; the drained
/// runtimes compensate, but revoked owner-identity durable capabilities stay
/// revoked. This is design-conformant fail-closed churn, not a split state —
/// see `CROSS_WORKSPACE_AGENT_LIBRARY.md` §3.3a (barrier sequence is fixed).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_identity_transition<R, S>(
    app_handle: tauri::AppHandle<R>,
    nsec: String,
    password: Option<String>,
    commit_fence: Option<std::sync::Arc<std::sync::Mutex<()>>>,
    store: &'static S,
    data_dir: std::path::PathBuf,
    early_validity_check: impl FnOnce() -> Result<(), String>,
    late_validity_check: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<IdentityInfo, String>
where
    R: tauri::Runtime,
    S: crate::app_state::IdentityKeyStore + Send + Sync + 'static,
{
    // ── Layer 1: identity_mutation (async serialization lock) ────────────────
    // Held for the full import to prevent a concurrent stale persist from
    // overwriting the imported key. Lock order: identity_mutation →
    // workspace_transition (UNCONDITIONALLY — P26-C1).
    //
    // Use a cloned handle for lock acquisition so the original `app_handle` is
    // free for the spawned blocking body below (no borrow conflict).
    let lock_handle = app_handle.clone();
    let lock_state = lock_handle.state::<AppState>();
    let _mutation_guard = lock_state.identity_mutation.lock().await;

    // ── Layer 1b: workspace_transition (UNCONDITIONAL) ───────────────────────
    // Always route through the transition lock, whether or not a scope appears
    // active — the active/no-scope decision is made INSIDE the blocking body
    // from a snapshot taken under this guard, so a concurrent apply_workspace
    // can neither slip a `None → Some` activation past the scope sample nor
    // race the durable commit (P26-C1).
    let _transition_guard = lock_state.workspace_transition.lock().await;

    // ── Early validity gate (P26-C1) ─────────────────────────────────────────
    // Under the held transition guard, BEFORE the Mesh preflight and drain: a
    // task superseded while queued on the locks is rejected here having done
    // ZERO disruptive work (no drain, no Mesh, no egress-barrier change).
    early_validity_check()?;

    // ── Mesh preflight (UNCONDITIONAL, no-op without `mesh-llm`) ──────────────
    // Fail closed if a client-mode Mesh runtime is active. Runs under the held
    // `workspace_transition` guard — the same orchestration invariant
    // `apply_workspace` relies on.
    #[cfg(feature = "mesh-llm")]
    crate::commands::mesh_llm::scope_impl::run_mesh_transition_preflight(&app_handle).await?;

    let app_for_body = app_handle.clone();
    let result = tokio::task::spawn_blocking(move || {
        import_identity_blocking(
            app_for_body,
            nsec,
            password,
            commit_fence,
            store,
            data_dir,
            late_validity_check,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    // Both transition guards must outlive spawn_blocking — drop explicitly here
    // so the compiler can see their lifetimes cover the blocking body.
    drop(_transition_guard);
    drop(_mutation_guard);

    result
}

/// Run `commit` under the transition commit fence, gated on `validity_check`.
///
/// When `fence` is supplied it is locked FIRST and held for the whole call, so
/// the validity check and the durable commit are indivisible against a racing
/// supersession that must take the same fence to invalidate the transition
/// (P26-C1). The check runs under the held fence; on `Err` it short-circuits
/// and `commit` is NEVER run. This is the single fence-guarded commit primitive
/// shared by the identity-transition coordinator and its supersession tests.
pub(crate) fn commit_under_fence<T>(
    fence: Option<&std::sync::Mutex<()>>,
    validity_check: impl FnOnce() -> Result<(), String>,
    commit: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _fence = match fence.map(|m| m.lock()) {
        Some(Ok(guard)) => Some(guard),
        Some(Err(e)) => return Err(format!("identity transition fence poisoned: {e}")),
        None => None,
    };
    validity_check()?;
    commit()
}

/// Blocking body of [`import_identity`]: key recovery, journaled drain (when a
/// scope is active), identity commit, and scope clear.  The caller has ALWAYS
/// acquired `workspace_transition` before invoking this (P26-C1); the
/// active/no-scope branch is decided here from a scope snapshot taken under
/// that held guard.
///
/// `validity_check` runs at the pre-commit boundary — after a successful drain,
/// immediately before the durable identity commit — while `commit_fence` (when
/// supplied) is held. When it returns `Err`, the drained runtimes are
/// compensated and NO identity is committed. `commit_fence` is held ALIVE
/// across the durable commit (P26-C1): for the phone-recovery caller this is
/// the pairing `generation_fence`, so a supersession that races the commit
/// either compensates (cancelled during drain, before the fence is taken) or
/// loses to the committed recovery (cancelled after the fenced commit begins).
/// Normal import supplies `None` + an always-`Ok(())` check.
fn import_identity_blocking<R, S>(
    app_handle: tauri::AppHandle<R>,
    nsec: String,
    password: Option<String>,
    commit_fence: Option<std::sync::Arc<std::sync::Mutex<()>>>,
    store: &S,
    data_dir: std::path::PathBuf,
    validity_check: impl FnOnce() -> Result<(), String>,
) -> Result<IdentityInfo, String>
where
    R: tauri::Runtime,
    S: crate::app_state::IdentityKeyStore,
{
    // NIP-49 backups require a passphrase and decrypt entirely in Rust.
    // Raw nsec/hex input follows the existing parser path unchanged.
    let password = password.map(zeroize::Zeroizing::new);
    let keys = crate::key_backup::recover_keys_from_input(
        &nsec,
        password.as_ref().map(|value| value.as_str()),
    )?;

    let state = app_handle.state::<AppState>();

    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
    let key_path = data_dir.join("identity.key");

    // ── Live-active path: journaled drain before swapping identity ─────────
    // Drain all managed-agent runtimes under `managed_agent_runtime_transition`
    // (Layer 2) BEFORE persisting the new identity — same protocol as
    // `apply_workspace`.  The store lock is held through drain/save; on drain
    // failure the transition guard is passed into compensate_drain so
    // compensation runs without any interleave window.
    //
    // The active/no-scope branch is decided HERE, from a scope snapshot taken
    // UNDER the `workspace_transition` guard the caller holds unconditionally
    // (P26-C1). Sampling under the held guard is what closes the `None → Some`
    // activation race: a concurrent `apply_workspace` cannot commit a new
    // active scope between this sample and the durable identity commit, because
    // both serialize on `workspace_transition`.
    let pre_import_scope = state.capture_active_scope();
    let has_active_scope = pre_import_scope.is_some();

    // The identity active before this transition (A), captured for the P28-C1
    // journal BEFORE any swap. `to_pubkey` is the imported identity (B).
    let from_pubkey = state
        .identity_lifecycle_keys_guard()
        .map_err(|e| format!("read current identity for transition journal: {e}"))?
        .public_key()
        .to_hex();
    let to_pubkey = keys.public_key().to_hex();

    let _rt_transition_guard = if has_active_scope {
        Some(
            state
                .managed_agent_runtime_transition
                .lock()
                .map_err(|e| format!("managed_agent_runtime_transition poisoned: {e}"))?,
        )
    } else {
        None
    };

    let _store_guard = if has_active_scope {
        Some(
            state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| format!("managed_agents_store_lock poisoned: {e}"))?,
        )
    } else {
        None
    };

    let stopped_entries = if has_active_scope {
        match drain_managed_agent_runtimes_for_import(&app_handle, &state) {
            Ok(stopped) => stopped,
            Err((stopped, drain_err)) => {
                let msg = compensate_import_drain(
                    &app_handle,
                    &stopped,
                    pre_import_scope.as_ref(),
                    _rt_transition_guard,
                    _store_guard,
                    &format!("identity import drain failed: {drain_err}"),
                );
                return Err(msg);
            }
        }
    } else {
        vec![]
    };

    // ── Owner-identity egress barrier (P29/P30-C1) ────────────────────────
    // Between the runtime drain and the durable dispatch, drain owner-identity
    // egress: bump the identity-persistence generation, refuse new lease
    // admission, await in-flight leases, and revoke old-generation durable
    // capabilities (sessions/bearers). Both Layer-2 guards remain held
    // continuously across this barrier — releasing either permits A-runtime
    // resurrection before B commits (Thufir UNSAFE verdict on the three-phase
    // split, Paul-ruled `092acdeb75`). The wait is synchronous
    // (`wait_egress_drain_blocking`) because this body holds two std mutex
    // guards that cannot cross `.await`.
    //
    // Deadlock-freedom is a lock-order invariant, not a source-text heuristic:
    // every operation that needs a coordinator-held lock acquires it BEFORE
    // egress admission. The owner-identity artifact commands are structurally
    // pinned by `identity_egress_ordering_tests`; the backup's transition-held-
    // mutation schedule is driven in `identity_key_backup_tests`. A future
    // admission site must preserve that order and add an operation-level
    // schedule if it crosses another coordinator-held lock.
    //
    // The barrier runs UNCONDITIONALLY, even on the no-scope path: owner sends
    // are scope-independent, so an in-flight owner lease can exist with no
    // active agent scope.
    let winning_generation = crate::owner_identity_egress::begin_egress_drain()?;
    crate::owner_identity_egress::wait_egress_drain_blocking();
    crate::owner_identity_egress::revoke_durable_capabilities_before(winning_generation);

    // ── Fenced pre-commit gate → journal → classified durable persist ─────
    // The commit fence (when supplied) is acquired FIRST and held across the
    // journal write AND the durable persist (P26-C1 fence retention): a racing
    // supersession that must take the same fence can neither slip between the
    // check and the durable dispatch nor interleave with it. The P28-C1 journal
    // is written ONLY after the validity gate passes, so a superseded recovery
    // leaves no journal residue. The persist is CLASSIFIED against durable fact
    // (P27-C1) — never the kernel's `Ok`/`Err`. `store` and `data_dir` are the
    // durable-persist seams injected by the caller (production: the shared
    // `SecretStore` + real `app_data_dir()`; tests: a fake store + tempdir).
    let pending = crate::identity_transition_journal::IdentityTransitionPending {
        from_pubkey,
        to_pubkey,
    };
    let barrier_result = commit_under_fence(commit_fence.as_deref(), validity_check, || {
        crate::identity_transition_journal::write_pending(&data_dir, &pending)?;
        Ok(
            crate::identity_persistence::persist_imported_identity_classified(
                &keys,
                store,
                &key_path,
                || crate::app_state::persist_imported_identity(store, &keys, &key_path, &data_dir),
            ),
        )
    });

    use crate::identity_persistence::PersistenceOutcome;
    let (pubkey, storage) = match barrier_result {
        // Validity gate or journal write failed BEFORE any durable B persist:
        // reopen admission, compensate the drained runtimes, no durable write.
        Err(e) => {
            crate::owner_identity_egress::resume_egress_live();
            let msg = compensate_import_drain(
                &app_handle,
                &stopped_entries,
                pre_import_scope.as_ref(),
                _rt_transition_guard,
                _store_guard,
                &e,
            );
            return Err(msg);
        }
        // Durable B proven canonical: finish the in-memory swap through the
        // already-held guards (no fallible acquisition after durability,
        // P26-C2), clear the journal, and reopen admission at generation B.
        Ok(PersistenceOutcome::Committed(storage)) => {
            let committed = super::commit_imported_identity(&state, &data_dir, keys, storage);
            match committed {
                Ok(committed) => committed,
                // The in-memory swap itself failed after durable B landed —
                // a poisoned `state.keys` guard. B IS durable, so this is NOT
                // compensatable: latch fail-closed and leave the journal for
                // boot reconciliation rather than restoring live A beside B.
                Err(e) => {
                    crate::owner_identity_egress::latch_identity_indeterminate();
                    return Err(format!(
                        "identity B durably committed but the in-memory swap failed: {e}; \
                         latched indeterminate — relaunch to reconcile from durable fact"
                    ));
                }
            }
        }
        // B never landed, A intact: the only outcome permitted to compensate.
        Ok(PersistenceOutcome::DefinitelyUnchanged) => {
            let _ = crate::identity_transition_journal::clear_pending(&data_dir);
            crate::owner_identity_egress::resume_egress_live();
            let msg = compensate_import_drain(
                &app_handle,
                &stopped_entries,
                pre_import_scope.as_ref(),
                _rt_transition_guard,
                _store_guard,
                "identity import persist failed and durable A is still canonical",
            );
            return Err(msg);
        }
        // Neither identity provable: latch the durable fail-closed state, LEAVE
        // the journal, do NOT compensate or clear scope — runtimes stay down.
        // Boot/reconciliation resolves it later from durable fact (P28-C1).
        Ok(PersistenceOutcome::Indeterminate(reason)) => {
            crate::owner_identity_egress::latch_identity_indeterminate();
            return Err(format!(
                "identity import could not prove either identity canonical: {reason}; \
                 latched indeterminate — relaunch to reconcile"
            ));
        }
    };

    // ── Proven Committed: clear journal, scope, reopen admission ──────────
    // The journal is cleared only here, at the proven `Committed` exit after
    // the in-memory swap completed (best-effort — a proven-canonical identity
    // must never be blocked by a stale delete). `clear_active_scope()` bumps
    // the scope generation, making all agent commands fail closed until the
    // frontend re-applies a workspace. `resume_egress_live()` reopens egress at
    // generation B. The fallback relay can never claim legacy data — claims are
    // only written inside apply_workspace's prepare stage.
    if let Err(e) = crate::identity_transition_journal::clear_pending(&data_dir) {
        eprintln!("buzz-desktop: identity committed, but journal clear failed: {e}");
    }
    state.clear_active_scope();
    crate::owner_identity_egress::resume_egress_live();

    let pubkey_hex = pubkey.to_hex();
    let display_name = super::truncated_display_name(&pubkey)?;

    eprintln!("buzz-desktop: imported identity pubkey {}", pubkey_hex);

    Ok(IdentityInfo {
        pubkey: pubkey_hex,
        display_name,
        storage: storage.as_str().to_string(),
        lost: false,
        locked: false,
        reset_failed: false,
    })
}

#[cfg(test)]
mod tests {
    //! Closed-world identity-swap sink test (P25-C1, spec §7): the durable
    //! in-memory identity swap has exactly ONE reachable site —
    //! [`commit_imported_identity`], called ONLY from this coordinator's
    //! `Committed` arm. A third path calling it directly would re-open the
    //! P25/P26 split-state (swap identity B without the drain / scope clear /
    //! generation bump the coordinator performs), so the number of CALL sites
    //! is fixed by inventory. Adding a caller — in a new file or an existing
    //! one — trips this scan until its row is updated, which is the deliberate
    //! act that must accompany routing a new swap path through the coordinator.

    /// Per-file inventory of expected `commit_imported_identity` CALL sites
    /// (`(file suffix, expected calls)`). The `fn commit_imported_identity`
    /// DEFINITION and doc/comment mentions are excluded by [`call_sites`]; only
    /// call expressions count. The sole legitimate caller is this module.
    const SINK_INVENTORY: &[(&str, usize)] = &[("src/commands/identity_transition.rs", 1)];

    fn needle() -> String {
        // Assembled at runtime so this scan file's own inventory row (1) is the
        // real call in `import_identity_blocking`, not a literal in the table.
        ["commit_imported", "_identity"].concat()
    }

    fn src_rust_files() -> Vec<std::path::PathBuf> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        walk(&root, &mut out);
        out
    }

    fn read_src_files() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        src_rust_files()
            .into_iter()
            .map(|path| {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                (rel, std::fs::read_to_string(&path).unwrap())
            })
            .collect()
    }

    /// Count call sites of the sink in one file's content: lines that invoke
    /// `commit_imported_identity(` but are neither the `fn` definition nor a
    /// `//` comment. `pub(crate) use ...` re-exports are definitions of a name,
    /// not calls, and carry no `(`, so they never match.
    fn call_sites(content: &str, needle: &str) -> usize {
        let call = format!("{needle}(");
        content
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//")
                    && !trimmed.contains(&format!("fn {call}"))
                    && trimmed.contains(&call)
            })
            .count()
    }

    fn sink_violations(files: &[(String, String)]) -> Vec<String> {
        let needle = needle();
        let mut violations = Vec::new();
        for (rel, content) in files {
            let expected = SINK_INVENTORY
                .iter()
                .find(|(suffix, _)| rel.ends_with(suffix))
                .map(|&(_, e)| e)
                .unwrap_or(0);
            let found = call_sites(content, &needle);
            if found != expected {
                violations.push(format!(
                    "{rel}: found {found} commit_imported_identity call site(s), \
                     inventory expects {expected}"
                ));
            }
        }
        violations
    }

    /// Closed world: every `commit_imported_identity` call site in
    /// `desktop/src-tauri/src` matches the inventory — exactly one, in this
    /// coordinator. A new caller anywhere fails this until its row is updated.
    #[test]
    fn commit_imported_identity_sink_is_closed_world() {
        let violations = sink_violations(&read_src_files());
        assert!(
            violations.is_empty(),
            "identity-swap sink drift — a new commit_imported_identity caller must \
             route through run_identity_transition, not call the swap directly; then \
             update SINK_INVENTORY:\n{}",
            violations.join("\n")
        );
    }

    /// Mutation proof: a second caller added to an already-inventoried file
    /// (this one) is caught — the scan is not vacuously passing.
    #[test]
    fn sink_scan_catches_a_new_caller_in_an_inventoried_file() {
        let mut files = read_src_files();
        let me = files
            .iter_mut()
            .find(|(rel, _)| rel.ends_with("src/commands/identity_transition.rs"))
            .expect("this module must be in the scan set");
        me.1.push_str(&format!(
            "\n    let _ = {}(&state, dir, keys, storage);\n",
            needle()
        ));
        let violations = sink_violations(&files);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("src/commands/identity_transition.rs")),
            "a second sink caller must trip the scan: {violations:?}"
        );
    }

    /// Mutation proof: a caller in a brand-new file (no inventory row) is
    /// caught — the closed world admits no unlisted swap path.
    #[test]
    fn sink_scan_catches_a_caller_in_a_new_file() {
        let mut files = read_src_files();
        files.push((
            "src/sneaky_swap.rs".to_string(),
            format!("fn bypass() {{ {}(&s, d, k, st); }}", needle()),
        ));
        let violations = sink_violations(&files);
        assert!(
            violations.iter().any(|v| v.contains("src/sneaky_swap.rs")),
            "a sink caller in an unlisted file must trip the scan: {violations:?}"
        );
    }

    /// EARLY-gate zero-disruptive-work schedule (P26-C1, spec §7): a recovery
    /// superseded while it queued on the transition locks is rejected by
    /// `early_validity_check` — which runs under the held `workspace_transition`
    /// guard, immediately after acquisition and BEFORE the Mesh preflight and
    /// the `spawn_blocking` body — having done ZERO disruptive work. Because the
    /// gate short-circuits with `?` before any disruptive call, "zero work" is
    /// structural; this fixture pins it by driving the real coordinator and
    /// asserting the durable side-effect surfaces are untouched: egress never
    /// left `Live` (no `begin_egress_drain`), the persistence generation did not
    /// move, and no active scope was committed. The `nsec` is deliberately
    /// invalid — reaching key recovery at all would prove the gate leaked past
    /// its boundary.
    #[tokio::test]
    // The two process-global test-serialization guards are held across the
    // coordinator `.await` deliberately: they must cover the whole transition
    // so a sibling egress/scope test cannot observe or mutate the shared
    // registry mid-drive. No other task contends for them inside this test's
    // runtime, so holding them across the await cannot stall or deadlock it.
    #[allow(clippy::await_holding_lock)]
    async fn early_gate_rejection_does_zero_disruptive_work() {
        use crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK;
        use crate::owner_identity_egress::{
            current_identity_persistence_generation, identity_persistence_state,
            reset_registry_for_test, IdentityPersistenceState, EGRESS_REGISTRY_TEST_LOCK,
        };
        use tauri::Manager;

        // Serialize against every other egress/scope-sensitive test.
        let _egress_guard = EGRESS_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _scope_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_registry_for_test();

        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("failed to build mock app");
        let app_handle = app.handle().clone();
        let state = app.state::<crate::app_state::AppState>();

        let generation_before = current_identity_persistence_generation();
        assert_eq!(identity_persistence_state(), IdentityPersistenceState::Live);
        assert!(
            state.capture_active_scope().is_none(),
            "pre-condition: no active scope"
        );

        // Drive the coordinator with an early gate that is already superseded.
        // The late gate would never be reached, so it asserts unreachable.
        let result = super::run_identity_transition(
            app_handle,
            "nsec-invalid-must-never-be-parsed".to_string(),
            None,
            None,
            crate::secret_store::SecretStore::shared(crate::app_state::keyring_service()),
            std::env::temp_dir(),
            || Err("superseded while queued".to_string()),
            || unreachable!("late gate must not run after an early-gate rejection"),
        )
        .await;

        assert_eq!(
            result.err().expect("early-gate rejection must return Err"),
            "superseded while queued",
            "the early-gate error must propagate verbatim"
        );

        // Zero disruptive work: egress untouched (never drained), no generation
        // movement, no scope committed.
        assert_eq!(
            identity_persistence_state(),
            IdentityPersistenceState::Live,
            "early-gate rejection must not begin the egress drain"
        );
        assert_eq!(
            current_identity_persistence_generation(),
            generation_before,
            "early-gate rejection must not bump the persistence generation"
        );
        assert!(
            state.capture_active_scope().is_none(),
            "early-gate rejection must not commit a scope"
        );

        reset_registry_for_test();
    }

    /// Minimal `Send + Sync + 'static` [`IdentityKeyStore`] for the
    /// commit-driving fixtures below: a reachable keyring backed by a `Mutex`
    /// slot, so `store` + read-back `verify_stored` both succeed and the
    /// classifier returns `Committed(SystemKeyring)` — the durable outcome the
    /// §7 no-scope / activation-race schedules require. `RefCell`-backed
    /// `FakeIdentityStore` is `!Sync` and cannot cross the coordinator's
    /// `spawn_blocking`, so this is the seam-injection payload.
    struct SyncKeyringStore {
        slot: std::sync::Mutex<std::collections::HashMap<String, String>>,
    }

    impl SyncKeyringStore {
        fn reachable() -> Self {
            Self {
                slot: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
        fn holds(&self, key: &str) -> Option<String> {
            self.slot.lock().unwrap().get(key).cloned()
        }
    }

    impl crate::app_state::IdentityKeyStore for SyncKeyringStore {
        fn probe(&self, _name: &str) -> crate::secret_store::KeyringProbe {
            crate::secret_store::KeyringProbe::ReachableButEmpty
        }
        fn load(&self, name: &str) -> Result<Option<String>, String> {
            Ok(self.slot.lock().unwrap().get(name).cloned())
        }
        fn store(&self, name: &str, value: &str) -> Result<(), String> {
            self.slot
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.slot.lock().unwrap().remove(name);
            Ok(())
        }
        fn verify_stored(&self, name: &str, expected: &str) -> Result<bool, String> {
            Ok(self
                .slot
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|v| v == expected))
        }
    }

    /// Build a fresh mock app + a leaked reachable store, resetting both the
    /// egress registry and returning the imported identity B's nsec. The store
    /// is `Box::leak`ed to satisfy the coordinator's `store: &'static S`; each
    /// fixture builds its own so no state bleeds across tests.
    fn commit_fixture_setup() -> (
        tauri::App<tauri::test::MockRuntime>,
        &'static SyncKeyringStore,
        std::path::PathBuf,
        nostr::Keys,
        String,
    ) {
        use nostr::ToBech32;
        use tauri::Manager;
        crate::owner_identity_egress::reset_registry_for_test();
        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("failed to build mock app");
        let store: &'static SyncKeyringStore = Box::leak(Box::new(SyncKeyringStore::reachable()));
        let data_dir = tempfile::tempdir().unwrap().keep();
        let keys_b = nostr::Keys::generate();
        let nsec_b = keys_b.secret_key().to_bech32().unwrap();
        let _ = app.state::<crate::app_state::AppState>();
        (app, store, data_dir, keys_b, nsec_b)
    }

    /// NO-ACTIVE-SCOPE recovery schedule (P26-C1, spec §7): with no workspace
    /// applied, the coordinator still acquires `workspace_transition`, selects
    /// the no-scope branch from the snapshot taken UNDER the held guard (no
    /// drain, no Mesh preflight), and drives a REAL durable commit — proving
    /// the injected seams reach persistence hermetically. The identity is
    /// committed once and both the persistence generation (egress barrier, run
    /// unconditionally) and the scope generation (`clear_active_scope` on the
    /// `Committed` arm) advance exactly once.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn no_active_scope_recovery_commits_and_bumps_generation_once() {
        use crate::managed_agents::scope::{current_scope_generation, SCOPE_GENERATION_TEST_LOCK};
        use crate::owner_identity_egress::{
            current_identity_persistence_generation, identity_persistence_state,
            reset_registry_for_test, IdentityPersistenceState, EGRESS_REGISTRY_TEST_LOCK,
        };
        use nostr::ToBech32;
        use tauri::Manager;

        let _egress_guard = EGRESS_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _scope_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let (app, store, data_dir, keys_b, nsec_b) = commit_fixture_setup();
        let app_handle = app.handle().clone();
        let state = app.state::<crate::app_state::AppState>();

        assert!(
            state.capture_active_scope().is_none(),
            "pre-condition: no active scope"
        );
        let persistence_gen_before = current_identity_persistence_generation();
        let scope_gen_before = current_scope_generation();

        let info = super::run_identity_transition(
            app_handle,
            nsec_b,
            None,
            None,
            store,
            data_dir,
            || Ok(()),
            || Ok(()),
        )
        .await
        .expect("no-scope recovery must commit");

        // Identity B is now the live in-memory identity and durably in the store.
        assert_eq!(info.pubkey, keys_b.public_key().to_hex());
        assert_eq!(
            state.current_pubkey().unwrap(),
            keys_b.public_key(),
            "in-memory identity must be B after commit"
        );
        assert_eq!(
            store.holds(crate::app_state::IDENTITY_KEY_NAME),
            Some(keys_b.secret_key().to_bech32().unwrap()),
            "B must be durably persisted through the injected store"
        );

        // No scope committed, egress reopened at generation B, each generation
        // advanced exactly once.
        assert!(
            state.capture_active_scope().is_none(),
            "no-scope recovery must not leave an active scope"
        );
        assert_eq!(identity_persistence_state(), IdentityPersistenceState::Live);
        assert_eq!(
            current_identity_persistence_generation(),
            persistence_gen_before + 1,
            "the egress barrier must bump the persistence generation exactly once"
        );
        assert_eq!(
            current_scope_generation(),
            scope_gen_before + 1,
            "the Committed arm's clear_active_scope must bump the scope generation once"
        );

        reset_registry_for_test();
    }

    /// `None → Some` activation race, ORDER (a) — recovery wins the lock
    /// (P26-C1, spec §7). The two orderings are the two deterministic outcomes
    /// of the unconditional `workspace_transition` serialization; forcing a
    /// live race is nondeterministic, so each resolved order is pinned
    /// directly. Here the recovery acquires the guard first, takes a TRUE
    /// no-scope snapshot, and commits identity B; the activation that follows
    /// then applies cleanly against B. No durable swap lands beside an
    /// undrained scope — the recovery saw none.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn activation_race_recovery_wins_commits_under_no_scope_snapshot() {
        use crate::managed_agents::scope::{
            next_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK,
        };
        use crate::owner_identity_egress::{reset_registry_for_test, EGRESS_REGISTRY_TEST_LOCK};
        use tauri::Manager;

        let _egress_guard = EGRESS_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _scope_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let (app, store, data_dir, keys_b, nsec_b) = commit_fixture_setup();
        let app_handle = app.handle().clone();
        let state = app.state::<crate::app_state::AppState>();
        let activation_dir = data_dir.clone();

        // Recovery wins the lock: no scope is present when it takes its snapshot.
        let info = super::run_identity_transition(
            app_handle,
            nsec_b,
            None,
            None,
            store,
            data_dir,
            || Ok(()),
            || Ok(()),
        )
        .await
        .expect("recovery must commit under a no-scope snapshot");

        assert_eq!(info.pubkey, keys_b.public_key().to_hex());
        assert!(
            state.capture_active_scope().is_none(),
            "recovery must have committed under a no-scope snapshot"
        );

        // The activation that lost the race now applies cleanly against B.
        let gen = next_scope_generation();
        state.commit_active_scope(WorkspaceAgentScope {
            scope_id: "activation-wins-b".to_string(),
            relay_url: "wss://relay.example".to_string(),
            owner_pubkey: keys_b.public_key().to_hex(),
            definitions_dir: activation_dir,
            generation: gen,
        });

        assert_eq!(
            state.current_pubkey().unwrap(),
            keys_b.public_key(),
            "identity B stays live under the activation applied after recovery"
        );
        assert!(
            state.capture_active_scope().is_some(),
            "the activation must apply against committed identity B"
        );

        reset_registry_for_test();
    }

    /// `None → Some` activation race, ORDER (b) — activation wins the lock
    /// (P26-C1, spec §7). A scope is committed BEFORE the recovery acquires
    /// `workspace_transition`; the recovery's snapshot — taken UNDER the held
    /// guard — therefore observes the NEW active scope and runs the full
    /// active-scope protocol (drain, commit B, scope clear). B does not land
    /// beside an undrained live scope: the scope the recovery saw is drained
    /// and cleared as part of the commit.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn activation_race_activation_wins_recovery_takes_active_scope_path() {
        use crate::managed_agents::scope::{
            next_scope_generation, WorkspaceAgentScope, SCOPE_GENERATION_TEST_LOCK,
        };
        use crate::owner_identity_egress::{reset_registry_for_test, EGRESS_REGISTRY_TEST_LOCK};
        use tauri::Manager;

        let _egress_guard = EGRESS_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _scope_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let (app, store, data_dir, keys_b, nsec_b) = commit_fixture_setup();
        let app_handle = app.handle().clone();
        let state = app.state::<crate::app_state::AppState>();

        // Activation won the lock first: a scope is live before the recovery
        // takes its under-guard snapshot.
        let gen = next_scope_generation();
        let scope = WorkspaceAgentScope {
            scope_id: "activation-first".to_string(),
            relay_url: "wss://relay.example".to_string(),
            owner_pubkey: "aa".repeat(32),
            definitions_dir: data_dir.clone(),
            generation: gen,
        };
        state.commit_active_scope(scope);
        assert!(state.capture_active_scope().is_some());

        let info = super::run_identity_transition(
            app_handle,
            nsec_b,
            None,
            None,
            store,
            data_dir,
            || Ok(()),
            || Ok(()),
        )
        .await
        .expect("active-scope recovery must commit");

        // The recovery observed the pre-committed scope, ran the active path,
        // committed B, and cleared the scope — no swap beside a live scope.
        assert_eq!(info.pubkey, keys_b.public_key().to_hex());
        assert_eq!(
            state.current_pubkey().unwrap(),
            keys_b.public_key(),
            "in-memory identity must be B after the active-scope commit"
        );
        assert!(
            state.capture_active_scope().is_none(),
            "the active-scope Committed arm must clear the scope the recovery drained"
        );

        reset_registry_for_test();
    }
}
