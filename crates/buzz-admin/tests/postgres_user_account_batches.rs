//! Real command wiring: modes, bounded exit status, and resumption.
use nostr::ToBech32;
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

const TARGET: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

async fn cli(url: &str, community: Uuid, pubkey: &str, args: &[&str], code: i32) -> Vec<Value> {
    let binary = std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into());
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new(binary)
            .args([
                "user-deletions",
                "account-batches",
                "--community",
                &community.to_string(),
                "--pubkey",
                pubkey,
                "--batch-size",
                "2",
            ])
            .args(args)
            .env("DATABASE_URL", url)
            .env("BUZZ_AUTO_MIGRATE", "false")
            .env("BUZZ_RELAY_PRIVATE_KEY", format!("{:064x}", 1))
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

async fn tokens(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,id) FROM api_tokens t")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn account_command_modes_cap_and_resume_use_real_database_engine() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let community = Uuid::new_v4();
    let other = Uuid::new_v4();
    let seed = include_str!("../../buzz-db/src/store/user_deletion/account_batches/fixture.sql")
        .replace("__TARGET__", TARGET)
        .replace("__COMMUNITY__", &community.to_string())
        .replace("__OTHER__", &other.to_string());
    sqlx::raw_sql(sqlx::AssertSqlSafe(seed))
        .execute(&pool)
        .await
        .unwrap();
    let before = tokens(&pool).await;
    let npub = nostr::PublicKey::parse(TARGET)
        .unwrap()
        .to_bech32()
        .unwrap();
    for (mode, key) in [
        (vec![], TARGET.to_uppercase()),
        (vec!["--read-only"], npub),
        (vec!["--print-only"], TARGET.to_owned()),
    ] {
        let rows = cli(&url, community, &key, &mode, 0).await;
        assert_eq!(rows[0]["record"], "account_targets");
        assert_eq!(rows[0]["target"], TARGET);
        let summary = rows.last().unwrap();
        assert_eq!(summary["record"], "account_handler_summary");
        assert_eq!(summary["items"], 35);
        assert_eq!(summary["read_only"], true);
        assert_eq!(summary["supported_scope_exhausted"], true);
        assert_eq!(tokens(&pool).await, before);
    }
    assert!(
        cli(&url, community, TARGET, &["--read-only", "--delete"], 2)
            .await
            .is_empty()
    );
    let cap = cli(&url, community, TARGET, &["--max-items", "1"], 2).await;
    assert_eq!(cap.last().unwrap()["supported_scope_exhausted"], false);
    assert_eq!(tokens(&pool).await, before);
    let cap = cli(
        &url,
        community,
        TARGET,
        &["--delete", "--max-items", "1"],
        2,
    )
    .await;
    assert_eq!(cap.last().unwrap()["items"], 1);
    assert_eq!(cap.last().unwrap()["supported_scope_exhausted"], false);
    assert_eq!(
        tokens(&pool).await.as_array().unwrap().len(),
        before.as_array().unwrap().len() - 1
    );
    let complete = cli(&url, community, TARGET, &["--delete"], 0).await;
    assert_eq!(complete.last().unwrap()["items"], 34);
    assert_eq!(complete.last().unwrap()["supported_scope_exhausted"], true);
    assert_eq!(complete.last().unwrap()["account_erasure_complete"], false);
    assert_eq!(
        cli(&url, community, TARGET, &["--delete"], 0)
            .await
            .last()
            .unwrap()["items"],
        0
    );
    let after = tokens(&pool).await;
    let unrelated = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|r| r["community_id"] == other.to_string())
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(unrelated(&after), unrelated(&before));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM entity_deletion_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn global_and_frozen_scope_commands_use_explicit_saved_inputs() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let binary = std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into());
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES ($1,'scope-cli.example.org')")
        .bind(community)
        .execute(&pool)
        .await
        .unwrap();
    // No membership, profile or agent ownership: global records must still run.
    sqlx::query("INSERT INTO product_feedback(event_id,submitter_pubkey,body,event_created_at) VALUES (decode($1,'hex'),decode($1,'hex'),'global private feedback',now())").bind(TARGET).execute(&pool).await.unwrap();
    let invoke = |args: Vec<String>| {
        let mut cmd = Command::new(&binary);
        cmd.arg("user-deletions")
            .args(args)
            .env("DATABASE_URL", &url)
            .env("BUZZ_AUTO_MIGRATE", "false")
            .env("BUZZ_RELAY_PRIVATE_KEY", format!("{:064x}", 1))
            .env_remove("RELAY_OPERATOR_PUBKEYS")
            .env_remove("RELAY_OWNER_PUBKEY")
            .kill_on_drop(true);
        cmd
    };
    let scope = invoke(vec![
        "account-scope".into(),
        "--pubkey".into(),
        TARGET.into(),
    ])
    .output()
    .await
    .unwrap();
    assert!(
        scope.status.success(),
        "{}",
        String::from_utf8_lossy(&scope.stderr)
    );
    let parsed: Value = serde_json::from_slice(&scope.stdout).unwrap();
    assert_eq!(parsed["communities"][0]["community"], community.to_string());
    let mut file = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut file, &scope.stdout).unwrap();
    let base = vec![
        "account-batches".into(),
        "--scope-file".into(),
        file.path().to_str().unwrap().into(),
        "--actor".into(),
        TARGET.into(),
    ];
    let preview = invoke(base.clone()).output().await.unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM product_feedback")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let mut delete = base;
    delete.push("--delete".into());
    let blocked = invoke(delete.clone())
        .env("RELAY_OPERATOR_PUBKEYS", TARGET)
        .output()
        .await
        .unwrap();
    assert!(!blocked.status.success());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM product_feedback")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let applied = invoke(delete.clone()).output().await.unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM product_feedback")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let repeated = invoke(delete).output().await.unwrap();
    assert!(repeated.status.success());
    let last: Value = serde_json::from_str(
        String::from_utf8_lossy(&repeated.stdout)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(last["record"], "account_scope_summary");
    assert_eq!(last["items"], 0);
    assert_eq!(last["account_erasure_complete"], false);
    let global = invoke(vec![
        "account-batches".into(),
        "--pubkey".into(),
        TARGET.into(),
        "--global".into(),
        "--actor".into(),
        TARGET.into(),
    ])
    .output()
    .await
    .unwrap();
    assert!(global.status.success());
    let first: Value = serde_json::from_str(
        String::from_utf8_lossy(&global.stdout)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["scope"], "deployment_global");
}
