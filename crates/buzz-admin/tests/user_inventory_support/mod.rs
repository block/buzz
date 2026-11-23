//! Shared assertions for the inventory CLI's demo and real workflow journeys.

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

pub async fn real_workflow_stops_at_gate(url: &str, id: &str) -> Value {
    let approved = workflow_cli(url, &["inspect", id], true).await;
    assert_eq!(approved["status"], "approved");
    assert!(approved["stage_history"].as_array().unwrap().is_empty());
    let preview = workflow_output(url, &["run", id, "--next-step"]).await;
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert_eq!(workflow_cli(url, &["inspect", id], true).await, approved);

    workflow_refuses(
        url,
        &["run", id, "--delete", "--next-step"],
        "real handler gate_user is not implemented",
    )
    .await;
    let stopped = workflow_cli(url, &["inspect", id], true).await;
    assert_eq!(stopped["status"], "retry_wait");
    let execution = &stopped["execution"];
    assert!(execution["lease_owner"].is_null());
    assert!(execution["lease_expires_at"].is_null());
    assert!(execution["completed_at"].is_null());
    assert!(execution["next_attempt_at"].is_string());
    let stages = stopped["stage_history"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    let stage = &stages[0];
    assert_eq!(stage["stage"], "gate_user");
    assert_eq!(stage["status"], "failed");
    assert_eq!(stage["is_current"], true);
    assert!(stage["started_at"].is_string());
    assert!(stage["finished_at"].is_null());
    assert!(stage["metadata_json"].get("result").is_none());
    assert!(stage["metadata_json"].get("transition").is_none());
    stopped
}

mod parity_postgres_tests;
