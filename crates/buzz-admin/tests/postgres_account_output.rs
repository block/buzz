//! The workflow emits one JSON document; direct account cleanup emits NDJSON.
use buzz_db::entity_deletion::{NewEntityStage, NextStage, StageTransition};
use buzz_db::user_deletion::UserDeletionInventory;
use buzz_db::Db;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::process::Output;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

const TARGET: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

async fn cli(url: &str, args: &[&str]) -> Output {
    let binary = std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into());
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new(binary)
            .arg("user-deletions")
            .args(args)
            .env("DATABASE_URL", url)
            .env("BUZZ_DB_POOL_SIZE", "2")
            .env("BUZZ_RELAY_PRIVATE_KEY", format!("{:064x}", 1))
            .env_remove("RELAY_OPERATOR_PUBKEYS")
            .env_remove("RELAY_OWNER_PUBKEY")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn account_workflow_stdout_is_one_document_and_direct_output_remains_ndjson() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let db = Db::from_pool(pool.clone());
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        db.migrate().await.unwrap();
    }
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES ($1,$2)")
        .bind(community)
        .bind(format!("{community}.example.org"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES ($1,decode($2,'hex'))")
        .bind(community)
        .bind(TARGET)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES ($1,$2,'member')")
        .bind(community)
        .bind(TARGET)
        .execute(&pool)
        .await
        .unwrap();
    let request = db
        .user_deletion_store()
        .submit(TARGET, "fixture", None)
        .await
        .unwrap();
    let store = db.entity_deletion_store();
    store
        .approve(
            request.id,
            "fixture",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let lease = store
        .claim(request.id, "fixture", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let inventory = UserDeletionInventory::read(&request).unwrap();
    let mut previous = None;
    // Gate/media are fixture preconditions. Content and account execution,
    // transitions, roster signing, stdout and lease release use production code.
    for (name, scope, decision) in [
        ("gate_user", None, Value::Null),
        (
            "purge_media",
            None,
            json!({"kind":"stage","stage":"purge_media"}),
        ),
        (
            "purge_content",
            Some(community),
            json!({"kind":"enter_communities"}),
        ),
    ] {
        let next = NewEntityStage {
            stage: name.into(),
            community: scope,
            inputs: inventory.inputs(TARGET, scope).unwrap(),
        };
        let result = store
            .transition_stage(&lease, previous, |_, _| {
                Ok(match previous {
                    None => StageTransition::Initialize(next),
                    Some(id) => StageTransition::Advance {
                        previous: id,
                        decision,
                        successor: Some(next),
                    },
                })
            })
            .await
            .unwrap();
        let NextStage::Stage(stage) = result else {
            panic!("expected stage");
        };
        if name != "purge_content" {
            store.start_stage(&lease, stage.id).await.unwrap();
            store
                .save_stage_success(&lease, stage.id, json!({"fixture":true}))
                .await
                .unwrap();
        }
        previous = Some(stage.id);
    }
    store.release(&lease).await.unwrap();
    let community_text = community.to_string();
    let direct = cli(
        &url,
        &[
            "account-batches",
            "--pubkey",
            TARGET,
            "--community",
            &community_text,
        ],
    )
    .await;
    let records: Vec<Value> = String::from_utf8(direct.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records
        .iter()
        .any(|r| r["record"] == "proposed_batch" && r["table"] == "relay_members"));
    let content = cli(
        &url,
        &["run", &request.id.to_string(), "--delete", "--next-step"],
    )
    .await;
    let content_document: Value = serde_json::from_slice(&content.stdout).unwrap();
    let stages = content_document["stage_history"].as_array().unwrap();
    assert!(stages
        .iter()
        .any(|s| s["stage"] == "purge_content" && s["status"] == "completed"));
    assert!(stages.iter().any(|s| s["stage"] == "purge_account"
        && s["status"] == "pending"
        && s["is_current"] == true));
    let output = cli(
        &url,
        &["run", &request.id.to_string(), "--delete", "--next-step"],
    )
    .await;
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["execution"]["lease_owner"], Value::Null);
    assert!(document["stage_history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["stage"] == "purge_account" && s["status"] == "completed"));
    let progress: Vec<Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(progress.iter().any(|r| r["saved_community_roster"] == true));
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_members WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community)
    .bind(TARGET)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 0);
    let tags: Value = sqlx::query_scalar(
        "SELECT tags FROM events WHERE community_id=$1 AND kind=13534 AND deleted_at IS NULL",
    )
    .bind(community)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!tags
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t[0] == "member" && t[1] == TARGET));
}
