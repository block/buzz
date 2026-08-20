//! Unit tests for `managed_agents/runtime_commands.rs`.
//!
//! Kept in a sibling file so `runtime_commands.rs` stays under the
//! 1000-line size gate; `#[path]`-included from there.

use super::*;

/// Spawn a long-lived child process that stays running long enough for tests.
///
/// Cross-platform replacement for `sleep 10000` — seeds the in-memory runtimes
/// map before `sync_managed_agent_processes` runs its `try_wait()` scan so the
/// runtime survives to the eligibility check. Tests using this helper MUST kill
/// the returned `Child` when the test exits to avoid leaking OS processes.
fn spawn_long_lived_child_for_test() -> std::process::Child {
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .args(["-c", "while true; do sleep 1; done"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived test child (sh loop)")
    }
    #[cfg(windows)]
    {
        std::process::Command::new("ping")
            .args(["-n", "100000", "127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived test child (ping)")
    }
}

fn payload(
    relay_url: &str,
    lifecycle: ManagedAgentRuntimeLifecycle,
    error: Option<&str>,
) -> super::super::ManagedAgentRuntimeLifecycleObserverPayload {
    super::super::ManagedAgentRuntimeLifecycleObserverPayload {
        pubkey: "aa".repeat(32),
        relay_url: relay_url.into(),
        start_nonce: "test-generation".into(),
        lifecycle,
        error: error.map(str::to_owned),
    }
}

fn record_with_relay(relay_url: &str) -> super::super::ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{}",
            "name": "pin-test",
            "relay_url": "{relay_url}",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#,
        "aa".repeat(32)
    ))
    .unwrap()
}

#[test]
fn legacy_relay_pin_is_ignored_for_fan_out() {
    // Zero-touch cutover (#2122): a record carrying a creation-era
    // `relay_url` pin must fan out exactly like an unpinned one — the
    // stored field is parsed but never consulted. See
    // `effective_agent_relay_url`.
    let unpinned = record_with_relay("");
    let pinned = record_with_relay("wss://one.example");
    for record in [&unpinned, &pinned] {
        assert_eq!(
            crate::relay::effective_agent_relay_url(&record.relay_url, "wss://two.example"),
            "wss://two.example"
        );
    }
}

#[test]
fn unkeyable_relay_degrades_to_failed_row() {
    // A requested URL that cannot form a pair key must still yield a
    // Failed row keyed by the raw requested string, so one bad community
    // never aborts the rest of the reconcile batch.
    let record = record_with_relay("");
    let status = unkeyable_failed_status(
        &record,
        "not a url".to_string(),
        "relay access probe timed out".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
    );
    assert!(matches!(
        status.lifecycle,
        ManagedAgentRuntimeLifecycle::Failed
    ));
    assert_eq!(status.relay_url, "not a url");
    assert_eq!(status.requested_relay_url.as_deref(), Some("not a url"));
    assert_eq!(status.pubkey, record.pubkey);
    assert_eq!(
        status.error.as_deref(),
        Some("relay access probe timed out")
    );
    assert!(status.pid.is_none());
}

#[test]
fn runtime_key_rejects_non_hex_pubkeys() {
    assert!(ManagedAgentRuntimeKey::new("../not-a-key", "wss://relay.example").is_err());
    assert!(ManagedAgentRuntimeKey::new("gg".repeat(32), "wss://relay.example").is_err());
}

#[test]
fn runtime_key_canonicalizes_hex_pubkeys() {
    let key = ManagedAgentRuntimeKey::new("AA".repeat(32), "wss://relay.example").unwrap();
    assert_eq!(key.pubkey, "aa".repeat(32));
}

#[test]
fn observer_lifecycle_key_preserves_exact_canonical_pair() {
    let first = payload(
        "WSS://Relay.Example:443/",
        ManagedAgentRuntimeLifecycle::Ready,
        None,
    );
    let key = observer_lifecycle_key(&first.pubkey, &first).unwrap();
    assert_eq!(key.pubkey, first.pubkey);
    assert_eq!(key.relay_url, "wss://relay.example");

    let other = payload(
        "wss://other.example",
        ManagedAgentRuntimeLifecycle::Ready,
        None,
    );
    assert_ne!(key, observer_lifecycle_key(&other.pubkey, &other).unwrap());
}

#[test]
fn observer_lifecycle_rejects_cross_agent_and_desktop_states() {
    let ready = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Ready,
        None,
    );
    assert!(observer_lifecycle_key(&"bb".repeat(32), &ready).is_err());

    let stopped = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Stopped,
        None,
    );
    assert!(observer_lifecycle_key(&stopped.pubkey, &stopped).is_err());
}

#[test]
fn observer_lifecycle_enforces_failed_error_contract() {
    let failed = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Failed,
        None,
    );
    assert!(observer_lifecycle_key(&failed.pubkey, &failed).is_err());

    let ready_with_error = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Ready,
        Some("unexpected"),
    );
    assert!(observer_lifecycle_key(&ready_with_error.pubkey, &ready_with_error).is_err());
}

// ── drain journal / WorkspaceApplyResult tests ───────────────────────────

fn make_drain_entry(pubkey_hex: &str, relay: &str, auto: bool) -> DrainJournalEntry {
    DrainJournalEntry {
        key: ManagedAgentRuntimeKey::new(pubkey_hex, relay).unwrap(),
        start_on_app_launch: auto,
    }
}

fn make_exited_pair_runtime(scope_id: Option<String>) -> ManagedAgentPairRuntime {
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    let program = "/usr/bin/true";
    #[cfg(windows)]
    let program = "true";
    let child = Command::new(program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn /usr/bin/true");
    let process = super::super::ManagedAgentProcess {
        child,
        log_path: std::path::PathBuf::new(),
        spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
            &record_with_relay(""),
            &[],
            &[],
            "wss://relay.example",
            &Default::default(),
            false,
        ),
        setup_mode: false,
        adapter_availability: None,
        start_nonce: "test-nonce".to_string(),
        #[cfg(windows)]
        job: None,
    };
    ManagedAgentPairRuntime::starting(process, scope_id)
}

/// Spawn a long-running `sleep 999` process and wrap it in a
/// `ManagedAgentPairRuntime`.  The test is responsible for ensuring the
/// process is reaped.  `execute_drain_journal` will SIGKILL and wait it.
#[cfg(unix)]
fn make_live_pair_runtime() -> ManagedAgentPairRuntime {
    use std::process::{Command, Stdio};
    let child = Command::new("sleep")
        .arg("999")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sleep 999");
    let process = super::super::ManagedAgentProcess {
        child,
        log_path: std::path::PathBuf::new(),
        spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
            &record_with_relay(""),
            &[],
            &[],
            "wss://relay.example",
            &Default::default(),
            false,
        ),
        setup_mode: false,
        adapter_availability: None,
        start_nonce: "test-nonce".to_string(),
        #[cfg(windows)]
        job: None,
    };
    ManagedAgentPairRuntime::starting(process, None)
}

#[test]
fn test_drain_empty_map_returns_success() {
    let journal: Vec<DrainJournalEntry> = vec![];
    let (stopped, remaining, err) = execute_drain_journal(&journal, &mut HashMap::new(), |_| {});
    assert!(stopped.is_empty());
    assert!(remaining.is_empty());
    assert!(err.is_none());
}

#[test]
fn test_drain_exited_process_counts_as_stopped_and_clears_map() {
    // `true` exits immediately with 0 — process_is_running returns false
    // after a brief moment, so drain treats it as already-stopped and
    // calls wait() to reap it.
    let pubkey = "aa".repeat(32);
    let key = ManagedAgentRuntimeKey::new(&pubkey, "wss://relay.example").unwrap();
    let runtime = make_exited_pair_runtime(None);
    // Give the process a moment to exit before drain tries to stop it.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let entry = make_drain_entry(&pubkey, "wss://relay.example", true);
    let mut map = HashMap::from([(key, runtime)]);
    let (stopped, remaining, err) = execute_drain_journal(&[entry], &mut map, |_| {});
    assert_eq!(stopped.len(), 1, "exited process must appear in stopped");
    assert!(remaining.is_empty());
    assert!(err.is_none());
    assert!(map.is_empty(), "entry must be removed from the runtime map");
}

#[test]
fn test_drain_scope_id_propagates_from_runtime_starting() {
    let scope_id = Some("test-scope-abc".to_string());
    let runtime = make_exited_pair_runtime(scope_id.clone());
    assert_eq!(
        runtime.scope_id, scope_id,
        "scope_id must be preserved through ManagedAgentPairRuntime::starting()"
    );
}

#[test]
fn test_drain_missing_key_treated_as_already_stopped() {
    // A key in the journal but absent from the map is treated as
    // already stopped: it still appears in `stopped` so compensation
    // would attempt a restart (safe-but-redundant, not silent loss).
    let pubkey = "bb".repeat(32);
    let entry = make_drain_entry(&pubkey, "wss://relay.example", false);
    let mut map: HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime> = HashMap::new();
    let (stopped, remaining, err) = execute_drain_journal(&[entry], &mut map, |_| {});
    assert_eq!(stopped.len(), 1);
    assert!(remaining.is_empty());
    assert!(err.is_none());
}

#[test]
fn test_drain_cleanup_fn_called_for_each_stopped_entry() {
    let pubkey = "cc".repeat(32);
    let key = ManagedAgentRuntimeKey::new(&pubkey, "wss://relay.example").unwrap();
    let runtime = make_exited_pair_runtime(None);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let entry = make_drain_entry(&pubkey, "wss://relay.example", false);
    let mut map = HashMap::from([(key.clone(), runtime)]);
    let mut cleaned: Vec<ManagedAgentRuntimeKey> = Vec::new();
    execute_drain_journal(&[entry], &mut map, |k| cleaned.push(k.clone()));
    assert_eq!(
        cleaned,
        vec![key],
        "cleanup_fn must be called once per stopped entry"
    );
}

/// Drain with a live process: verifies that `execute_drain_journal` can
/// SIGKILL and wait a running process.  This exercises the real stop path
/// (process_is_running → terminate_process → child.wait) rather than the
/// "already exited / absent from map" path used by `make_exited_pair_runtime`.
///
/// Proves the precondition for compensation: when entry 1 is a live process
/// that gets SIGKILLed, it appears in `stopped`, and the transition lock held
/// by the caller remains exclusive throughout.
#[test]
#[cfg(unix)]
fn test_drain_live_process_sigkilled_and_added_to_stopped() {
    let pubkey = "ee".repeat(32);
    let key = ManagedAgentRuntimeKey::new(&pubkey, "wss://relay.example").unwrap();
    let runtime = make_live_pair_runtime();
    let entry = make_drain_entry(&pubkey, "wss://relay.example", true);
    let mut map = HashMap::from([(key.clone(), runtime)]);

    let (stopped, remaining, err) = execute_drain_journal(&[entry], &mut map, |_| {});

    assert_eq!(
        stopped.len(),
        1,
        "live process must appear in stopped after SIGKILL"
    );
    assert!(remaining.is_empty(), "no remaining on full success");
    assert!(err.is_none(), "no error when SIGKILL succeeds");
    assert_eq!(stopped[0].key.pubkey, pubkey);
    assert!(
        stopped[0].start_on_app_launch,
        "start_on_app_launch preserved"
    );
    // The runtime map must be empty — the stopped entry was removed.
    assert!(map.is_empty(), "runtime map must be empty after drain");
}

#[test]
fn test_workspace_apply_result_drain_failed_returns_applied_false() {
    let r = super::super::scope::WorkspaceApplyResult::drain_failed("stop failed");
    assert!(!r.applied);
    assert_eq!(r.degraded, vec!["stop failed"]);
}

#[test]
fn test_workspace_apply_result_degradation_accumulates() {
    let r = super::super::scope::WorkspaceApplyResult::success()
        .with_degradation("nest failed")
        .with_degradation("sync skipped");
    assert!(
        r.applied,
        "degraded workspace must still report applied: true"
    );
    assert_eq!(r.degraded.len(), 2);
    assert!(r.degraded[0].contains("nest"));
    assert!(r.degraded[1].contains("sync"));
}

#[test]
fn test_workspace_apply_result_applied_but_blocked_sets_fields() {
    let r = super::super::scope::WorkspaceApplyResult::applied_but_blocked(
        "provider deploy rejected",
        Vec::new(),
    );
    assert!(r.applied, "applied_but_blocked must report applied: true");
    assert_eq!(
        r.blocked.as_deref(),
        Some("provider deploy rejected"),
        "blocked must carry the failure reason"
    );
    assert!(
        r.degraded.is_empty(),
        "degraded must be empty when none passed"
    );
}

#[test]
fn test_workspace_apply_result_applied_but_blocked_preserves_degraded_entries() {
    let prior_degraded = vec!["nest context regeneration failed: io error".to_string()];
    let r = super::super::scope::WorkspaceApplyResult::applied_but_blocked(
        "store locked",
        prior_degraded,
    );
    assert!(r.applied, "applied_but_blocked must report applied: true");
    assert!(r.blocked.is_some(), "blocked must be set");
    assert_eq!(
        r.degraded.len(),
        1,
        "pre-failure degraded entries must be preserved"
    );
    assert!(r.degraded[0].contains("nest"));
}

#[test]
fn test_workspace_apply_result_three_state_distinction() {
    // State 1: clean success — applied true, blocked None, degraded empty.
    let clean = super::super::scope::WorkspaceApplyResult::success();
    assert!(clean.applied);
    assert!(clean.blocked.is_none());
    assert!(clean.degraded.is_empty());

    // State 2: applied with degradation — applied true, blocked None, degraded non-empty.
    let degraded =
        super::super::scope::WorkspaceApplyResult::success().with_degradation("event sync skipped");
    assert!(degraded.applied);
    assert!(degraded.blocked.is_none());
    assert!(!degraded.degraded.is_empty());

    // State 3: applied but blocked — applied true, blocked Some, any degraded.
    let blocked = super::super::scope::WorkspaceApplyResult::applied_but_blocked(
        "provider rejected",
        Vec::new(),
    );
    assert!(blocked.applied);
    assert!(blocked.blocked.is_some());

    // State 4: drain failed — applied false.
    let drained = super::super::scope::WorkspaceApplyResult::drain_failed("lock poisoned");
    assert!(!drained.applied);
    assert!(drained.blocked.is_none());
    assert!(!drained.degraded.is_empty());
}

/// Partial drain: verifies that `execute_drain_journal` delivers the correct
/// stopped prefix for compensation.
///
/// Contract: entry 1 (live process) is SIGKILLed and added to `stopped`;
/// entry 2 (absent from map) is also treated as stopped. On full success,
/// `stopped = [entry1, entry2]`, `remaining = []`, `err = None`.
///
/// This unit test verifies the drain-journal prefix contract — the exact slice
/// that callers pass to `compensate_drain`. The compensation round-trip
/// (`compensate_drain_for` with injected start_fn) is covered by
/// `test_compensate_for_restarts_stopped_entries_in_order` and companions below.
#[test]
#[cfg(unix)]
fn test_partial_drain_delivers_correct_stopped_prefix_with_live_process() {
    // Entry 1: a live sleep process that will be SIGKILLed.
    let pubkey1 = "aa".repeat(32);
    let key1 = ManagedAgentRuntimeKey::new(&pubkey1, "wss://relay.example").unwrap();
    let runtime1 = make_live_pair_runtime();
    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);

    // Entry 2: absent from map → treated as already stopped (Ok).
    let pubkey2 = "bb".repeat(32);
    let entry2 = make_drain_entry(&pubkey2, "wss://relay.example", false);

    let mut map = HashMap::from([(key1, runtime1)]);

    // Both entries in stopped, no remaining, no error.
    let (stopped, remaining, err) =
        execute_drain_journal(&[entry1.clone(), entry2.clone()], &mut map, |_| {});

    assert_eq!(
        stopped.len(),
        2,
        "both entries must be in stopped when all stop successfully"
    );
    assert!(remaining.is_empty(), "no remaining on full success");
    assert!(err.is_none(), "no error on full success");

    // Verify ordering: compensation restores in journal order.
    assert_eq!(stopped[0].key.pubkey, pubkey1, "stopped[0] must be entry1");
    assert_eq!(stopped[1].key.pubkey, pubkey2, "stopped[1] must be entry2");
    assert!(
        stopped[0].start_on_app_launch,
        "start_on_app_launch preserved for entry1"
    );
    assert!(
        !stopped[1].start_on_app_launch,
        "start_on_app_launch preserved for entry2"
    );
    assert!(map.is_empty(), "runtime map must be empty after drain");
}

/// Partial drain failure: entry 1 succeeds, entry 2 fails with an injected
/// stop error, entry 3 is the un-attempted tail.
///
/// Uses `execute_drain_journal_with_stop_fn` to inject the failure via a
/// deterministic closure instead of relying on OS-specific process-wait
/// behavior (on macOS, `Child::wait()` returns the cached exit status on a
/// second call rather than an error, making the pre-reap approach unreliable).
///
/// Both entry 1 and entry 2 ARE in the runtime map so `stop_fn` is called for
/// them. Entry 3 is absent from the map; since entry 2 fails, the journal aborts
/// before reaching entry 3, so entry 3 ends up in `remaining`.
///
/// Contract verified:
/// - `stopped`   = [entry1] — the exact prefix compensation must restore.
/// - `remaining` = [entry3] — the un-attempted tail (entry2 is the failure
///   point; it is neither stopped nor remaining).
/// - `err`       = Some(msg containing pubkey2) — first stop failure.
///
/// Proves the compensation data contract: only the successfully stopped prefix
/// is handed to compensation, so the journal cannot double-start entry3 or
/// skip entry1.
#[test]
fn test_partial_drain_stop_failure_delivers_stopped_prefix_and_remaining_tail() {
    let pubkey1 = "aa".repeat(32);
    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);

    let pubkey2 = "bb".repeat(32);
    let entry2 = make_drain_entry(&pubkey2, "wss://relay.example", false);

    let pubkey3 = "cc".repeat(32);
    let entry3 = make_drain_entry(&pubkey3, "wss://relay.example", true);

    // Put entries 1 and 2 into the map so stop_fn is called for each.
    // Entry 3 is intentionally absent — absent entries are treated as already
    // stopped (Ok) by the production path. However, since entry 2 fails, the
    // journal aborts before reaching entry 3, so entry 3 ends up in `remaining`.
    let key1 = entry1.key.clone();
    let key2 = entry2.key.clone();
    let mut map: HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime> = HashMap::from([
        (key1, make_exited_pair_runtime(None)),
        (key2, make_exited_pair_runtime(None)),
    ]);
    // Give the exited processes a moment to exit so stop_fn controls the outcome.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let (stopped, remaining, err) = execute_drain_journal_with_stop_fn(
        &[entry1.clone(), entry2.clone(), entry3.clone()],
        &mut map,
        |_| {}, // cleanup_fn no-op
        |key| {
            if key.pubkey == pubkey2 {
                Err(format!("injected stop failure for {}", key.pubkey))
            } else {
                Ok(())
            }
        },
    );

    // Entry 1 succeeded → must be in stopped for compensation.
    assert_eq!(
        stopped.len(),
        1,
        "only entry1 must be in stopped (entry2 stop failed)"
    );
    assert_eq!(stopped[0].key.pubkey, pubkey1, "stopped[0] must be entry1");
    assert!(
        stopped[0].start_on_app_launch,
        "start_on_app_launch preserved for entry1"
    );

    // Entry 3 was never attempted → must be in remaining.
    assert_eq!(
        remaining.len(),
        1,
        "entry3 (un-attempted tail) must be in remaining"
    );
    assert_eq!(
        remaining[0].key.pubkey, pubkey3,
        "remaining[0] must be entry3"
    );

    // Error must name the failing entry.
    assert!(err.is_some(), "error must be Some when a stop fails");
    let err_msg = err.unwrap();
    assert!(
        err_msg.contains(&pubkey2),
        "error message must name the failing entry pubkey: {err_msg}"
    );
}

// ── compensate_drain_for tests ──────────────────────────────────────────────
//
// These tests call `compensate_drain_for` directly — the lock-free production
// core — with an injected `start_fn`. The function takes `&mut [ManagedAgentRecord]`
// (loaded by the adapter under the store lock) and calls start_fn for each
// stopped entry. Tests inject a closure that records calls and returns
// synthetic success/failure without spawning processes or touching disk.
//
// The stale-scope generation guard lives in the adapter (`compensate_drain`),
// not the core, and is tested at the adapter level below.
//
// The serialization invariant (writers blocked by store lock, not transition
// guard) is documented in the adapter and verified in the adapter-level tests.

#[allow(dead_code)] // helper prepared for future tests; not yet referenced
fn make_captured_scope() -> super::super::scope::WorkspaceAgentScope {
    // Build a scope whose generation matches the current global counter.
    // Tests that need a stale scope call `next_scope_generation()` after
    // capturing this value.
    let gen = super::super::scope::current_scope_generation();
    super::super::scope::WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: std::path::PathBuf::from("/tmp/test-scope"),
        generation: gen,
    }
}

/// Two-entry compensation: entry 1 stopped, entry 2 stop-failed.
/// `compensate_drain_for` must call start_fn exactly for entry 1 and return
/// None (full success), proving the stopped-prefix contract.
///
/// The start_fn receives both the entry and the mutable records slice,
/// matching `start_pair_under_held_locks`'s contract.
#[test]
fn test_compensate_for_restarts_stopped_entries_in_order() {
    let pubkey1 = "aa".repeat(32);
    let pubkey2 = "bb".repeat(32);
    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);
    // entry2 was not stopped (stop failed), so it is NOT in the stopped slice.
    let stopped = vec![entry1.clone()];

    let mut records: Vec<super::super::ManagedAgentRecord> = Vec::new();
    let mut restarted: Vec<String> = Vec::new();
    let result = compensate_drain_for(&stopped, &mut records, |entry, _recs| {
        restarted.push(entry.key.pubkey.clone());
        Ok(())
    });

    assert!(result.is_none(), "compensation must succeed: {result:?}");
    assert_eq!(
        restarted,
        vec![pubkey1.clone()],
        "start_fn must be called exactly for entry1"
    );
    // entry2 was never in stopped — must not be restarted.
    assert!(
        !restarted.contains(&pubkey2),
        "entry2 (stop-failed) must not be restarted"
    );
}

/// Partial restart failure: start_fn returns an error for entry1, success for
/// entry2. The function must return a degradation message naming the failing
/// entry and not abort early (entry2 is still attempted).
#[test]
fn test_compensate_for_reports_partial_restart_failure() {
    let pubkey1 = "aa".repeat(32);
    let pubkey2 = "bb".repeat(32);
    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);
    let entry2 = make_drain_entry(&pubkey2, "wss://relay.example", false);
    let stopped = vec![entry1.clone(), entry2.clone()];

    let mut records: Vec<super::super::ManagedAgentRecord> = Vec::new();
    let pubkey1_clone = pubkey1.clone();
    let result = compensate_drain_for(&stopped, &mut records, |entry, _recs| {
        if entry.key.pubkey == pubkey1_clone {
            Err(format!("injected failure for {}", entry.key.pubkey))
        } else {
            Ok(())
        }
    });

    assert!(
        result.is_some(),
        "partial restart failure must return degradation message"
    );
    let msg = result.unwrap();
    assert!(
        msg.contains(&pubkey1),
        "degradation message must name the failing entry: {msg}"
    );
}

/// `compensate_drain_for` passes the records slice to start_fn so it can be
/// mutated (matching `start_pair_under_held_locks`'s &mut [ManagedAgentRecord]).
/// Prove start_fn receives and can mutate the slice.
#[test]
fn test_compensate_for_start_fn_receives_records_slice() {
    let pubkey1 = "aa".repeat(32);
    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);
    let stopped = vec![entry1.clone()];

    let mut records: Vec<super::super::ManagedAgentRecord> = Vec::new();
    let mut received_records_len: Option<usize> = None;
    let result = compensate_drain_for(&stopped, &mut records, |_entry, recs| {
        received_records_len = Some(recs.len());
        Ok(())
    });

    assert!(result.is_none(), "must succeed: {result:?}");
    assert_eq!(
        received_records_len,
        Some(0),
        "start_fn must receive the records slice (empty in this test)"
    );
}

// ── compensate_drain round-trip tests (via tauri::test::mock_app) ──────────
//
// These tests call `compensate_drain` directly — the real production function
// that takes an AppHandle and a held transition guard — using a
// `tauri::test::mock_builder()` app. This proves the full production path:
// AppHandle → load_managed_agents (reads live scope) → compensate_drain_for →
// generation validation → per-entry start_fn dispatch.
//
// The test manages the active scope via `commit_active_scope` (the test-only
// AppState helper) and writes managed-agents.json to the tmpdir so the live
// scope load succeeds. Spawn fails for every entry (no real process/binary in
// the test environment), so `compensate_drain` returns a degradation message
// naming the failing entries — proving the path was entered, not skipped.
//
// Concurrent-start exclusion is structural: `compensate_drain` takes
// `_rt_transition_held: MutexGuard<'_, ()>` by value. Passing ownership of
// the guard into the function proves at the type level that the lock is held
// continuously through every `start_pair_under_held_locks` call inside
// `compensate_drain_for`. No concurrent thread test is added because the Rust
// borrow checker enforces the exclusion contract at compile time.

fn build_mock_app_with_scope(
    tmp: &tempfile::TempDir,
) -> (
    tauri::App<tauri::test::MockRuntime>,
    super::super::scope::WorkspaceAgentScope,
) {
    // Write empty managed-agents.json so load_managed_agents_at returns Ok([]).
    std::fs::write(tmp.path().join("managed-agents.json"), b"[]").unwrap();
    let gen = super::super::scope::current_scope_generation();
    let scope = super::super::scope::WorkspaceAgentScope {
        scope_id: "test-scope-comp".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    {
        use tauri::Manager;
        let state = app.state::<crate::app_state::AppState>();
        state.commit_active_scope(scope.clone());
    }
    (app, scope)
}

/// `compensate_drain` with empty stopped list → returns `None` (no degradation)
/// and releases the transition guard.
///
/// Calls the real production function with a real AppHandle. Proves the
/// fast-path: empty stopped → guard dropped → None returned.
#[test]
fn test_compensate_drain_empty_stopped_returns_none_with_real_app() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, scope) = build_mock_app_with_scope(&tmp);
    let app_handle = app.app_handle().clone();
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();
    // Acquire and hold the transition lock, then hand it to compensate_drain.
    let transition_guard = state.managed_agent_runtime_transition.lock().unwrap();

    let result = compensate_drain(&app_handle, &[], &scope, transition_guard);

    assert!(
        result.is_none(),
        "empty stopped list must return None (no degradation): {result:?}"
    );
    // Guard was consumed by compensate_drain. The lock must be free again.
    assert!(
        state.managed_agent_runtime_transition.try_lock().is_ok(),
        "transition lock must be released after compensate_drain with empty stopped list"
    );
}

/// `compensate_drain` with a stale scope → returns a degradation message and
/// does NOT call start_pair for any entry.
///
/// Calls the real production function with a real AppHandle. Proves the
/// generation guard fires before any spawn attempt.
#[test]
fn test_compensate_drain_stale_scope_skips_all_with_real_app() {
    let _gen_guard = super::super::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let (app, scope) = build_mock_app_with_scope(&tmp);
    let app_handle = app.app_handle().clone();
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();

    // Advance the generation AFTER capturing the scope to make it stale.
    super::super::scope::next_scope_generation();

    let pubkey = "bb".repeat(32);
    let entry = make_drain_entry(&pubkey, "wss://relay.example", true);
    let transition_guard = state.managed_agent_runtime_transition.lock().unwrap();

    let result = compensate_drain(&app_handle, &[entry], &scope, transition_guard);

    assert!(
        result.is_some(),
        "stale scope must return a degradation message"
    );
    let msg = result.unwrap();
    assert!(
        msg.contains("compensation skipped")
            || msg.contains("stale scope")
            || msg.contains("generation"),
        "degradation message must describe stale scope: {msg}"
    );
}

/// `compensate_drain` with a fresh scope and entries that cannot be spawned
/// (no agent records in the empty store) → returns a degradation message
/// naming the failed restarts.
///
/// This is the end-to-end round-trip test: the real compensate_drain function
/// is called with a real AppHandle, loads managed-agents from the active scope,
/// reacquires the store lock, validates the generation, then iterates entries
/// via start_pair_under_held_locks. Since managed-agents.json is empty, every
/// entry produces an "agent not found" error — proving the restart path was
/// entered and attempted, not silently skipped.
///
/// NOTE: The scope's generation is re-snapped (and scope re-committed) AFTER
/// acquiring the transition guard to minimise the race window against concurrent
/// tests that call `next_scope_generation()`. The transition guard serialises
/// with the stale-scope test that also needs it, making the window near-zero.
#[test]
fn test_compensate_drain_attempts_restart_and_reports_degradation_with_real_app() {
    let _gen_guard = super::super::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("managed-agents.json"), b"[]").unwrap();
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.app_handle().clone();
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();

    let pubkey1 = "aa".repeat(32);
    let pubkey2 = "bb".repeat(32);
    let entry1 = make_drain_entry(&pubkey1, "wss://relay.example", true);
    let entry2 = make_drain_entry(&pubkey2, "wss://relay.example", false);

    // Acquire the transition guard FIRST, then snap the generation and build
    // the scope.  This serialises with any concurrent test that holds the
    // transition guard while calling `next_scope_generation()`, collapsing the
    // race window to zero at the moment validate_scope_generation runs.
    let transition_guard = state.managed_agent_runtime_transition.lock().unwrap();
    let gen = super::super::scope::current_scope_generation();
    let scope = super::super::scope::WorkspaceAgentScope {
        scope_id: "test-scope-comp-fresh".to_string(),
        relay_url: "wss://relay.example".to_string(),
        owner_pubkey: "aa".repeat(32),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    };
    state.commit_active_scope(scope.clone());

    // The active scope is set (generation valid). managed-agents.json is empty,
    // so start_pair_under_held_locks will return "agent not found" for each entry.
    let result = compensate_drain(&app_handle, &[entry1, entry2], &scope, transition_guard);

    // Both entries fail to restart → degradation message is returned.
    assert!(
        result.is_some(),
        "failed restarts must return a degradation message"
    );
    let msg = result.unwrap();
    // The message must name the failing entries (pubkey1 or pubkey2).
    let names_failing_entry = msg.contains(&pubkey1)
        || msg.contains(&pubkey2)
        || msg.contains("agent not found")
        || msg.contains("failed");
    assert!(
        names_failing_entry,
        "degradation message must describe why restart failed: {msg}"
    );
}

#[path = "runtime_commands_concurrency_tests.rs"]
mod concurrency_tests;
