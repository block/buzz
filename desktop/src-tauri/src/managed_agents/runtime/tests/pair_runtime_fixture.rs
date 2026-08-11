use std::process::{Command, Stdio};

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

pub(super) fn make_pair_runtime_placeholder() -> crate::managed_agents::ManagedAgentPairRuntime {
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
        isolation_run: None,
        #[cfg(windows)]
        job: None,
    };
    crate::managed_agents::ManagedAgentPairRuntime::starting(process)
}
