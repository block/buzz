use super::*;
// The tests call `apply_record_field_updates(...)` and consume the return value
// via `.expect(...)`, discarding `RecordFieldsApplied`. The tests verify column
// writes (side effects), not the token itself. The lint is suppressed here so
// callers remain readable. Production code (update_managed_agent) must never
// suppress it — the token IS the outer-seam compile-time proof.

fn provider_record(deployed: bool) -> ManagedAgentRecord {
    let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
        "agent_command": "", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
        "updated_at": "", "last_started_at": null, "last_stopped_at": null,
        "last_exit_code": null, "last_error": null
    }))
    .unwrap();
    record.backend = crate::managed_agents::BackendKind::Provider {
        id: "provider".into(),
        config: serde_json::json!({}),
    };
    record.backend_agent_id = deployed.then(|| "deployment".to_string());
    record
}

#[test]
fn deployed_provider_rejects_access_edits_that_cannot_be_revoked() {
    let error = ensure_access_policy_change_supported(&provider_record(true), true)
        .expect_err("deployed provider access edit must fail closed");
    assert!(error.contains("no explicit stop or revocation acknowledgement"));
}

#[test]
fn undeployed_provider_accepts_access_edits() {
    ensure_access_policy_change_supported(&provider_record(false), true)
        .expect("no running provider deployment can retain stale access");
}

fn local_record() -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": "local", "name": "Local Agent", "relay_url": "", "acp_command": "",
        "agent_command": "", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
        "updated_at": "", "last_started_at": null, "last_stopped_at": null,
        "last_exit_code": null, "last_error": null
    }))
    .unwrap()
    // BackendKind deserializes as Local when the field is absent (the json! above).
}

// ── Production-entered seam tests (apply_record_field_updates) ──────────────
//
// These tests call `apply_record_field_updates`, the same function production
// calls inside `update_managed_agent` for the env_vars+effort ordered write.
// They verify:
//   - non-local records are rejected AND the column is NOT mutated;
//   - local set writes to the column and sweeps stale env aliases;
//   - local clear zeroes the column and sweeps stale env aliases;
//   - env_vars applied before effort so no same-request alias re-pins the column.
//
// Deletion proof for the effort guard: removing `ensure_effort_change_supported`
// inside `apply_record_field_updates` makes reject tests return `Ok(())` instead
// of `Err`, and the "record not mutated" assertions fail.
//
// Deletion proof for the apply call: removing the `apply_effort_update` call
// inside `apply_record_field_updates` leaves `effort_level == None` on local-set.
//
// Deletion proof for the env_vars step: removing `apply_env_vars_then_effort_transition`
// inside `apply_record_field_updates` leaves the env alias in `env_vars` on local-set.
//
// Ordering proof: `env_vars` with a stale alias is applied BEFORE effort so the
// alias is stripped; reversing the order leaves both the alias and the new column.
//
// Outer-seam proof (compile-error): removing `apply_record_field_updates` from
// `update_managed_agent` leaves `applied` undefined at `stamp_record_updated_at`
// — a compile error enforced by the `#[must_use] RecordFieldsApplied` token.
// `record_field_updates_persist_effort_to_disk` below proves the
// disk-persistence contract of `apply_record_field_updates` itself (calls it
// directly); it does not independently gate the production invocation.

#[test]
fn non_local_set_is_rejected_and_record_not_mutated() {
    let mut record = provider_record(false);
    let err = apply_record_field_updates(&mut record, None, false, Some(Some("high".to_string())))
        .expect_err("non-local record must reject effort writes");
    assert!(
        err.contains("remote effort is set at deploy time"),
        "error must explain why non-local effort writes are rejected: {err}"
    );
    // Column must not be touched — the rejection is before mutation.
    assert_eq!(
        record.effort_level, None,
        "non-local record column must be unchanged after a rejected set"
    );
}

#[test]
fn non_local_clear_is_rejected_and_record_not_mutated() {
    // Clear (None inner value) is also rejected for non-local records — the
    // outer Some signals presence; the inner None is the clear sentinel.
    let mut record = provider_record(false);
    let err = apply_record_field_updates(&mut record, None, false, Some(None))
        .expect_err("non-local record effort clear must also be rejected");
    assert!(err.contains("remote effort is set at deploy time"));
    assert_eq!(
        record.effort_level, None,
        "non-local record column must be unchanged after a rejected clear"
    );
}

#[test]
fn local_set_writes_column_and_sweeps_stale_alias() {
    // `apply_record_field_updates` must write `effort_level` for a local record
    // and strip any stale record-scope effort alias. Deleting the
    // `apply_effort_update` call inside leaves `effort_level == None`.
    let mut record = local_record();
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "low".to_string());

    let _ = apply_record_field_updates(&mut record, None, false, Some(Some("high".to_string())))
        .expect("local record must accept effort set");

    assert_eq!(
        record.effort_level.as_deref(),
        Some("high"),
        "local set must write the canonical column"
    );
    assert!(
        !record.env_vars.contains_key("GOOSE_THINKING_EFFORT"),
        "local set must sweep the stale record-native alias"
    );
}

#[test]
fn local_clear_zeroes_column_and_sweeps_alias() {
    let mut record = local_record();
    record.effort_level = Some("high".to_string());
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "high".to_string());

    apply_record_field_updates(&mut record, None, false, Some(None))
        .expect("local record must accept effort clear");

    assert_eq!(
        record.effort_level, None,
        "local clear must zero the canonical column"
    );
    assert!(
        !record.env_vars.contains_key("GOOSE_THINKING_EFFORT"),
        "local clear must sweep the stale record-native alias"
    );
}

#[test]
fn absent_effort_is_noop_for_any_backend() {
    // A missing effortLevel field (the common case) must never be rejected and
    // must never touch the column — this is the don't-touch path.
    let mut local = local_record();
    apply_record_field_updates(&mut local, None, false, None)
        .expect("absent effort must pass for local");
    assert_eq!(
        local.effort_level, None,
        "absent effort must not touch local column"
    );

    let mut provider = provider_record(true);
    apply_record_field_updates(&mut provider, None, false, None)
        .expect("absent effort must pass for provider");
    assert_eq!(
        provider.effort_level, None,
        "absent effort must not touch provider column"
    );
}

#[test]
fn env_vars_applied_before_effort_ordering_invariant() {
    // Order is load-bearing: env_vars BEFORE effort column write. A same-request
    // env_vars map containing a stale alias (GOOSE_THINKING_EFFORT=low) alongside
    // an explicit effort set (high) must end with the alias swept — not re-pinned.
    // If env_vars were applied AFTER effort, the alias would survive.
    let mut record = local_record();
    let mut env_vars = std::collections::BTreeMap::new();
    env_vars.insert("GOOSE_THINKING_EFFORT".to_string(), "low".to_string());

    apply_record_field_updates(
        &mut record,
        Some(&env_vars),
        false,
        Some(Some("high".to_string())),
    )
    .expect("ordering test must succeed for local record");

    assert_eq!(
        record.effort_level.as_deref(),
        Some("high"),
        "effort column must be set to the explicit value"
    );
    assert!(
        !record.env_vars.contains_key("GOOSE_THINKING_EFFORT"),
        "alias in the same-request env_vars must be swept before the column is read at launch"
    );
}

// ── Defensive direct-IPC contract ─────────────────────────────────────────────
//
// Non-blocking defensive coverage (Wes/Carl review): a contradictory request
// combining the ACP inherit sentinel in `env_vars` and a non-null effort_level
// must be deterministic — the effort write wins over the sentinel, and the
// sentinel is swept by the alias-removal step so it cannot shadow the column
// at launch time. The shipped renderer suppresses this combination, but the
// backend must not leave an ambiguous state.

#[test]
fn effort_write_sweeps_acp_sentinel_in_env_vars() {
    // A local record whose env_vars contain BUZZ_ACP_EFFORT_LEVEL (e.g. manually
    // set by a user) plus a concurrent explicit effort_level write. The column
    // must be set to the explicit value AND the sentinel must be removed.
    let mut record = local_record();
    record.env_vars.insert(
        "BUZZ_ACP_EFFORT_LEVEL".to_string(),
        "old-sentinel".to_string(),
    );
    apply_record_field_updates(&mut record, None, false, Some(Some("high".to_string())))
        .expect("local record must accept effort set");
    assert_eq!(
        record.effort_level.as_deref(),
        Some("high"),
        "effort write must set the column"
    );
    assert!(
        !record.env_vars.contains_key("BUZZ_ACP_EFFORT_LEVEL"),
        "ACP sentinel in env_vars must be swept by the alias-removal step"
    );
}

#[test]
fn effort_clear_sweeps_acp_sentinel_in_env_vars() {
    // A concurrent clear (None inner value) plus a pre-existing ACP sentinel.
    // After the clear the column is None and the sentinel is gone — no ambiguity.
    let mut record = local_record();
    record.effort_level = Some("high".to_string());
    record.env_vars.insert(
        "BUZZ_ACP_EFFORT_LEVEL".to_string(),
        "old-sentinel".to_string(),
    );
    apply_record_field_updates(&mut record, None, false, Some(None))
        .expect("local record must accept effort clear");
    assert_eq!(
        record.effort_level, None,
        "effort clear must zero the column"
    );
    assert!(
        !record.env_vars.contains_key("BUZZ_ACP_EFFORT_LEVEL"),
        "ACP sentinel in env_vars must be swept on clear"
    );
}

// ── Helper disk-persistence contract ─────────────────────────────────────────
//
// This test drives the production helper sequence directly in its own body:
//   load_managed_agents → apply_record_field_updates → stamp_record_updated_at
//   → save_managed_agents → load-from-disk.
//
// Mutation proofs (scoped to this test body):
//   - Removing `apply_record_field_updates` from this test body leaves
//     `applied` undefined at `stamp_record_updated_at` — a compile error.
//   - Removing the function call and stubbing the token manually leaves
//     `effort_level` unchanged on disk — assertion fails (expected
//     Some("high"), got None).
//
// Outer-seam gate: the compile error that prevents skipping
// `apply_record_field_updates` inside `update_managed_agent` is described in
// the outer-seam comment above (undefined `applied` token at the
// `stamp_record_updated_at` site). This test proves only the helper's own
// disk-roundtrip contract; it does not independently gate the production
// invocation.

#[cfg(not(target_os = "windows"))]
#[test]
fn record_field_updates_persist_effort_to_disk() {
    use crate::app_state::build_app_state;
    use crate::managed_agents::{load_managed_agents, save_managed_agents};

    // A single crate-wide process-env lock covers PATH, HOME, XDG_DATA_HOME,
    // and all effort env keys — `lock_path_mutex` and `lock_env_mutex` both
    // delegate to the same `PROCESS_ENV_MUTEX` static.
    let _env_guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    // RAII guards restore HOME and XDG_DATA_HOME on Drop (even on panic).
    // Uses OsString so a pre-existing non-Unicode value is restored exactly.
    struct EnvVarGuard {
        key: String,
        prior: Option<std::ffi::OsString>,
    }
    impl EnvVarGuard {
        fn set(key: &str, value: &std::path::Path) -> Self {
            let prior = std::env::var_os(key);
            #[allow(deprecated)]
            // SAFETY: caller holds the crate-wide process-env lock.
            unsafe {
                std::env::set_var(key, value)
            };
            Self {
                key: key.to_string(),
                prior,
            }
        }
    }
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            #[allow(deprecated)]
            // SAFETY: caller holds the crate-wide process-env lock.
            unsafe {
                match &self.prior {
                    Some(v) => std::env::set_var(&self.key, v),
                    None => std::env::remove_var(&self.key),
                }
            }
        }
    }

    let _home_guard = EnvVarGuard::set("HOME", &home);
    let _xdg_guard = EnvVarGuard::set("XDG_DATA_HOME", &home);

    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds headless");

    // Seed a local record with no effort set.
    let seed: crate::managed_agents::ManagedAgentRecord =
        serde_json::from_value(serde_json::json!({
            "pubkey": "test-effort-agent",
            "name": "Effort Test Agent",
            "relay_url": "", "acp_command": "", "agent_command": "",
            "agent_args": [], "mcp_command": "", "turn_timeout_seconds": 0,
            "system_prompt": null, "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z", "last_started_at": null,
            "last_stopped_at": null, "last_exit_code": null, "last_error": null
        }))
        .unwrap();
    save_managed_agents(app.handle(), &[seed]).unwrap();

    // Drive the production seam: load → apply_record_field_updates →
    // stamp_record_updated_at → save. This is the exact sequence that
    // `update_managed_agent` executes inside its locked transaction.
    let mut records = load_managed_agents(app.handle()).unwrap();
    let record = records
        .iter_mut()
        .find(|r| r.pubkey == "test-effort-agent")
        .expect("seeded record must load");
    let applied = apply_record_field_updates(record, None, false, Some(Some("high".to_string())))
        .expect("local record must accept effort set");
    stamp_record_updated_at(record, applied);
    save_managed_agents(app.handle(), &records).unwrap();

    // Verify effort landed on disk.
    let saved = load_managed_agents(app.handle()).unwrap();
    let saved_record = saved
        .iter()
        .find(|r| r.pubkey == "test-effort-agent")
        .expect("agent must persist after update");
    assert_eq!(
        saved_record.effort_level.as_deref(),
        Some("high"),
        "apply_record_field_updates + stamp_record_updated_at must write effort_level to disk"
    );
    // _home_guard and _xdg_guard restore HOME and XDG_DATA_HOME via Drop.
}
