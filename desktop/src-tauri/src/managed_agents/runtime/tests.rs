use crate::managed_agents::known_acp_runtime;

#[test]
fn buzz_agent_has_mcp_hooks() {
    let p = known_acp_runtime("buzz-agent").expect("should resolve");
    assert!(p.mcp_hooks);
    assert_eq!(p.mcp_command, Some("buzz-dev-mcp"));
}

#[test]
fn managed_adapter_rejects_path_shaped_buzz_agent_override() {
    let command = format!("/tmp/buzz-agent{}", std::env::consts::EXE_SUFFIX);
    let error = super::validate_managed_adapter_descriptor(&command, &[])
        .expect_err("a basename match must not authorize a custom executable");
    assert!(error.contains("unsupported_managed_adapter"));
}

#[test]
fn managed_adapter_rejects_non_default_arguments() {
    let error = super::validate_managed_adapter_descriptor(
        &crate::managed_agents::default_agent_command(),
        &["--unsafe".into()],
    )
    .expect_err("custom native capabilities must fail closed");
    assert!(error.contains("unsupported_managed_adapter"));
}

#[test]
fn managed_adapter_accepts_canonical_catalog_entry() {
    super::validate_managed_adapter_descriptor(
        &crate::managed_agents::default_agent_command(),
        &[],
    )
    .expect("the bundled default must remain supported");
}

#[test]
fn managed_adapter_binary_must_be_a_desktop_or_test_profile_sibling() {
    assert!(super::is_bundled_sibling(
        std::path::Path::new("/bundle/buzz-agent"),
        std::path::Path::new("/bundle/buzz-desktop"),
    ));
    assert!(super::is_bundled_sibling(
        std::path::Path::new("/target/debug/buzz-agent"),
        std::path::Path::new("/target/debug/deps/desktop-tests"),
    ));
    assert!(!super::is_bundled_sibling(
        std::path::Path::new("/tmp/buzz-agent"),
        std::path::Path::new("/bundle/buzz-desktop"),
    ));
}

#[test]
fn bundled_sibling_candidate_uses_app_or_test_target_directory() {
    let suffix = std::env::consts::EXE_SUFFIX;
    assert_eq!(
        super::adapter::bundled_sibling_candidate(
            std::path::Path::new("/bundle/buzz-desktop"),
            "buzz-dev-mcp",
        ),
        Some(std::path::PathBuf::from(format!(
            "/bundle/buzz-dev-mcp{suffix}"
        ))),
    );
    assert_eq!(
        super::adapter::bundled_sibling_candidate(
            std::path::Path::new("/target/debug/deps/desktop-tests"),
            "buzz-dev-mcp",
        ),
        Some(std::path::PathBuf::from(format!(
            "/target/debug/buzz-dev-mcp{suffix}"
        ))),
    );
}

#[test]
fn canonical_executable_rejects_empty_build_placeholder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir
        .path()
        .join(format!("buzz-agent{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&path, []).expect("write placeholder");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("mark placeholder executable");
    }
    assert!(
        super::adapter::canonical_executable(&path).is_none(),
        "an empty Cargo placeholder must not shadow the real workspace binary"
    );

    std::fs::write(&path, b"not-empty").expect("write executable fixture");
    assert_eq!(
        super::adapter::canonical_executable(&path),
        Some(std::fs::canonicalize(&path).expect("canonical fixture")),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_control_rpc_bridge_does_not_nest_runtime() {
    let value =
        crate::managed_agents::block_on_runtime_io(async { Ok::<_, std::io::Error>(42_u8) })
            .expect("control RPC bridge");
    assert_eq!(value, 42);
}
#[test]
fn buzz_agent_resolved_via_path() {
    assert!(known_acp_runtime("/usr/local/bin/buzz-agent").is_some_and(|p| p.mcp_hooks));
}

#[test]
fn codex_has_mcp_command() {
    let p = known_acp_runtime("codex-acp").expect("should resolve");
    assert!(!p.mcp_hooks, "codex-acp does not handle MCP_HOOK_SERVERS");
    assert_eq!(p.mcp_command, Some("buzz-dev-mcp"));
}

#[test]
fn goose_has_no_mcp_hooks() {
    let p = known_acp_runtime("goose").expect("should resolve");
    assert!(!p.mcp_hooks);
    assert_eq!(p.mcp_command, None);
}

#[test]
fn unknown_command_returns_none() {
    assert!(known_acp_runtime("custom-agent").is_none());
}

// ── build_respond_to_env tests ───────────────────────────────────────

use super::test_fixtures::{expected_mode, expected_owner_only};
use super::{build_respond_to_env, build_respond_to_env_with_policy};
use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

/// Construct a minimal record fixture for env-building tests. Only the
/// fields read by `build_respond_to_env` matter here.
fn fixture(
    respond_to: RespondTo,
    allowlist: Vec<String>,
    auth_tag: Option<String>,
) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "p".into(),
        name: "n".into(),
        persona_id: None,
        private_key_nsec: "nsec1fake".into(),
        auth_tag,
        relay_url: "ws://localhost:3000".into(),
        avatar_url: None,
        acp_command: "buzz-acp".into(),
        agent_command: "goose".into(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        provider_policy_pending: false,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to,
        respond_to_allowlist: allowlist,
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    }
}

fn configured_env(
    command: &std::process::Command,
) -> std::collections::BTreeMap<String, Option<String>> {
    command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect()
}

#[test]
fn managed_acp_env_unset_timeouts_preserve_harness_defaults_and_pair_controls() {
    let record = fixture(RespondTo::OwnerOnly, vec![], None);
    let lock_path = std::path::Path::new("/tmp/buzz/runtime-locks/pair.lock");
    let mut command = std::process::Command::new("buzz-acp");

    super::configure_managed_acp_turn_environment(&mut command, &record, lock_path);
    let env = configured_env(&command);

    assert_eq!(env.get("BUZZ_ACP_TURN_TIMEOUT"), Some(&None));
    assert_eq!(env.get("BUZZ_ACP_IDLE_TIMEOUT"), Some(&None));
    assert_eq!(env.get("BUZZ_ACP_MAX_TURN_DURATION"), Some(&None));
    assert_eq!(
        env.get("BUZZ_ACP_MULTIPLE_EVENT_HANDLING"),
        Some(&Some("steer".into()))
    );
    assert_eq!(env.get("BUZZ_ACP_DEDUP"), Some(&Some("queue".into())));
    assert_eq!(
        env.get("BUZZ_ACP_RUNTIME_LOCK_PATH"),
        Some(&Some(lock_path.display().to_string()))
    );
    assert_eq!(super::effective_acp_turn_limits(&record), (900, 7_200));
    let log_line = super::acp_turn_limits_log_line(&record);
    assert!(log_line.contains("idle 900s"));
    assert!(log_line.contains("maximum duration 7200s"));
    assert!(log_line.contains("do not bound managed runtime or job-runner lifetime"));
}

#[test]
fn managed_acp_env_emits_only_explicit_split_timeout_overrides() {
    let mut record = fixture(RespondTo::OwnerOnly, vec![], None);
    record.idle_timeout_seconds = Some(45);
    record.max_turn_duration_seconds = Some(180);
    let mut command = std::process::Command::new("buzz-acp");

    super::configure_managed_acp_turn_environment(
        &mut command,
        &record,
        std::path::Path::new("/tmp/pair.lock"),
    );
    let env = configured_env(&command);

    assert_eq!(env.get("BUZZ_ACP_TURN_TIMEOUT"), Some(&None));
    assert_eq!(env.get("BUZZ_ACP_IDLE_TIMEOUT"), Some(&Some("45".into())));
    assert_eq!(
        env.get("BUZZ_ACP_MAX_TURN_DURATION"),
        Some(&Some("180".into()))
    );
}

#[test]
fn durable_and_job_publication_rollout_gates_are_independent() {
    let rollback = super::ManagedRuntimeFeatureGates::from_values(Some("false"), Some("true"));
    assert_eq!(
        rollback.launch_mode(),
        super::ManagedRuntimeLaunchMode::LegacyPhase0
    );
    let auto_default = super::ManagedRuntimeFeatureGates::from_values(None, Some("true"));
    assert_eq!(
        auto_default.launch_mode(),
        super::ManagedRuntimeLaunchMode::DurableV2 {
            job_event_publication: true
        }
    );

    let durable_private =
        super::ManagedRuntimeFeatureGates::from_values(Some("true"), Some("false"));
    assert_eq!(
        durable_private.launch_mode(),
        super::ManagedRuntimeLaunchMode::DurableV2 {
            job_event_publication: false
        }
    );

    let durable_public = super::ManagedRuntimeFeatureGates::from_values(Some("1"), Some("yes"));
    assert_eq!(
        durable_public.launch_mode(),
        super::ManagedRuntimeLaunchMode::DurableV2 {
            job_event_publication: true
        }
    );
    let mut command = std::process::Command::new("buzz-acp");
    super::configure_rollout_gate_environment(&mut command, rollback.launch_mode());
    let env = configured_env(&command);
    assert_eq!(
        env.get("BUZZ_ACP_DURABLE_RUNTIME"),
        Some(&Some("false".into()))
    );
    assert_eq!(
        env.get("BUZZ_ACP_JOB_EVENT_PUBLICATION"),
        Some(&Some("false".into()))
    );

    let mut command = std::process::Command::new("buzz-acp");
    super::configure_rollout_gate_environment(&mut command, durable_public.launch_mode());
    let env = configured_env(&command);
    assert_eq!(
        env.get("BUZZ_ACP_DURABLE_RUNTIME"),
        Some(&Some("true".into()))
    );
    assert_eq!(
        env.get("BUZZ_ACP_JOB_EVENT_PUBLICATION"),
        Some(&Some("true".into()))
    );
}

#[test]
fn managed_job_env_explicitly_disables_unavailable_driver_and_roots() {
    let mut command = std::process::Command::new("buzz-acp");

    super::environment::configure_managed_job_environment(&mut command, None, None);
    let env = configured_env(&command);

    assert_eq!(env.get("BUZZ_ACP_LH_COMMAND"), Some(&Some(String::new())));
    assert_eq!(
        env.get("BUZZ_ACP_JOB_WORKSPACE_ROOTS"),
        Some(&Some(String::new()))
    );
}

#[test]
fn managed_job_env_keeps_each_valid_operator_value_when_the_other_is_unavailable() {
    let workspace = tempfile::tempdir().expect("temp dir");
    let canonical_workspace = std::fs::canonicalize(workspace.path()).expect("canonical workspace");
    let expected_roots = std::env::join_paths([&canonical_workspace])
        .expect("valid workspace roots")
        .to_string_lossy()
        .into_owned();
    let executable = std::env::current_exe().expect("current executable");
    let canonical_executable = std::fs::canonicalize(&executable)
        .expect("canonical executable")
        .to_string_lossy()
        .into_owned();

    let mut no_driver = std::process::Command::new("buzz-acp");
    super::environment::configure_managed_job_environment(
        &mut no_driver,
        None,
        Some(workspace.path().as_os_str()),
    );
    let no_driver_env = configured_env(&no_driver);
    assert_eq!(
        no_driver_env.get("BUZZ_ACP_LH_COMMAND"),
        Some(&Some(String::new()))
    );
    assert_eq!(
        no_driver_env.get("BUZZ_ACP_JOB_WORKSPACE_ROOTS"),
        Some(&Some(expected_roots.clone()))
    );

    let mut no_roots = std::process::Command::new("buzz-acp");
    super::environment::configure_managed_job_environment(&mut no_roots, Some(executable), None);
    let no_roots_env = configured_env(&no_roots);
    assert_eq!(
        no_roots_env.get("BUZZ_ACP_LH_COMMAND"),
        Some(&Some(canonical_executable))
    );
    assert_eq!(
        no_roots_env.get("BUZZ_ACP_JOB_WORKSPACE_ROOTS"),
        Some(&Some(String::new()))
    );
}

#[test]
fn build_env_owner_only_sets_mode_and_removes_others() {
    let rec = fixture(RespondTo::OwnerOnly, vec![], Some("tag".into()));
    let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("owner-only")
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    if expected_owner_only() {
        assert_eq!(
            set_map
                .get("BUZZ_ACP_ALLOWED_RESPOND_TO")
                .map(String::as_str),
            Some("owner-only")
        );
        assert!(!remove.contains(&"BUZZ_ACP_ALLOWED_RESPOND_TO"));
    } else {
        assert!(!set_map.contains_key("BUZZ_ACP_ALLOWED_RESPOND_TO"));
        assert!(remove.contains(&"BUZZ_ACP_ALLOWED_RESPOND_TO"));
    }
    // auth_tag is present → no AGENT_OWNER fallback fires.
    assert!(remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

// select_untracked_bundle_harnesses tests live in runtime/sweep.rs (mod tests).

#[test]
fn build_env_allowlist_sets_both_envs_and_joins() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let rec = fixture(
        RespondTo::Allowlist,
        vec![a.clone(), b.clone()],
        Some("tag".into()),
    );
    let (set, _remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some(expected_mode("allowlist")),
        "runtime wrapper did not apply the declared build policy",
    );
    if expected_owner_only() {
        assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    } else {
        assert_eq!(
            set_map
                .get("BUZZ_ACP_RESPOND_TO_ALLOWLIST")
                .map(String::as_str),
            Some(format!("{a},{b}").as_str()),
        );
    }
}

#[test]
fn build_env_anyone_omits_allowlist_var() {
    let rec = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some(expected_mode("anyone")),
        "runtime wrapper did not apply the declared build policy",
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
}

#[test]
fn owner_only_access_policy_overrides_stale_anyone_record_at_runtime() {
    let rec = fixture(RespondTo::Anyone, vec!["a".repeat(64)], Some("tag".into()));
    let (set, remove) = build_respond_to_env_with_policy(&rec, Some("owner"), true).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();

    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("owner-only"),
        "owner-only-access runtime env widened stale access",
    );
    assert_eq!(
        set_map
            .get("BUZZ_ACP_ALLOWED_RESPOND_TO")
            .map(String::as_str),
        Some("owner-only"),
        "owner-only-access runtime env omitted the owner-only guard",
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
}

#[test]
fn build_env_legacy_record_without_auth_tag_emits_agent_owner() {
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (set, remove) = build_respond_to_env(&rec, Some("ownerhex")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_AGENT_OWNER").map(String::as_str),
        Some("ownerhex")
    );
    assert!(!remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_legacy_record_without_owner_hex_removes_agent_owner() {
    // No owner available to forward → make sure we don't inherit a leaked
    // env var from the parent.
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (_set, remove) = build_respond_to_env(&rec, None).unwrap();
    assert!(remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_rejects_corrupted_allowlist() {
    let rec = fixture(
        RespondTo::Allowlist,
        vec!["not-hex".into()],
        Some("tag".into()),
    );
    assert!(build_respond_to_env(&rec, Some("owner")).is_err());
}

#[test]
fn build_env_rejects_empty_allowlist_in_allowlist_mode() {
    let rec = fixture(RespondTo::Allowlist, vec![], Some("tag".into()));
    if expected_owner_only() {
        let (set, _) = build_respond_to_env(&rec, Some("owner")).unwrap();
        let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
        assert_eq!(
            set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
            Some("owner-only")
        );
    } else {
        let err = build_respond_to_env(&rec, Some("owner")).unwrap_err();
        assert!(err.contains("at least one pubkey"));
    }
}

// ── persona fixture helpers ─────────────────────────────────────────

fn persona_with_provider(
    id: &str,
    prompt: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> crate::managed_agents::AgentDefinition {
    crate::managed_agents::AgentDefinition {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-06-09T00:00:00Z".to_string(),
        updated_at: "2026-06-09T00:00:00Z".to_string(),
    }
}

// ── persona env refresh acceptance ──────────────────────────────────────
//
// The refresh lifecycle Wes decided: `record.env_vars` holds agent-level
// overrides only, the live persona env is merged underneath at read time
// (spawn / readiness / deploy), so persona env edits — like prompt/model/
// provider — reach the agent on the next spawn without delete+recreate.
// The merge assertions are load-bearing: they witness the credential refresh
// that the old create-time env baking silently blocked.

use crate::managed_agents::env_vars::{live_persona_env, merged_user_env};
use std::collections::BTreeMap;

/// Apply a persona snapshot onto a record, mirroring `create_managed_agent`:
/// links the record to `persona.id`, then delegates the actual snapshot
/// (prompt/model/provider/runtime/source_version) to the real production
/// `apply_persona_snapshot` — so a change to that function's behavior is
/// exercised by these tests instead of silently diverging from it.
fn pin_persona(record: &mut ManagedAgentRecord, persona: &crate::managed_agents::AgentDefinition) {
    record.persona_id = Some(persona.id.clone());
    crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
}

fn persona_v(
    id: &str,
    prompt: &str,
    env: &[(&str, &str)],
) -> crate::managed_agents::AgentDefinition {
    let mut p = persona_with_provider(id, prompt, Some("model-v"), Some("anthropic"));
    p.env_vars = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    p
}

/// The spawn-time user env for `record` under `personas` — the same
/// live-persona-under-overrides merge `spawn_agent_child` and
/// `resolve_effective_agent_env` perform.
fn spawn_user_env(
    record: &ManagedAgentRecord,
    personas: &[crate::managed_agents::AgentDefinition],
) -> BTreeMap<String, String> {
    merged_user_env(
        &live_persona_env(personas, record.persona_id.as_deref()),
        &record.env_vars,
    )
}

#[test]
fn create_keeps_env_vars_as_overrides_only() {
    let p0 = persona_v("p", "prompt-v0", &[("ANTHROPIC_API_KEY", "key-v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &p0);

    assert_eq!(record.system_prompt.as_deref(), Some("prompt-v0"));
    assert_eq!(record.provider.as_deref(), Some("anthropic"));
    assert!(
        record.env_vars.is_empty(),
        "create must NOT bake persona env into the record — overrides only"
    );
    // The credential still reaches the spawned process via the live merge.
    assert_eq!(
        spawn_user_env(&record, std::slice::from_ref(&p0))
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("key-v0"),
        "spawn env must carry the persona credential via the live merge"
    );
    assert!(record.persona_source_version.is_some());
}

#[test]
fn restart_after_persona_edit_refreshes_credential() {
    // Create from P0.
    let p0 = persona_v("p", "prompt-v0", &[("ANTHROPIC_API_KEY", "key-v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &p0);

    // Edit the persona to P1 (prompt + credential change). Restart reuses the
    // SAME record, but spawn merges the live persona env — so the edited
    // credential reaches the next spawn without delete+recreate.
    let p1 = persona_v("p", "prompt-v1", &[("ANTHROPIC_API_KEY", "key-v1")]);
    assert_eq!(
        spawn_user_env(&record, std::slice::from_ref(&p1))
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("key-v1"),
        "restart must pick up the edited credential — the refresh Wes asked for"
    );

    // The badge flips: the record's snapshot lags the edited persona until the
    // spawn-path re-snapshot runs.
    let (out_of_date, orphaned) = super::persona_drift_state(&record, std::slice::from_ref(&p1));
    assert!(
        out_of_date,
        "edited persona must mark the instance out of date"
    );
    assert!(!orphaned);
}

#[test]
fn agent_env_overrides_win_over_persona_env_at_spawn() {
    // Agent-level env_vars layer over persona env on collision at read time
    // (persona env < agent env).
    let persona = persona_v("p", "prompt", &[("ANTHROPIC_API_KEY", "persona-key")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    record.env_vars = BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "agent-key".to_string())]);
    pin_persona(&mut record, &persona);

    assert_eq!(
        spawn_user_env(&record, std::slice::from_ref(&persona))
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("agent-key"),
        "agent override must win over persona env"
    );
}

#[test]
fn orphaned_agent_refused_at_spawn_boundary() {
    // Persona deleted: `spawn_agent_child` must refuse before any process
    // side effect, not silently degrade to the record's stale overrides.
    // `require_resolved` on the shared resolver is the pure predicate
    // `spawn_agent_child` checks first — this pins the contract without
    // needing a real `AppHandle`.
    let persona = persona_v("p", "prompt", &[("ANTHROPIC_API_KEY", "persona-key")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    record.env_vars = BTreeMap::from([("EXTRA".to_string(), "agent-value".to_string())]);
    pin_persona(&mut record, &persona);

    // The persona is absent from the live catalog — same shape restore/start
    // see when a persona was deleted on another device.
    let no_personas: &[crate::managed_agents::AgentDefinition] = &[];
    let error = crate::managed_agents::effective_config::resolve_effective_config(
        &record,
        no_personas,
        &Default::default(),
    )
    .require_resolved()
    .unwrap_err();
    assert_eq!(
        error,
        crate::managed_agents::effective_config::ORPHANED_INSTANCE_ERROR.to_string(),
        "an orphaned linked instance must be refused, not spawned from its own overrides"
    );
}

#[test]
fn self_heal_drops_overrides_equal_to_persona_value() {
    // Pre-refresh records baked persona env in as pseudo-overrides. The
    // spawn-path retain() treats an override equal to the persona's current
    // value as inherited, so later persona edits refresh it.
    let p0 = persona_v("p", "prompt", &[("ANTHROPIC_API_KEY", "key-v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    record.env_vars = BTreeMap::from([
        ("ANTHROPIC_API_KEY".to_string(), "key-v0".to_string()), // baked-in persona value
        ("GENUINE".to_string(), "override".to_string()),         // real override
    ]);
    pin_persona(&mut record, &p0);

    // Mirror the start/restore self-heal.
    record.env_vars.retain(|k, v| p0.env_vars.get(k) != Some(v));

    assert_eq!(
        record.env_vars.get("ANTHROPIC_API_KEY"),
        None,
        "baked-in persona value must be reclassified as inherited"
    );
    assert_eq!(
        record.env_vars.get("GENUINE").map(String::as_str),
        Some("override"),
        "genuine overrides must survive the self-heal"
    );

    // After the persona edits the key, the healed record refreshes.
    let p1 = persona_v("p", "prompt", &[("ANTHROPIC_API_KEY", "key-v1")]);
    assert_eq!(
        spawn_user_env(&record, std::slice::from_ref(&p1))
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("key-v1"),
        "healed record must inherit the edited persona credential"
    );
}

#[test]
fn deleted_persona_is_orphaned_not_out_of_date() {
    let p0 = persona_v("p", "prompt-v0", &[("KEY", "v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &p0);

    // Persona no longer in the catalog → orphaned, never out of date (no
    // current persona to respawn into).
    let (out_of_date, orphaned) = super::persona_drift_state(&record, &[]);
    assert!(!out_of_date);
    assert!(orphaned);
}

#[test]
fn non_persona_agent_never_drifts() {
    // A hand-built agent (no persona_id) has nothing to drift from.
    let record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    assert_eq!(record.persona_id, None);
    let (out_of_date, orphaned) = super::persona_drift_state(&record, &[]);
    assert!(!out_of_date);
    assert!(!orphaned);
}

use super::runtime_metadata_env_vars;

#[test]
fn runtime_metadata_env_vars_injects_model_and_provider() {
    let vars = runtime_metadata_env_vars(
        Some("GOOSE_MODEL"),
        Some("GOOSE_PROVIDER"),
        false,
        Some("gpt-4o"),
        Some("openai"),
    );
    assert_eq!(
        vars,
        vec![("GOOSE_MODEL", "gpt-4o"), ("GOOSE_PROVIDER", "openai")]
    );
}

#[test]
fn runtime_metadata_env_vars_skips_provider_when_locked() {
    let vars = runtime_metadata_env_vars(
        None, // claude has no model_env_var
        None, // claude has no provider_env_var
        true, // provider_locked = true
        Some("claude-opus-4-7"),
        Some("anthropic"),
    );
    assert!(vars.is_empty());
}

#[test]
fn runtime_metadata_env_vars_injects_model_even_with_acp_model_switching() {
    // buzz-agent has supports_acp_model_switching=true but we still inject
    // the model env var because ACP model switching is post-bootstrap
    let vars = runtime_metadata_env_vars(
        Some("BUZZ_AGENT_MODEL"),
        Some("BUZZ_AGENT_PROVIDER"),
        false,
        Some("goose-claude-4-6-opus"),
        Some("databricks"),
    );
    assert_eq!(
        vars,
        vec![
            ("BUZZ_AGENT_MODEL", "goose-claude-4-6-opus"),
            ("BUZZ_AGENT_PROVIDER", "databricks"),
        ]
    );
}

#[test]
fn claude_spawn_uses_the_probed_cli_executable() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().expect("temp dir");
    let cli = temp
        .path()
        .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&cli, "").expect("write fake cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755))
            .expect("make fake cli executable");
    }
    let original_path = std::env::var_os("PATH");
    std::env::set_var("PATH", temp.path());

    let mut command = std::process::Command::new("buzz-acp");
    super::configure_runtime_cli(&mut command, super::known_acp_runtime("claude-agent-acp"));

    if let Some(path) = original_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }
    assert!(command
        .get_envs()
        .any(|(key, value)| { key == "CLAUDE_CODE_EXECUTABLE" && value == Some(cli.as_os_str()) }));
}

#[test]
fn codex_spawn_does_not_set_a_claude_executable() {
    let mut command = std::process::Command::new("buzz-acp");
    super::configure_runtime_cli(&mut command, super::known_acp_runtime("codex-acp"));
    assert!(!command
        .get_envs()
        .any(|(key, _)| key == "CLAUDE_CODE_EXECUTABLE"));
}

/// On Windows, `.cmd` and `.bat` batch shims must NOT be assigned to
/// `CLAUDE_CODE_EXECUTABLE` — `CreateProcess` cannot exec them directly and
/// returns EINVAL (issue #2397). The adapter must fall back to its own PATH
/// lookup instead.
///
/// These tests exercise `is_batch_shim` directly — a pure path predicate with
/// no global PATH or resolve_command cache involvement — so they run on every
/// host and cannot be poisoned by the `claude_spawn_uses_the_probed_cli_executable`
/// test that runs before them.
#[test]
fn batch_shim_cmd_extension_is_rejected() {
    assert!(
        super::path::is_batch_shim(std::path::Path::new("claude.cmd")),
        "claude.cmd must be identified as a batch shim"
    );
}

#[test]
fn batch_shim_cmd_extension_uppercase_is_rejected() {
    assert!(
        super::path::is_batch_shim(std::path::Path::new("claude.CMD")),
        "claude.CMD must be identified as a batch shim (case-insensitive)"
    );
}

#[test]
fn batch_shim_bat_extension_is_rejected() {
    assert!(
        super::path::is_batch_shim(std::path::Path::new("claude.bat")),
        "claude.bat must be identified as a batch shim"
    );
}

#[test]
fn batch_shim_bat_extension_uppercase_is_rejected() {
    assert!(
        super::path::is_batch_shim(std::path::Path::new("claude.BAT")),
        "claude.BAT must be identified as a batch shim (case-insensitive)"
    );
}

#[test]
fn batch_shim_exe_extension_is_not_rejected() {
    assert!(
        !super::path::is_batch_shim(std::path::Path::new("claude.exe")),
        "claude.exe must not be identified as a batch shim"
    );
}

#[test]
fn batch_shim_no_extension_is_not_rejected() {
    assert!(
        !super::path::is_batch_shim(std::path::Path::new("claude")),
        "claude (no extension) must not be identified as a batch shim"
    );
}

// ── workspace pair-key resolution (summary/stop scoping) ────────────────

#[test]
fn missing_phase_zero_receipt_with_legacy_pid_requires_manual_stop() {
    assert_eq!(
        super::process::classify_missing_legacy_receipt(Some(42), false),
        super::LegacyMigrationGate::ManualLegacyStopRequired
    );
    assert_eq!(
        super::process::classify_missing_legacy_receipt(None, false),
        super::LegacyMigrationGate::Clear
    );
    assert_eq!(
        super::process::classify_missing_legacy_receipt(None, true),
        super::LegacyMigrationGate::ManualLegacyStopRequired
    );
}

#[test]
fn phase_zero_lock_proof_is_bound_to_pair_and_exact_lock_path() {
    let key = super::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
        .expect("valid pair key");
    let lock_path = std::path::Path::new("/tmp/buzz-pair.lock");
    let mut receipt = super::super::LegacyManagedAgentRuntimeReceipt {
        schema_version: buzz_runtime_pkg::LEGACY_RUNTIME_RECEIPT_SCHEMA_VERSION,
        key: key.clone(),
        pid: 42,
        process_start_marker: "marker".into(),
        desktop_instance_id: "desktop-generation".into(),
        started_at: "2026-08-02T00:00:00Z".into(),
        lock_protocol_version: super::super::RUNTIME_LOCK_PROTOCOL_VERSION,
        lock_path_hash: super::super::runtime_lock_path_hash(lock_path),
    };
    assert!(super::process::legacy_receipt_has_lock_proof(
        &receipt, &key, lock_path
    ));

    receipt.lock_path_hash = "00".repeat(32);
    assert!(!super::process::legacy_receipt_has_lock_proof(
        &receipt, &key, lock_path
    ));
    receipt.lock_path_hash = super::super::runtime_lock_path_hash(lock_path);
    receipt.key =
        super::ManagedAgentRuntimeKey::new("bb".repeat(32), "wss://relay.example").unwrap();
    assert!(!super::process::legacy_receipt_has_lock_proof(
        &receipt, &key, lock_path
    ));
}
#[test]
fn automatic_rollout_is_phase_zero_first_then_default_on_v2() {
    let preferred = super::ManagedRuntimeFeatureGates::from_values(None, None).launch_mode();
    assert_eq!(
        super::process::select_rollout_launch_mode(
            preferred,
            false,
            false,
            super::LegacyMigrationGate::Clear,
        ),
        Ok(super::ManagedRuntimeLaunchMode::LegacyPhase0)
    );
    assert_eq!(
        super::process::select_rollout_launch_mode(
            preferred,
            false,
            true,
            super::LegacyMigrationGate::Clear,
        ),
        Ok(super::ManagedRuntimeLaunchMode::DurableV2 {
            job_event_publication: true,
        })
    );
    assert_eq!(
        super::process::select_rollout_launch_mode(
            preferred,
            true,
            false,
            super::LegacyMigrationGate::Clear,
        ),
        Ok(super::ManagedRuntimeLaunchMode::DurableV2 {
            job_event_publication: true,
        })
    );
    assert_eq!(
        super::process::select_rollout_launch_mode(
            preferred,
            false,
            true,
            super::LegacyMigrationGate::ManualLegacyStopRequired,
        ),
        Err(super::LegacyMigrationGate::ManualLegacyStopRequired)
    );
}
#[test]
fn unpinned_record_resolves_pair_key_per_workspace() {
    // Community-scoped truth: an unpinned agent running only on relay A must
    // read as running in workspace A and stopped in workspace B — the pair

    // key the summary looks up differs per workspace.
    let pubkey = "aa".repeat(32);

    let key_a = super::resolve_workspace_pair_key(&pubkey, "", "wss://one.example").unwrap();
    let key_b = super::resolve_workspace_pair_key(&pubkey, "", "wss://two.example").unwrap();

    let runtimes = std::collections::HashMap::from([(key_a.clone(), ())]);
    assert!(runtimes.contains_key(&key_a));
    assert!(!runtimes.contains_key(&key_b));
}

#[test]
fn stored_relay_pin_is_ignored_in_pair_key_resolution() {
    // Legacy pins are ignored (#2122): a record carrying a creation-era
    // `relay_url` resolves the same per-workspace pair key an unpinned record
    // does, so summaries/stop act on the community being viewed.
    let pubkey = "aa".repeat(32);
    let from_a =
        super::resolve_workspace_pair_key(&pubkey, "wss://pinned.example", "wss://one.example")
            .unwrap();
    let from_b =
        super::resolve_workspace_pair_key(&pubkey, "wss://pinned.example", "wss://two.example")
            .unwrap();
    assert_ne!(from_a, from_b);
    assert_eq!(from_a.relay_url, "wss://one.example");
    assert_eq!(from_b.relay_url, "wss://two.example");
}

#[test]
fn legacy_migration_without_lock_proof_requires_manual_stop() {
    assert_eq!(
        super::process::classify_legacy_migration(false, false, false),
        super::LegacyMigrationGate::ManualLegacyStopRequired
    );
}

#[test]
fn legacy_migration_with_held_pair_lock_reports_active_runtime() {
    assert_eq!(
        super::process::classify_legacy_migration(true, true, true),
        super::LegacyMigrationGate::LegacyRuntimeActive
    );
}

#[test]
fn legacy_migration_with_released_pair_lock_allows_cutover() {
    assert_eq!(
        super::process::classify_legacy_migration(true, false, false),
        super::LegacyMigrationGate::Clear
    );
}

#[test]
fn live_legacy_pid_without_the_proven_pair_lock_blocks_cutover() {
    assert_eq!(
        super::process::classify_legacy_migration(true, false, true),
        super::LegacyMigrationGate::ManualLegacyStopRequired
    );
}

#[test]
fn workspace_pair_key_is_canonical() {
    // Spawn stamps the canonical key; lookup must hit the same entry even
    // when the workspace relay is written in a non-canonical form.
    let pubkey = "aa".repeat(32);
    let stamped = super::resolve_workspace_pair_key(&pubkey, "", "wss://one.example").unwrap();
    let viewed = super::resolve_workspace_pair_key(&pubkey, "", "WSS://One.Example:443/").unwrap();
    assert_eq!(stamped, viewed);
}

#[test]
fn invalid_pubkey_resolves_no_pair_key() {
    // Key-less records (keys minted on first start) cannot form a pair key;
    // the summary must fall back to stopped state rather than panic.
    assert!(super::resolve_workspace_pair_key("not-a-key", "", "wss://one.example").is_none());
}

// ── restart_eligible tests ──────────────────────────────────────────────

#[test]
fn restart_eligible_true_when_non_orphan_has_hash_drift() {
    assert!(super::restart_eligible(false, None, false, true, false));
}

#[test]
fn restart_eligible_true_when_non_orphan_has_availability_drift() {
    assert!(super::restart_eligible(false, None, false, false, true));
}

#[test]
fn restart_eligible_false_when_orphan_has_hash_drift() {
    // An orphan can never be restarted successfully — spawn refuses it — so hash drift alone must not surface "Restart required".
    assert!(!super::restart_eligible(false, None, true, true, false));
}

#[test]
fn restart_eligible_false_when_orphan_has_availability_drift() {
    assert!(!super::restart_eligible(false, None, true, false, true));
}

#[test]
fn restart_eligible_false_when_orphan_has_no_drift() {
    assert!(!super::restart_eligible(false, None, true, false, false));
}

#[test]
fn restart_eligible_false_when_non_orphan_has_no_drift() {
    assert!(!super::restart_eligible(false, None, false, false, false));
}

#[test]
fn restart_eligible_defers_all_drift_while_a_job_is_active() {
    assert!(!super::restart_eligible(true, None, false, true, true));
}

#[test]
fn restart_eligible_defers_all_drift_for_every_nonterminal_assignment() {
    use buzz_runtime_pkg::protocol::AssignmentState::{
        Blocked, NeedsApproval, Reading, Recovering, Waiting, Working,
    };

    for state in [
        Reading,
        Working,
        Waiting,
        NeedsApproval,
        Blocked,
        Recovering,
    ] {
        assert!(
            !super::restart_eligible(false, Some(state), false, true, true),
            "{state:?} must fence config-drift restart"
        );
    }
}

#[test]
fn terminal_assignment_does_not_fence_config_drift_restart() {
    use buzz_runtime_pkg::protocol::AssignmentState::{Cancelled, Completed, Failed};
    for state in [Completed, Failed, Cancelled] {
        assert!(super::restart_eligible(
            false,
            Some(state),
            false,
            true,
            false
        ));
    }
}

#[test]
fn fresh_desktop_app_state_reattaches_without_restarting_runtime_or_job() {
    use std::sync::Arc;

    use buzz_runtime_pkg::{
        protocol::{
            ControlError, ControlOperation, ControlPayload, JobState, JobStatus,
            ManagedAgentRuntimeKey as RuntimeKey, PublicationState, RuntimeDiagnostics,
            RuntimeReceipt, RuntimeStatusSnapshot, WorkState, CONTROL_PROTOCOL_VERSION,
            RUNTIME_RECEIPT_SCHEMA_VERSION,
        },
        ControlHandlerFn, ControlServerConfig, RuntimeServer,
    };
    use chrono::Utc;
    use uuid::Uuid;

    let temp = tempfile::tempdir().expect("create Desktop reattach fixture");
    let receipt_path = temp.path().join("runtime-receipt.json");
    let lock_path = temp.path().join("pair.lock");
    std::fs::write(&lock_path, b"").expect("create pair lock path");

    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("ab".repeat(32), "wss://relay.example")
            .expect("build canonical pair key");
    let runtime_id = key.runtime_id();
    let generation = Uuid::new_v4();
    let job_id = Uuid::new_v4();

    #[cfg(unix)]
    let mut runner = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn long-running job fixture");
    #[cfg(windows)]
    let mut runner = std::process::Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .spawn()
        .expect("spawn long-running job fixture");
    let runner_pid = runner.id();
    let runner_marker =
        buzz_runtime_pkg::process_start_marker(runner_pid).expect("read runner start marker");

    let job = JobStatus {
        job_id,
        request_event_id: Some("cd".repeat(32)),
        source_event_id: Some("ef".repeat(32)),
        channel_id: Uuid::new_v4(),
        state: JobState::Running,
        attempt: 1,
        progress_seq: 1,
        summary: "governed job remains active".into(),
        started_at: Some(Utc::now()),
        finished_at: None,
        exit_code: None,
        error_code: None,
        publication_state: PublicationState::Published,
        runner_pid: Some(runner_pid),
        runner_start_marker: Some(runner_marker),
    };
    let status = RuntimeStatusSnapshot {
        runtime_id: runtime_id.clone(),
        generation,
        work_state: WorkState::Working,
        recovering: false,
        recovery_reason: None,
        queued_inbox: 0,
        in_turn_inbox: 0,
        dead_letter_inbox: 0,
        capacity_rejections: 0,
        active_assignment: None,
        active_job: Some(job_id),
        active_jobs: vec![job_id],
        diagnostics: RuntimeDiagnostics::default(),
    };
    let server_config = ControlServerConfig::new(runtime_id.clone(), generation);
    let controller_token = server_config.controller_token.clone();
    let model_token = server_config.model_token.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let server_status = status.clone();
    let server_job = job.clone();
    let server_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime control fixture");
        runtime.block_on(async move {
            let server = RuntimeServer::bind(server_config)
                .await
                .expect("bind runtime control fixture");
            ready_tx
                .send(server.local_addr().expect("read runtime control address"))
                .expect("publish runtime control address");
            let handler = Arc::new(ControlHandlerFn(move |_capability, operation| {
                let status = server_status.clone();
                let job = server_job.clone();
                async move {
                    match operation {
                        ControlOperation::Status => Ok(ControlPayload::Status(status)),
                        ControlOperation::JobsStatus { job_id } if job_id == job.job_id => {
                            Ok(ControlPayload::Job(job))
                        }
                        _ => Err(ControlError::new(
                            "unsupported",
                            "unsupported test operation",
                        )),
                    }
                }
            }));
            tokio::select! {
                result = server.serve(handler) => {
                    result.expect("serve runtime control fixture");
                }
                _ = stop_rx => {}
            }
        });
    });
    let control_addr = ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("runtime control fixture must start");
    let runtime_pid = std::process::id();
    let receipt = RuntimeReceipt {
        schema_version: RUNTIME_RECEIPT_SCHEMA_VERSION,
        key: RuntimeKey {
            pubkey: key.pubkey.clone(),
            relay_url: key.relay_url.clone(),
        },
        runtime_id,
        pid: runtime_pid,
        process_start_marker: buzz_runtime_pkg::process_start_marker(runtime_pid)
            .expect("read runtime process marker"),
        generation,
        control_addr,
        controller_token,
        model_token,
        started_at: Utc::now(),
        protocol_version: CONTROL_PROTOCOL_VERSION,
        lock_protocol_version: crate::managed_agents::RUNTIME_LOCK_PROTOCOL_VERSION,
        lock_path_hash: crate::managed_agents::runtime_lock_path_hash(&lock_path),
        ready: true,
    };
    buzz_runtime_pkg::write_runtime_receipt(&receipt_path, &receipt)
        .expect("write authenticated runtime receipt");

    let adopt = || {
        let state = crate::app_state::build_app_state();
        let (receipt, controller, status) = super::adopt_schema_v2_runtime(&receipt_path, &key)
            .expect("fresh Desktop state must authenticate runtime receipt");
        let active_job = tauri::async_runtime::block_on(controller.jobs_status(job_id))
            .expect("fresh Desktop state must recover active job");
        let pair = crate::managed_agents::ManagedAgentPairRuntime::connected(
            None,
            receipt,
            receipt_path.clone(),
            controller,
            &status,
            Some(active_job),
        );
        let observed = (
            pair.pid(),
            pair.active_job
                .as_ref()
                .and_then(|job| job.runner_pid)
                .expect("reattached Desktop must retain runner PID"),
            status.generation,
        );
        state
            .managed_agent_processes
            .lock()
            .expect("lock fresh Desktop runtime registry")
            .insert(key.clone(), pair);
        (state, observed)
    };

    let (first_app_state, first) = adopt();
    drop(first_app_state);
    let (relaunched_app_state, second) = adopt();
    assert_eq!(second, first, "Desktop relaunch must adopt, never respawn");
    assert_eq!(second.0, runtime_pid);
    assert_eq!(second.1, runner_pid);
    assert_eq!(second.2, generation);
    drop(relaunched_app_state);

    let _ = stop_tx.send(());
    server_thread.join().expect("join runtime control fixture");
    let _ = runner.kill();
    let _ = runner.wait();
}
