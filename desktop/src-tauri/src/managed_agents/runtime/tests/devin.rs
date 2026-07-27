use crate::managed_agents::known_acp_runtime;

#[test]
fn uses_native_acp_without_mcp_hooks() {
    let runtime = known_acp_runtime("/usr/local/bin/devin").expect("should resolve");
    assert_eq!(runtime.id, "devin");
    assert_eq!(runtime.default_args, &["acp"]);
    assert!(!runtime.defer_agent_start_until_work);
    assert_eq!(runtime.default_idle_timeout_secs, Some(120));
    assert!(runtime.default_env.is_empty());
    assert_eq!(
        runtime.enforced_env,
        &[
            ("BUZZ_ACP_PERMISSION_MODE", "default"),
            ("BUZZ_ACP_AUTO_APPROVE_PERMISSIONS", "false"),
            ("BUZZ_ACP_INTERACTIVE_PERMISSIONS", "true"),
            ("BUZZ_ACP_SELF_PUBLISH_COMPLETION_GRACE", "30"),
        ]
    );
    assert_eq!(runtime.scrub_env_vars, &["WINDSURF_API_KEY", "ACP_BACKEND"]);
    assert!(!runtime.mcp_hooks);
    assert_eq!(runtime.mcp_command, None);
}

#[test]
fn permission_default_does_not_change_existing_runtimes() {
    assert_eq!(
        known_acp_runtime("goose")
            .expect("Goose runtime")
            .default_env,
        &[("GOOSE_MODE", "auto")]
    );
    for command in ["claude-agent-acp", "codex-acp", "buzz-agent"] {
        let runtime = known_acp_runtime(command).expect("existing runtime");
        assert!(runtime.default_env.is_empty());
        assert!(runtime.enforced_env.is_empty());
        assert!(runtime.scrub_env_vars.is_empty());
        assert!(runtime.defer_agent_start_until_work);
        assert_eq!(runtime.default_idle_timeout_secs, None);
    }
}

#[test]
fn process_name_resolves_from_the_runtime_catalog() {
    assert!(super::super::name_matches_known_binary("devin"));
}
