//! ACP connection EOF must join session creation and MCP transport cleanup.
mod common;
use common::Harness;
use serde_json::json;
use std::{path::Path, time::Duration};

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn eof_joins_session_creation_and_mcp_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let pid = dir.path().join("pid");
    let eof = dir.path().join("eof");
    let mut agent = Harness::spawn("http://127.0.0.1:1").await;
    agent
        .send(
            "session/new",
            json!({"cwd":dir.path(), "mcpServers":[{
                "name":"fixture", "command":env!("CARGO_BIN_EXE_fake-mcp"), "args":[],
                "env":[{"name":"FAKE_MCP_INIT_DELAY_MS","value":"300"},
                    {"name":"FAKE_MCP_PID_FILE","value":pid},
                    {"name":"FAKE_MCP_EOF_FILE","value":eof}]
            }]}),
        )
        .await;
    // EOF during initialization, not just after a registered session is idle.
    wait_file(&pid).await;
    agent.finish_connection().await;
    assert_eq!(std::fs::read_to_string(eof).unwrap(), "cleanup complete");
}

#[tokio::test]
async fn partial_registry_error_awaits_previously_started_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let eof = dir.path().join("eof");
    let mut agent = Harness::spawn("http://127.0.0.1:1").await;
    let id = agent
        .send(
            "session/new",
            json!({"cwd":dir.path(), "mcpServers":[{
        "name":"fixture", "command":env!("CARGO_BIN_EXE_fake-mcp"), "args":[],
        "env":[{"name":"FAKE_MCP_EOF_FILE","value":eof}]
    }, {"name":"invalid__name", "command":"not-executed", "args":[], "env":[]}]}),
        )
        .await;
    let result = agent.recv_until(|value| value["id"] == id).await;
    assert!(result.get("error").is_some(), "{result}");
    assert_eq!(std::fs::read_to_string(eof).unwrap(), "cleanup complete");
    agent.finish_connection().await;
}

async fn shutdown_result(env: Vec<serde_json::Value>) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Harness::spawn("http://127.0.0.1:1").await;
    let id = agent
        .send(
            "session/new",
            json!({"cwd":dir.path(), "mcpServers":[{
                "name":"fixture", "command":env!("CARGO_BIN_EXE_fake-mcp"), "args":[], "env":env
            }]}),
        )
        .await;
    let session = agent.recv_until(|v| v["id"] == id).await;
    assert!(session.get("result").is_some(), "{session}");
    let id = agent.send("_buzz/shutdown_v1", json!({})).await;
    let response = agent.recv_until(|v| v["id"] == id).await;
    agent.finish_connection().await;
    response["result"]["ownedWorkStopped"].as_bool().unwrap()
}

#[tokio::test]
async fn supported_shutdown_requires_both_explicit_work_result_and_successful_exit() {
    let capability = json!({"name":"FAKE_MCP_NAMED_TOOLS","value":"_buzz_shutdown_v1"});
    assert!(shutdown_result(vec![capability.clone()]).await);
    assert!(
        !shutdown_result(vec![]).await,
        "exit zero without capability is unknown"
    );
    assert!(
        !shutdown_result(vec![
            capability.clone(),
            json!({"name":"FAKE_MCP_FAIL_EXIT","value":"1"})
        ])
        .await
    );
    assert!(
        !shutdown_result(vec![
            capability,
            json!({"name":"FAKE_MCP_HANG_EXIT","value":"1"})
        ])
        .await,
        "rmcp timeout-kill is not a successful teardown"
    );
}
