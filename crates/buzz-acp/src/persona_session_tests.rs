use super::*;

// These exercise the actual JSON-RPC serializer and hook gate, not a parallel
// model of the launch payload. They do not claim a real harness executed tools.
async fn capture_client(root: &std::path::Path, response: &str) -> AcpClient {
    AcpClient::spawn("/bin/sh", &["-c".into(), format!(
        "IFS= read -r line; printf '%s' \"$line\" > \"$CAPTURE\"; printf '%s\\n' '{response}'; sleep 10"
    )], &[("CAPTURE".into(), root.join("request.json").to_string_lossy().into_owned())], false).await.unwrap()
}

#[tokio::test]
async fn assigned_plugin_mcp_prompt_and_title_survive_session_serialization() {
    let root = tempfile::tempdir().unwrap();
    let mut client = capture_client(
        root.path(),
        r#"{"jsonrpc":"2.0","id":0,"result":{"sessionId":"fixture"}}"#,
    )
    .await;
    let observer = ObserverHandle::in_process();
    client.set_observer(Some(observer.clone()), 0);
    client.claude_thinking_summaries = true;
    let server = McpServer {
        name: "probe".into(),
        command: "node".into(),
        args: vec!["two words".into()],
        env: vec![EnvVar {
            name: "ROLE".into(),
            value: "reviewer".into(),
        }],
    };
    client
        .session_new_with_plugins(
            "/work",
            vec![server],
            Some(SystemPromptTransport::ClaudeMeta("persona instructions")),
            Some("Reviewer"),
            &["/task/plugin".into()],
            &[],
        )
        .await
        .unwrap();
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("request.json")).unwrap()).unwrap();
    let p = &request["params"];
    assert_eq!(p["cwd"], "/work");
    assert_eq!(p["mcpServers"][0]["args"][0], "two words");
    assert_eq!(p["mcpServers"][0]["env"][0]["value"], "reviewer");
    assert_eq!(p["_meta"]["systemPrompt"]["append"], "persona instructions");
    assert_eq!(p["_meta"]["sessionTitle"], "Reviewer");
    assert_eq!(
        p["_meta"]["claudeCode"]["options"]["plugins"][0]["path"],
        "/task/plugin"
    );
    assert_eq!(
        p["_meta"]["claudeCode"]["options"]["extraArgs"]["thinking-display"],
        "summarized"
    );
    let written = observer
        .snapshot()
        .into_iter()
        .find(|e| e.kind == "acp_write")
        .unwrap();
    assert_eq!(
        written.payload["params"]["mcpServers"],
        "[redacted MCP configuration]"
    );
    assert!(!written.payload.to_string().contains("reviewer"));
    assert!(!written.payload.to_string().contains("two words"));
    assert_eq!(
        crate::persona_capabilities::diagnostic_wire_value(&request),
        written.payload
    );
    client.shutdown().await;
}

#[tokio::test]
async fn codex_native_skill_roots_are_sent_as_additional_directories() {
    let root = tempfile::tempdir().unwrap();
    let mut client = capture_client(
        root.path(),
        r#"{"jsonrpc":"2.0","id":0,"result":{"sessionId":"fixture"}}"#,
    )
    .await;
    client
        .session_new_with_plugins(
            "/work",
            vec![],
            None,
            Some("Codex persona"),
            &[],
            &["/task/native-skills".into()],
        )
        .await
        .unwrap();
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("request.json")).unwrap()).unwrap();
    assert_eq!(
        request["params"]["additionalDirectories"],
        serde_json::json!(["/task/native-skills"])
    );
    assert!(request["params"]["_meta"].get("claudeCode").is_none());
    assert_eq!(request["params"]["cwd"], "/work");
    client.shutdown().await;
}

#[tokio::test]
async fn message_hook_observes_exact_text_and_output_does_not_modify_wire_prompt() {
    let root = tempfile::tempdir().unwrap();
    let hook = root.path().join("message.sh");
    std::fs::write(
        &hook,
        "cat > received; printf 'INJECTION MUST NOT APPEAR'\n",
    )
    .unwrap();
    let mut client = capture_client(
        root.path(),
        r#"{"jsonrpc":"2.0","id":0,"result":{"stopReason":"end_turn"}}"#,
    )
    .await;
    client.set_persona_message_hook(
        Some(hook.to_string_lossy().into_owned()),
        root.path().to_string_lossy().into_owned(),
    );
    client
        .session_prompt_blocks_with_idle_timeout(
            "fixture",
            &["/review", "context 日本語"],
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("received")).unwrap(),
        "/review\n\ncontext 日本語"
    );
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("request.json")).unwrap()).unwrap();
    assert_eq!(request["params"]["prompt"][0]["text"], "/review");
    assert_eq!(request["params"]["prompt"][1]["text"], "context 日本語");
    assert!(!request.to_string().contains("INJECTION"));
    client.shutdown().await;
}

#[tokio::test]
async fn failed_message_hook_writes_no_prompt() {
    let root = tempfile::tempdir().unwrap();
    let hook = root.path().join("message.sh");
    std::fs::write(&hook, "exit 7\n").unwrap();
    let mut client = capture_client(root.path(), "{}").await;
    client.set_persona_message_hook(
        Some(hook.to_string_lossy().into_owned()),
        root.path().to_string_lossy().into_owned(),
    );
    let err = client
        .session_prompt_blocks_with_idle_timeout(
            "fixture",
            &["must not be sent"],
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(3),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AcpError::PersonaHookRejected(_)));
    assert!(!root.path().join("request.json").exists());
    client.shutdown().await;
}

#[tokio::test]
async fn persona_message_hook_also_blocks_native_steering_before_wire_write() {
    let root = tempfile::tempdir().unwrap();
    let hook = root.path().join("reject.sh");
    std::fs::write(&hook, "exit 7\n").unwrap();
    let mut client = capture_client(root.path(), "{}").await;
    client.steering_supported = true;
    client.set_persona_message_hook(
        Some(hook.to_string_lossy().into_owned()),
        root.path().to_string_lossy().into_owned(),
    );
    let (written, ack) =
        super::tests::run_one_steer(&mut client, &root.path().join("request.json")).await;
    assert!(written.is_none());
    assert!(matches!(
        ack,
        crate::pool::SteerAck::Err(crate::pool::SteerError::PersonaHookRejected)
    ));
    client.shutdown().await;
}
