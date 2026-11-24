//! Shared assertions for the inventory CLI and persisted workflow.

use serde_json::Value;
use std::process::Output;
use tokio::process::Command;

async fn workflow_output(url: &str, args: &[&str]) -> Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        Command::new(super::admin_binary())
            .arg("user-deletions")
            .args(args)
            .env("DATABASE_URL", url)
            .env("BUZZ_DB_POOL_SIZE", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}

pub async fn workflow_cli(url: &str, args: &[&str], success: bool) -> Value {
    let output = workflow_output(url, args).await;
    assert_eq!(
        output.status.success(),
        success,
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if success {
        serde_json::from_slice(&output.stdout).unwrap()
    } else {
        Value::Null
    }
}

async fn workflow_refuses(url: &str, args: &[&str], expected: &str) {
    let output = workflow_output(url, args).await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
    assert!(stderr.contains(expected), "{args:?}: {stderr}");
}

pub async fn step_real_workflow_past_gate(url: &str, id: &str) -> Value {
    let approved = workflow_cli(url, &["inspect", id], true).await;
    assert_eq!(approved["status"], "approved");
    assert!(approved["stage_history"].as_array().unwrap().is_empty());
    workflow_refuses(
        url,
        &["run", id, "--next-step"],
        "workflow execution requires --delete",
    )
    .await;
    // Refusing the missing flag must happen before claiming or initializing.
    assert_eq!(workflow_cli(url, &["inspect", id], true).await, approved);

    let stepped = workflow_cli(url, &["run", id, "--delete", "--next-step"], true).await;
    assert_eq!(stepped["status"], "running");
    let stages = stepped["stage_history"].as_array().unwrap();
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0]["stage"], "gate_user");
    assert_eq!(stages[0]["status"], "completed");
    assert_eq!(stages[0]["metadata_json"]["result"]["implemented"], false);
    assert_eq!(
        stages[0]["metadata_json"]["result"]["execution_mode"],
        "delete"
    );
    assert_eq!(stages[1]["stage"], "purge_media");
    assert_eq!(stages[1]["status"], "pending");
    assert_eq!(stages[1]["is_current"], true);
    assert!(stepped["execution"]["lease_owner"].is_null());
    stepped
}
