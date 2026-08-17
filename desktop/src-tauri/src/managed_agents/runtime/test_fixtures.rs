use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

pub(super) const EXPECTED_ACCESS_ENV: &str = "BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY";

pub(super) fn expected_owner_only() -> bool {
    match std::env::var(EXPECTED_ACCESS_ENV) {
        Ok(value) => value
            .parse::<bool>()
            .unwrap_or_else(|_| panic!("{EXPECTED_ACCESS_ENV} must be true or false")),
        Err(std::env::VarError::NotPresent)
            if !crate::managed_agents::owner_only_access_build() =>
        {
            false
        }
        Err(std::env::VarError::NotPresent) => {
            panic!("{EXPECTED_ACCESS_ENV} must be set for owner-only-access-build tests")
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("{EXPECTED_ACCESS_ENV} must be valid UTF-8")
        }
    }
}

pub(super) fn expected_mode(oss_mode: &'static str) -> &'static str {
    if expected_owner_only() {
        "owner-only"
    } else {
        oss_mode
    }
}

/// Construct a minimal record fixture for runtime tests.
pub(super) fn fixture(
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
        provider_policy_pending: false,
        provider_binary_path: None,
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

pub(crate) fn make_pair_runtime_with_connect_url(
    connect_relay_url: &str,
) -> crate::managed_agents::ManagedAgentPairRuntime {
    use std::process::{Command, Stdio};
    // Spawn a real child so ManagedAgentProcess's Child field is satisfied.
    // `true` exits immediately with 0 — just a handle we need for type purposes.
    //
    // Absolute `/usr/bin/true` on unix (present on both macOS and Linux):
    // parallel tests holding `lock_path_mutex` swap PATH to a tempdir, and a
    // bare `true` lookup during that window fails with NotFound (observed
    // flake). Windows keeps the PATH lookup — no test there swaps PATH.
    #[cfg(unix)]
    let program = "/usr/bin/true";
    #[cfg(windows)]
    let program = "true";
    let child = Command::new(program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn true for placeholder");
    let process = crate::managed_agents::ManagedAgentProcess {
        child,
        log_path: std::path::PathBuf::new(),
        connect_relay_url: connect_relay_url.to_string(),
        spawn_config: crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
            &minimal_record(&"cc".repeat(32)),
            &[],
            &[],
            "wss://relay.example",
            &Default::default(),
        ),
        setup_mode: false,
        adapter_availability: None,
        start_nonce: "test-nonce".to_string(),
        #[cfg(windows)]
        job: None,
    };
    crate::managed_agents::ManagedAgentPairRuntime::starting(process)
}

pub(super) fn minimal_record(pubkey: &str) -> crate::managed_agents::ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "test",
            "private_key_nsec": "nsec1fake",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "model": null,
            "provider": null,
            "env_vars": {{}},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }}"#
    ))
    .expect("minimal_record fixture")
}

pub(super) fn receipt_fixture(
    key: crate::managed_agents::ManagedAgentRuntimeKey,
) -> crate::managed_agents::ManagedAgentRuntimeReceipt {
    crate::managed_agents::ManagedAgentRuntimeReceipt {
        key,
        pid: std::process::id(),
        desktop_instance_id: "test-instance".into(),
        started_at: "now".into(),
        connect_relay_url: None,
    }
}
