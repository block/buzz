#[test]
fn runtime_catalog_projects_capability_facts_for_ipc() {
    let runtime = super::known_acp_runtime_exact("buzz-agent")
        .expect("buzz-agent runtime metadata should exist");
    let entry = super::discover_acp_runtime_phase1(runtime).entry;

    assert_eq!(
        entry.capabilities.supports_acp_native_config,
        runtime.supports_acp_native_config
    );
    assert_eq!(
        entry.capabilities.supports_acp_model_switching,
        runtime.supports_acp_model_switching
    );
    assert_eq!(entry.capabilities.mcp_hooks, runtime.mcp_hooks);

    let serialized = serde_json::to_value(entry).expect("catalog entry should serialize");
    assert_eq!(
        serialized["supports_acp_native_config"],
        runtime.supports_acp_native_config
    );
    assert_eq!(
        serialized["supports_acp_model_switching"],
        runtime.supports_acp_model_switching
    );
    assert_eq!(serialized["mcp_hooks"], runtime.mcp_hooks);
}
