const CONTENT_CANARY: &str = "BUZZ_TRACE_CONTENT_CANARY_7f4d21";
const THOUGHT_CANARY: &str = "BUZZ_TRACE_THOUGHT_CANARY_67ab09";
const TOOL_CANARY: &str = "BUZZ_TRACE_TOOL_OUTPUT_CANARY_5cc820";
const PRIVATE_KEY_CANARY: &str = "BUZZ_TRACE_PRIVATE_KEY_CANARY_b5500a";
const AUTH_TAG_CANARY: &str = "BUZZ_TRACE_AUTH_TAG_CANARY_e06fac";
const BEARER_CANARY: &str = "BUZZ_TRACE_BEARER_CANARY_d38891";
const CAPABILITY_CANARY: &str = "BUZZ_TRACE_CAPABILITY_CANARY_c1a70e";
const SYSTEM_PROMPT_CANARY: &str = "BUZZ_TRACE_SYSTEM_PROMPT_CANARY_239dbc";
const MCP_ENV_CANARY: &str = "BUZZ_TRACE_MCP_ENV_CANARY_3e7771";
const TITLE_CANARY: &str = "BUZZ_TRACE_TITLE_CANARY_08ab51";
const KIND_CANARY: &str = "BUZZ_TRACE_KIND_CANARY_903fd2";
const TOOL_ID_CANARY: &str = "BUZZ_TRACE_TOOL_ID_CANARY_921ced";
const STATUS_CANARY: &str = "BUZZ_TRACE_STATUS_CANARY_4955aa";
const COMMAND_CANARY: &str = "BUZZ_TRACE_COMMAND_CANARY_dbe712";
const RUN_ID_CANARY: &str = "BUZZ_TRACE_RUN_ID_CANARY_8f342c";
const UPDATE_TYPE_CANARY: &str = "BUZZ_TRACE_UPDATE_TYPE_CANARY_ca2851";
const CHILD_STDOUT_CANARY: &str = "BUZZ_TRACE_CHILD_STDOUT_CANARY_c8bc95";
const CHILD_STDERR_CANARY: &str = "BUZZ_TRACE_CHILD_STDERR_CANARY_3d73be";

#[tokio::test]
async fn trace_logs_never_expose_content_or_credentials() {
    let canaries = [
        ("content", CONTENT_CANARY),
        ("thought", THOUGHT_CANARY),
        ("tool_output", TOOL_CANARY),
        ("private_key", PRIVATE_KEY_CANARY),
        ("auth_tag", AUTH_TAG_CANARY),
        ("bearer", BEARER_CANARY),
        ("adapter_capability", CAPABILITY_CANARY),
        ("hostile_system_prompt", SYSTEM_PROMPT_CANARY),
        ("mcp_env", MCP_ENV_CANARY),
        ("title", TITLE_CANARY),
        ("kind", KIND_CANARY),
        ("tool_id", TOOL_ID_CANARY),
        ("status", STATUS_CANARY),
        ("command", COMMAND_CANARY),
        ("run_id", RUN_ID_CANARY),
        ("update_type", UPDATE_TYPE_CANARY),
        ("child_stdout", CHILD_STDOUT_CANARY),
        ("child_stderr", CHILD_STDERR_CANARY),
    ];

    let captured = buzz_acp::run_trace_redaction_probe_for_test(&canaries)
        .await
        .expect("real-process ACP trace probe");

    assert!(captured.status_success, "the ACP canary turn must complete");
    for marker in [
        "title_hash",
        "kind_hash",
        "tool_id_hash",
        "status_hash",
        "command_count",
        "run_id_hash",
        "update_type_hash",
        "agent child stderr line",
    ] {
        assert!(
            captured.stderr.contains(marker),
            "real-process capture did not exercise redacted marker {marker}: {}",
            captured.stderr,
        );
    }
    for (class, canary) in canaries {
        assert!(
            !captured.stdout.contains(canary) && !captured.stderr.contains(canary),
            "RUST_LOG=trace leaked {class} canary",
        );
    }
}
