//! Workspace-scoped agent definition store.
//!
//! Every agent definition (managed-agents.json, teams.json,
//! global-agent-config.json) lives under a `(relay_url, owner_pubkey)` scope
//! so that definitions created in workspace A never appear in workspace B's
//! store or relay. The scope identity uses the same sha256 derivation as the
//! retention database, so "same scope" can never disagree between the two
//! subsystems.
//!
//! # Scoped layout
//!
//! ```text
//! <app-data>/agents/scopes/<scope_id>/managed-agents.json
//! <app-data>/agents/scopes/<scope_id>/teams.json
//! <app-data>/agents/scopes/<scope_id>/global-agent-config.json
//! ```
//!
//! # Active scope lifecycle
//!
//! `Option<WorkspaceAgentScope>` is `None` from boot until the first
//! successful `apply_workspace`. Every agent command fails closed on `None`
//! ("no active workspace"). There is NO fallback to the legacy unscoped root;
//! that fallback would recreate split-brain storage.
//!
//! # Transition model (four stages)
//!
//! See [`crate::commands::workspace`] for the full transition machine.
//! 1. **Prepare (reversible):** derive/init target scope while old scope stays active.
//! 2. **Drain (journaled, compensating):** stop old-scope runtimes; on failure
//!    compensate by restarting the journaled set; return `applied: false` with
//!    explicit degradation when compensation itself fails.
//! 3. **Commit (infallible critical section):** pure in-memory swaps, no I/O.
//! 4. **Post-commit (non-rollback):** nest regen, event sync, new-scope restore;
//!    failures surface as degradation on an `applied: true` result.
//!
//! # Lock architecture (two layers)
//!
//! **Layer 1 — async serialization (Tokio mutexes, awaits OK):**
//! `identity_mutation` → `workspace_transition` → Mesh `rearm_lock` → `mesh_llm_runtime`
//!
//! **Layer 2 — synchronous commit epoch (no `.await` while any guard held):**
//! `managed_agent_runtime_transition` → `managed_agents_store_lock` →
//! `managed_agent_processes` → short commit locks (relay override, keys,
//! active_agent_scope).
//!
//! Generation checks bridge the layers: state read under Layer 1 is
//! revalidated by generation inside the Layer 2 epoch immediately before
//! commit.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// The single scope authority for a workspace's agent definition store.
///
/// Immutable after creation — every field is `pub` for read access only;
/// mutations always produce a new `WorkspaceAgentScope`. Callers capture one
/// scope at the entry of any operation that crosses an `.await` or a thread
/// boundary and thread it through every load/save via `_at(scope)` APIs.
/// A stale commit (generation mismatch) must abort; a stale spawn must
/// additionally terminate its child and remove its receipt.
#[derive(Debug, Clone)]
pub struct WorkspaceAgentScope {
    /// The sha256 scope identifier — byte-identical to the retention DB's
    /// derivation so the two subsystems can never disagree about ownership.
    pub scope_id: String,
    /// The normalized relay URL for this scope.
    pub relay_url: String,
    /// The owner's hex pubkey.
    pub owner_pubkey: String,
    /// The scoped definitions directory: `<agents-base>/scopes/<scope_id>/`.
    pub definitions_dir: PathBuf,
    /// Monotonically increasing generation counter; incremented on every scope
    /// change (including identity import that clears the active scope to None).
    /// Used by long-running operations to detect a mid-flight workspace switch.
    pub generation: u64,
}

/// Process-lifetime generation counter. Monotonically incremented every time
/// the active scope changes (new scope committed or scope cleared). Operations
/// that cross awaits read this at entry and re-validate before commit.
static SCOPE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Increment and return the new generation. Called by the commit stage of
/// `apply_workspace` and by identity import when the active scope is cleared.
pub(crate) fn next_scope_generation() -> u64 {
    SCOPE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1
}

/// Read the current generation without incrementing.
pub fn current_scope_generation() -> u64 {
    SCOPE_GENERATION.load(Ordering::Acquire)
}

/// Validate that a captured scope's generation still matches the global
/// generation counter.
///
/// Call this immediately before any commit or runtime registration that was
/// prepared under a previously-captured scope. Returns `Err` when a workspace
/// switch happened between capture and commit, with a message naming the
/// pubkey whose operation is being aborted.
///
/// Usage pattern:
/// ```rust,ignore
/// let scope = state.capture_active_scope().ok_or("no active scope")?;
/// // ... async work ...
/// validate_scope_generation(&scope)?;
/// // commit / register runtime
/// ```
pub fn validate_scope_generation(captured: &WorkspaceAgentScope) -> Result<(), String> {
    let current = current_scope_generation();
    if captured.generation == current {
        Ok(())
    } else {
        Err(format!(
            "stale scope: captured generation {} ≠ current {}; workspace switched mid-operation",
            captured.generation, current
        ))
    }
}

/// Relay-URL normalization used to derive a scope identifier. Must be
/// identical to `normalized_relay_scope` in `retention.rs` so the sha256
/// output is byte-identical.
pub(crate) fn normalize_relay_for_scope(relay_url: &str) -> &str {
    relay_url.trim().trim_end_matches('/')
}

/// Derive the sha256 scope identifier for a `(relay_url, owner_pubkey)` pair.
///
/// **This is the canonical derivation** — both the retention DB path
/// (`retention::scoped_retention_db_path`) and the definition scope directory
/// go through this function. The hash encodes the pair so relay URLs never
/// become path components.
///
/// byte-identical to `retention::scoped_retention_db_path`'s inner hash.
pub fn derive_scope_id(relay_url: &str, owner_pubkey: &str) -> String {
    let normalized_relay = normalize_relay_for_scope(relay_url);
    let mut hasher = Sha256::new();
    hasher.update(owner_pubkey.trim().to_ascii_lowercase().as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized_relay.as_bytes());
    hex::encode(hasher.finalize())
}

/// Resolve the definition scope directory for a `(relay_url, owner_pubkey)`
/// pair under `base_dir`.
///
/// Layout: `<base_dir>/scopes/<scope_id>/`
pub fn scoped_definitions_dir(base_dir: &std::path::Path, scope_id: &str) -> PathBuf {
    base_dir.join("scopes").join(scope_id)
}

impl WorkspaceAgentScope {
    /// Construct a new scope, deriving the scope_id and definitions_dir from
    /// the relay/owner pair.
    pub fn new(
        relay_url: String,
        owner_pubkey: String,
        base_dir: &std::path::Path,
        generation: u64,
    ) -> Self {
        let scope_id = derive_scope_id(&relay_url, &owner_pubkey);
        let definitions_dir = scoped_definitions_dir(base_dir, &scope_id);
        Self {
            scope_id,
            relay_url,
            owner_pubkey,
            definitions_dir,
            generation,
        }
    }

    /// Ensure the definitions directory exists.
    #[allow(dead_code)] // Called in tests; here as a utility for future callers.
    pub fn ensure_dir(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.definitions_dir).map_err(|e| {
            format!(
                "failed to create scope dir {}: {e}",
                self.definitions_dir.display()
            )
        })
    }

    /// Path to the scoped `managed-agents.json`.
    #[allow(dead_code)] // Called in tests; here as a canonical path accessor.
    pub fn managed_agents_path(&self) -> PathBuf {
        self.definitions_dir.join("managed-agents.json")
    }

    /// Path to the scoped `teams.json`.
    #[allow(dead_code)] // Called in tests; here as a canonical path accessor.
    pub fn teams_path(&self) -> PathBuf {
        self.definitions_dir.join("teams.json")
    }

    /// Path to the scoped `global-agent-config.json`.
    #[allow(dead_code)] // Called in tests; here as a canonical path accessor.
    pub fn global_config_path(&self) -> PathBuf {
        self.definitions_dir.join("global-agent-config.json")
    }
}

/// Result returned by `apply_workspace` / `import_identity` drain-then-commit.
///
/// Three distinct states:
///
/// | `applied` | `blocked`    | `degraded` | Meaning |
/// |-----------|--------------|------------|---------|
/// | `true`    | `None`       | empty      | Clean success. |
/// | `true`    | `None`       | non-empty  | Applied with post-commit degradation (workspace IS active; warnings only). |
/// | `true`    | `Some(msg)`  | any        | Applied but blocked: scope committed, post-commit provider reconciliation failed. Workspace IS active; caller must surface this as a hard gate, not a warning, because dependent post-commit steps were skipped to preserve fail-closed behavior. |
/// | `false`   | `None`       | non-empty  | Drain failed; old scope still active. |
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceApplyResult {
    /// `true` when the new scope was committed; `false` when drain or
    /// compensation failed (old scope is still active).
    pub applied: bool,
    /// Non-empty when the workspace applied but some post-commit step (nest
    /// regen, event sync, runtime restore) failed. The workspace IS active;
    /// the degradation is informational. Also populated on drain-failure with
    /// the specific runtime(s) that could not be stopped or restored.
    pub degraded: Vec<String>,
    /// Set when `applied` is `true` but a post-commit provider-access
    /// reconciliation step failed hard. The workspace scope IS committed (the
    /// new relay/keys are active), but dependent post-commit steps (event sync,
    /// agent restore) were skipped to preserve fail-closed behavior. The caller
    /// must treat this as a gate failure — park on the loading screen and
    /// surface the reason — because rendering community UI against a workspace
    /// whose provider deployment may not have accepted owner-only access is
    /// unsafe. Retry by re-applying the workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

impl WorkspaceApplyResult {
    pub fn success() -> Self {
        Self {
            applied: true,
            degraded: Vec::new(),
            blocked: None,
        }
    }

    pub fn with_degradation(mut self, msg: impl Into<String>) -> Self {
        self.degraded.push(msg.into());
        self
    }

    pub fn drain_failed(msg: impl Into<String>) -> Self {
        Self {
            applied: false,
            degraded: vec![msg.into()],
            blocked: None,
        }
    }

    /// Scope committed, but post-commit provider reconciliation failed.
    ///
    /// `applied` is `true` (the new workspace IS active); `blocked` carries
    /// the failure reason. Any `degraded` entries collected before the failure
    /// (e.g. nest-regen) are preserved. Dependent post-commit steps must be
    /// skipped by the caller after returning this result.
    pub fn applied_but_blocked(reason: impl Into<String>, degraded: Vec<String>) -> Self {
        Self {
            applied: true,
            degraded,
            blocked: Some(reason.into()),
        }
    }
}

/// Process-global mutex that serializes tests touching the process-global
/// scope generation counter.
///
/// Any test that (a) captures a generation and requires it to be stable
/// through Phase 3a or (b) calls `next_scope_generation()` inside a hook
/// must hold this guard for its entire duration.  Tests across modules share
/// the same counter so they must share the same serialization primitive.
///
/// Exposed only under `#[cfg(test)]` to avoid polluting the production API.
#[cfg(test)]
pub(crate) static SCOPE_GENERATION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The scope-id derivation must be byte-identical to
    /// `retention::scoped_retention_db_path`'s inner hash.
    ///
    /// `scoped_retention_db_path` computes:
    ///   sha256(owner.trim().to_ascii_lowercase() + "\0" + normalized_relay)
    /// and encodes as hex. We verify parity by computing both and asserting
    /// equality.
    #[test]
    fn test_scope_id_parity_with_retention_hash() {
        use sha2::{Digest, Sha256};

        let relay = "wss://relay.example.com/";
        let owner = "AABBCCDD".repeat(8); // 64-char hex

        // What retention.rs computes:
        let normalized = relay.trim().trim_end_matches('/');
        let mut hasher = Sha256::new();
        hasher.update(owner.trim().to_ascii_lowercase().as_bytes());
        hasher.update(b"\0");
        hasher.update(normalized.as_bytes());
        let expected = hex::encode(hasher.finalize());

        // What our helper computes:
        let got = derive_scope_id(relay, &owner);

        assert_eq!(
            got, expected,
            "scope_id must be byte-identical to retention hash"
        );
    }

    #[test]
    fn test_scope_id_trailing_slash_normalization() {
        let owner = "aa".repeat(32);
        assert_eq!(
            derive_scope_id("wss://a.example/", &owner),
            derive_scope_id("wss://a.example", &owner),
            "trailing slash must produce same scope_id"
        );
    }

    #[test]
    fn test_scope_id_separates_relay_and_owner() {
        let owner_a = "aa".repeat(32);
        let owner_b = "bb".repeat(32);
        let relay_a = "wss://a.example";
        let relay_b = "wss://b.example";

        assert_ne!(
            derive_scope_id(relay_a, &owner_a),
            derive_scope_id(relay_b, &owner_a)
        );
        assert_ne!(
            derive_scope_id(relay_a, &owner_a),
            derive_scope_id(relay_a, &owner_b)
        );
        assert_eq!(
            derive_scope_id(relay_a, &owner_a),
            derive_scope_id(relay_a, &owner_a)
        );
    }

    #[test]
    fn test_scoped_definitions_dir_layout() {
        let base = Path::new("/data/agents");
        let scope_id = "abcdef1234";
        let dir = scoped_definitions_dir(base, scope_id);
        assert_eq!(dir, base.join("scopes").join(scope_id));
    }

    #[test]
    fn test_workspace_agent_scope_paths() {
        let base = std::env::temp_dir();
        let scope =
            WorkspaceAgentScope::new("wss://relay.example.com".into(), "aa".repeat(32), &base, 0);
        assert_eq!(
            scope.managed_agents_path(),
            scope.definitions_dir.join("managed-agents.json")
        );
        assert_eq!(scope.teams_path(), scope.definitions_dir.join("teams.json"));
        assert_eq!(
            scope.global_config_path(),
            scope.definitions_dir.join("global-agent-config.json")
        );
    }

    #[test]
    fn test_generation_increments_monotonically() {
        let _gen_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let before = current_scope_generation();
        let next = next_scope_generation();
        assert_eq!(next, before + 1);
        assert_eq!(current_scope_generation(), next);
    }

    /// Stale-commit detection: an operation that captured generation G must
    /// abort when the generation has advanced past G by commit time.
    ///
    /// This simulates the pattern used by every await-crossing workflow:
    /// capture generation at entry → do async work → re-read current → abort
    /// if stale. It verifies the global counter advances strictly so the
    /// check `captured != current` is reliable.
    #[test]
    fn test_generation_staleness_detected_after_scope_change() {
        let _gen_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let captured = next_scope_generation(); // capture at operation entry
                                                // Simulate a concurrent workspace switch bumping the generation.
        let after_switch = next_scope_generation();
        // The captured generation no longer matches the current one.
        assert_ne!(
            captured,
            current_scope_generation(),
            "captured generation must be stale after a concurrent switch"
        );
        assert_eq!(
            after_switch,
            current_scope_generation(),
            "after_switch must equal the current generation"
        );
    }

    /// A→B→A round-trip: applying scope A, then B, then A again produces a
    /// strictly increasing generation each time. The scope's relay and owner
    /// fields correctly reflect the active workspace at each step.
    #[test]
    fn test_scope_switch_a_to_b_to_a_advances_generation() {
        let _gen_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir();
        let owner = "aa".repeat(32);

        // Step A: commit scope A.
        let gen_a1 = next_scope_generation();
        let scope_a =
            WorkspaceAgentScope::new("wss://a.example".into(), owner.clone(), &base, gen_a1);
        assert_eq!(scope_a.relay_url, "wss://a.example");
        assert_eq!(scope_a.generation, gen_a1);

        // Step B: commit scope B — generation advances.
        let gen_b = next_scope_generation();
        let scope_b =
            WorkspaceAgentScope::new("wss://b.example".into(), owner.clone(), &base, gen_b);
        assert_eq!(scope_b.relay_url, "wss://b.example");
        assert!(gen_b > gen_a1, "B's generation must exceed A's");

        // Step A again: generation continues to advance.
        let gen_a2 = next_scope_generation();
        let scope_a2 =
            WorkspaceAgentScope::new("wss://a.example".into(), owner.clone(), &base, gen_a2);
        assert_eq!(scope_a2.relay_url, "wss://a.example");
        assert!(gen_a2 > gen_b, "A's second activation must exceed B's");
        assert_ne!(
            gen_a2, gen_a1,
            "same relay does not reset the generation counter"
        );
    }

    /// Rapid A→B→C: three distinct relays produce three strictly ordered
    /// generations. Any in-flight stale-spawn at generation A or B detects
    /// staleness after C is committed.
    #[test]
    fn test_rapid_scope_switch_a_b_c_all_stale_after_c() {
        let _gen_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir();
        let owner = "bb".repeat(32);

        let gen_a = next_scope_generation();
        let _ = WorkspaceAgentScope::new("wss://a.example".into(), owner.clone(), &base, gen_a);

        let gen_b = next_scope_generation();
        let _ = WorkspaceAgentScope::new("wss://b.example".into(), owner.clone(), &base, gen_b);

        let gen_c = next_scope_generation();
        let current = current_scope_generation();

        // Both A and B are stale relative to C.
        assert_ne!(gen_a, current, "gen_a must be stale after C");
        assert_ne!(gen_b, current, "gen_b must be stale after C");
        assert_eq!(gen_c, current, "gen_c is the current generation");
        assert!(gen_a < gen_b, "A < B");
        assert!(gen_b < gen_c, "B < C");
    }

    /// Switch-during-restore: a restore operation that captured generation G
    /// at entry must abort rather than commit its output if the active scope
    /// changed while restore was in progress. This test verifies the detection
    /// invariant without spawning threads — the generation counter is the
    /// source of truth.
    #[test]
    fn test_switch_during_restore_detected_by_generation_check() {
        let _gen_guard = SCOPE_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Restore captures the generation at its entry.
        let captured_at_restore_entry = next_scope_generation();

        // Simulate the restore doing async work (IO, network probes, etc.).
        // Concurrently, a workspace switch bumps the generation.
        let _new_scope_generation = next_scope_generation();

        // Restore tries to commit: checks whether its captured generation
        // still matches the current one.
        let current = current_scope_generation();
        assert_ne!(
            captured_at_restore_entry, current,
            "restore must detect the mid-flight switch and abort its commit"
        );
    }
}
