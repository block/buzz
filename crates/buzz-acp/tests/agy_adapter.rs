#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

fn test_directory() -> PathBuf {
    std::env::temp_dir().join(format!("buzz-agy-adapter-test-{}", uuid::Uuid::new_v4()))
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write fake AGY executable");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .expect("mark fake AGY executable");
}

async fn spawn_adapter(fake_agy: &Path) -> Child {
    spawn_adapter_with_app_data(fake_agy, None).await
}

async fn spawn_adapter_with_app_data(fake_agy: &Path, app_data: Option<&Path>) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command
        .arg("agy-acp")
        .env("BUZZ_AGY_COMMAND", fake_agy)
        .env("BUZZ_PRIVATE_KEY", "nsec-test-identity")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(app_data) = app_data {
        command.env("BUZZ_AGY_APP_DATA_DIR", app_data);
    }
    command.spawn().expect("spawn AGY adapter")
}

async fn write_request(child: &mut Child, request: Value) {
    let stdin = child.stdin.as_mut().expect("adapter stdin");
    stdin
        .write_all(
            serde_json::to_string(&request)
                .expect("serialize")
                .as_bytes(),
        )
        .await
        .expect("write request");
    stdin.write_all(b"\n").await.expect("terminate request");
    stdin.flush().await.expect("flush request");
}

async fn read_response(reader: &mut BufReader<tokio::process::ChildStdout>) -> Value {
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut line),
    )
    .await
    .expect("adapter response timed out")
    .expect("read adapter response");
    assert!(!line.is_empty(), "adapter exited before responding");
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}

async fn read_until_id(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    expected_id: u64,
) -> Vec<Value> {
    let mut messages = Vec::new();
    loop {
        let message = read_response(reader).await;
        let done = message.get("id") == Some(&json!(expected_id));
        messages.push(message);
        if done {
            return messages;
        }
    }
}

async fn initialize_session(
    child: &mut Child,
    reader: &mut BufReader<tokio::process::ChildStdout>,
    cwd: &Path,
) -> String {
    write_request(
        child,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": 2, "clientCapabilities": {} }
        }),
    )
    .await;
    let initialize = read_response(reader).await;
    assert_eq!(initialize["result"]["agentInfo"]["name"], "Antigravity");

    write_request(
        child,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": {
                "cwd": cwd,
                "mcpServers": [],
                "systemPrompt": "Use the Buzz CLI."
            }
        }),
    )
    .await;
    read_response(reader).await["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_string()
}

#[tokio::test]
async fn bridges_print_output_identity_and_hooks_to_acp() {
    let directory = test_directory();
    std::fs::create_dir_all(&directory).expect("create test directory");
    let fake_agy = directory.join("agy");
    write_executable(
        &fake_agy,
        r#"#!/bin/sh
set -eu
last=""
previous=""
resume="none"
workspace="missing"
for arg in "$@"; do
  if [ "$previous" = "--conversation" ]; then resume="$arg"; fi
  if [ "$previous" = "--add-dir" ]; then workspace="$arg"; fi
  previous="$arg"
  last="$arg"
done
printf '%s' '{"conversationId":"conv-1","stepIdx":3,"transcriptPath":"/tmp/transcript.jsonl","artifactDirectoryPath":"/tmp/artifacts","toolCall":{"name":"replace_file_content","args":{"TargetFile":"src/lib.rs","ReplacementContent":"new"}}}' \
  | buzz-acp agy-hook pre-tool-use >/dev/null
printf '%s' '{"conversationId":"conv-1","stepIdx":3,"transcriptPath":"/tmp/transcript.jsonl","artifactDirectoryPath":"/tmp/artifacts","error":""}' \
  | buzz-acp agy-hook post-tool-use >/dev/null
printf '%s' '{"conversationId":"conv-1","executionNum":1,"terminationReason":"model_stop","error":"","fullyIdle":true,"transcriptPath":"/tmp/transcript.jsonl","artifactDirectoryPath":"/tmp/artifacts"}' \
  | buzz-acp agy-hook stop >/dev/null
printf 'fake response (%s, resume=%s, workspace=%s): %s\n' \
  "${BUZZ_PRIVATE_KEY:-missing}" "$resume" "$workspace" "$last"
"#,
    );

    let mut child = spawn_adapter(&fake_agy).await;
    let stdout = child.stdout.take().expect("adapter stdout");
    let mut reader = BufReader::new(stdout);
    let session_id = initialize_session(&mut child, &mut reader, &directory).await;

    write_request(
        &mut child,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "Make the change." }]
            }
        }),
    )
    .await;

    let messages = read_until_id(&mut reader, 3).await;

    let tool_start = messages
        .iter()
        .position(|message| {
            message["params"]["update"]["sessionUpdate"] == "tool_call"
                && message["params"]["update"]["kind"] == "edit"
        })
        .expect("edit tool start");
    let tool_end = messages
        .iter()
        .position(|message| {
            message["params"]["update"]["sessionUpdate"] == "tool_call_update"
                && message["params"]["update"]["status"] == "completed"
        })
        .expect("edit tool completion");
    assert!(tool_start < tool_end);
    assert_eq!(
        messages[tool_start]["params"]["update"]["_meta"]["antigravity"]["transcriptPath"],
        "/tmp/transcript.jsonl"
    );
    assert!(messages.iter().any(|message| {
        message["params"]["update"]["content"]["text"]
            .as_str()
            .is_some_and(|text| {
                text.contains("fake response (nsec-test-identity,")
                    && text.contains("resume=none")
                    && text.contains(&format!("workspace={}", directory.display()))
                    && text.contains("Make the change.")
            })
    }));
    assert_eq!(
        messages.last().expect("prompt response")["result"]["stopReason"],
        "end_turn"
    );

    write_request(
        &mut child,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "Continue." }]
            }
        }),
    )
    .await;
    let resumed_messages = read_until_id(&mut reader, 4).await;
    assert!(resumed_messages.iter().any(|message| {
        message["params"]["update"]["content"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("resume=conv-1") && text.contains("Continue."))
    }));

    child.kill().await.expect("stop adapter");
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn cancellation_terminates_the_active_antigravity_process() {
    let directory = test_directory();
    std::fs::create_dir_all(&directory).expect("create test directory");
    let fake_agy = directory.join("agy");
    write_executable(&fake_agy, "#!/bin/sh\nexec sleep 30\n");

    let mut child = spawn_adapter(&fake_agy).await;
    let stdout = child.stdout.take().expect("adapter stdout");
    let mut reader = BufReader::new(stdout);
    let session_id = initialize_session(&mut child, &mut reader, &directory).await;

    write_request(
        &mut child,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "Wait." }]
            }
        }),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    write_request(
        &mut child,
        json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": session_id }
        }),
    )
    .await;

    let response = read_response(&mut reader).await;
    assert_eq!(response["id"], 3);
    assert_eq!(response["result"]["stopReason"], "cancelled");

    child.kill().await.expect("stop adapter");
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn resumes_new_trajectory_from_documented_transcript_path_without_hooks() {
    let directory = test_directory();
    std::fs::create_dir_all(&directory).expect("create test directory");
    let app_data = directory.join("agy-data");
    let fake_agy = directory.join("agy");
    write_executable(
        &fake_agy,
        r#"#!/bin/sh
set -eu
previous=""
resume="none"
for arg in "$@"; do
  if [ "$previous" = "--conversation" ]; then resume="$arg"; fi
  previous="$arg"
done
trajectory_id="814807b5-4002-4f04-82b6-d85c0ed87d04"
transcript_dir="$BUZZ_AGY_APP_DATA_DIR/brain/$trajectory_id/.system_generated/logs"
mkdir -p "$transcript_dir"
: > "$transcript_dir/transcript.jsonl"
printf 'resume=%s\n' "$resume"
"#,
    );

    let mut child = spawn_adapter_with_app_data(&fake_agy, Some(&app_data)).await;
    let stdout = child.stdout.take().expect("adapter stdout");
    let mut reader = BufReader::new(stdout);
    let session_id = initialize_session(&mut child, &mut reader, &directory).await;

    write_request(
        &mut child,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "First." }]
            }
        }),
    )
    .await;
    let first = read_until_id(&mut reader, 3).await;
    assert!(first
        .iter()
        .any(|message| { message["params"]["update"]["content"]["text"] == "resume=none\n" }));

    write_request(
        &mut child,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "Second." }]
            }
        }),
    )
    .await;
    let second = read_until_id(&mut reader, 4).await;
    assert!(second.iter().any(|message| {
        message["params"]["update"]["content"]["text"]
            == "resume=814807b5-4002-4f04-82b6-d85c0ed87d04\n"
    }));

    child.kill().await.expect("stop adapter");
    let _ = std::fs::remove_dir_all(directory);
}
