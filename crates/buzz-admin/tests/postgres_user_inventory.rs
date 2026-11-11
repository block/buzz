//! CLI inventory permissions, explicit execution modes, and saved request identity.

use nostr::ToBech32;
use serde_json::Value;
use sqlx::PgPool;
use tokio::process::Command;
use url::Url;
use uuid::Uuid;

fn admin_binary() -> std::ffi::OsString {
    // Nextest relocates binaries when extracting an archive. Cargo's build-time
    // path remains the fallback for local cargo test runs.
    std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_inventory_cli_uses_only_existing_membership_tables() {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = PgPool::connect(&database_url).await.unwrap();
    // All interpolated identifiers are this fixed prefix plus generated hex.
    let name = format!("inventory_{}", Uuid::new_v4().simple());
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {name}; SET LOCAL search_path TO {name}, public;"
    )))
    .execute(&mut *tx)
    .await
    .unwrap();
    // These are existing deployed tables. No entity_deletion_* schema is needed.
    sqlx::raw_sql(include_str!(
        "../../../schema/tables/public/communities.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../schema/tables/public/relay_members.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    for schema in [
        include_str!("../../../schema/tables/public/channels.sql"),
        include_str!("../../../schema/tables/public/channel_members.sql"),
    ] {
        sqlx::raw_sql(schema).execute(&mut *tx).await.unwrap();
    }
    let pubkey = "5e4083a7a793d02575876eaf47932b4835999f5af2653d26250a909b450da7c9";
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'inventory-cli.example')")
        .bind(community)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(community)
        .bind(pubkey)
        .execute(&mut *tx)
        .await
        .unwrap();
    let participant_community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'channel-only-cli.example')")
        .bind(participant_community)
        .execute(&mut *tx)
        .await
        .unwrap();
    for target in [community, participant_community] {
        for _ in 0..2 {
            let channel = Uuid::new_v4();
            sqlx::query("INSERT INTO channels (community_id, id, name, created_by) VALUES ($1, $2, 'inventory-test-' || $2::uuid::text, decode($3, 'hex'))")
                .bind(target).bind(channel).bind(pubkey).execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey) VALUES ($1, $2, decode($3, 'hex'))")
                .bind(target).bind(channel).bind(pubkey).execute(&mut *tx).await.unwrap();
        }
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {name} LOGIN PASSWORD 'inventory-local-fixture'; \
         GRANT USAGE ON SCHEMA {name} TO {name}; \
         GRANT SELECT ON ALL TABLES IN SCHEMA {name} TO {name};"
    )))
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut reader_url = Url::parse(&database_url).unwrap();
    reader_url.set_username(&name).unwrap();
    reader_url
        .set_password(Some("inventory-local-fixture"))
        .unwrap();
    reader_url
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={name}"));
    let npub = nostr::PublicKey::parse(pubkey)
        .unwrap()
        .to_bech32()
        .unwrap();
    let mut outputs = Vec::new();
    for input in [pubkey, npub.as_str()] {
        outputs.push(
            tokio::time::timeout(
                std::time::Duration::from_secs(15),
                Command::new(admin_binary())
                    .args(["user-deletions", "inventory", "--pubkey", input])
                    .env("DATABASE_URL", reader_url.as_str())
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .unwrap()
            .unwrap(),
        );
    }
    let demo = Command::new(admin_binary())
        .args(["user-deletions", "inventory"])
        .env("DATABASE_URL", reader_url.as_str())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    // Clean cluster-wide role state before asserting the subprocess results.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {name} CASCADE; DROP OWNED BY {name}; DROP ROLE {name};"
    )))
    .execute(&pool)
    .await
    .unwrap();
    let inventories: Vec<Value> = outputs
        .into_iter()
        .map(|output| {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice(&output.stdout).unwrap()
        })
        .collect();
    assert_eq!(inventories[0], inventories[1]);
    assert!(
        demo.status.success(),
        "{}",
        String::from_utf8_lossy(&demo.stderr)
    );
    let demo: Value = serde_json::from_slice(&demo.stdout).unwrap();
    assert_eq!(
        demo,
        serde_json::json!({
            "communities": [
                {"community_id": "00000000-0000-0000-0000-00000000000b", "label": "demo:community-b", "relationship": "participant"},
                {"community_id": "00000000-0000-0000-0000-00000000000c", "label": "demo:community-c", "relationship": "former_participant"}
            ]
        })
    );
    let inventory = &inventories[0];
    assert_eq!(inventory.as_object().unwrap().len(), 1);
    assert_eq!(demo.as_object().unwrap().len(), 1);
    let communities = inventory["communities"].as_array().unwrap();
    assert_eq!(communities.len(), 2);
    let explicit_member = communities
        .iter()
        .find(|item| item["community_id"] == community.to_string())
        .unwrap();
    assert_eq!(explicit_member["label"], "inventory-cli.example");
    assert_eq!(explicit_member["relationship"], "participant");
    let participant = communities
        .iter()
        .find(|item| item["community_id"] == participant_community.to_string())
        .unwrap();
    assert_eq!(participant["label"], "channel-only-cli.example");
    assert_eq!(participant["relationship"], "participant");
    for output in [&demo, inventory] {
        for row in output["communities"].as_array().unwrap() {
            assert_eq!(row.as_object().unwrap().len(), 3);
        }
    }
}

#[test]
fn invalid_inventory_key_fails_before_database_connection() {
    let output = std::process::Command::new(admin_binary())
        .args(["user-deletions", "inventory", "--pubkey", "invalid-npub"])
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("expected a 64-character hex public key or an npub"));
}

async fn workflow_cli(url: &str, args: &[&str], success: bool) -> Value {
    workflow_cli_records(url, args, success)
        .await
        .pop()
        .unwrap_or(Value::Null)
}

async fn workflow_cli_records(url: &str, args: &[&str], success: bool) -> Vec<Value> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        Command::new(admin_binary())
            .arg("user-deletions")
            .args(args)
            .env("DATABASE_URL", url)
            .env("BUZZ_DB_POOL_SIZE", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if success {
        serde_json::Deserializer::from_slice(&output.stdout)
            .into_iter::<Value>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    } else {
        vec![]
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn inventory_submission_keeps_preview_separate_from_explicit_workflow_execution() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let key = "ab".repeat(32);
    let npub = nostr::PublicKey::parse(&key).unwrap().to_bech32().unwrap();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'workflow-fixture.example')")
        .bind(community)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels (community_id, id, name, created_by) VALUES ($1, $2, 'fixture', decode($3, 'hex'))")
        .bind(community).bind(channel).bind(&key).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey) VALUES ($1, $2, decode($3, 'hex'))")
        .bind(community).bind(channel).bind(&key).execute(&pool).await.unwrap();

    for subject in ["demo", npub.as_str()] {
        let preview = workflow_cli(&url, &["inventory", "--pubkey", subject], true).await;
        let saved = workflow_cli(
            &url,
            &[
                "submit",
                "--pubkey",
                subject,
                "--requested-by",
                "operator-a",
            ],
            true,
        )
        .await;
        assert_eq!(saved["inventory_manifest"], preview);
        assert_eq!(
            saved["target"],
            if subject == "demo" {
                "demo:default"
            } else {
                &key
            }
        );
        assert_eq!(saved["status"], "inventoried");
        let id = saved["id"].as_str().unwrap();
        let initial = workflow_cli(&url, &["inspect", id], true).await;
        let preview = workflow_cli_records(&url, &["run", id, "--next-step"], true).await;
        assert!(preview.iter().any(|record| record["read_only"] == true));
        assert_eq!(workflow_cli(&url, &["inspect", id], true).await, initial);
        workflow_cli(
            &url,
            &["run", id, "--delete", "--offline", "--next-step"],
            false,
        )
        .await;
        workflow_cli(
            &url,
            &[
                "approve",
                id,
                "--approved-by",
                "operator-b",
                "--inventory-digest",
                "wrong",
            ],
            false,
        )
        .await;
        let approved = workflow_cli(
            &url,
            &[
                "approve",
                id,
                "--approved-by",
                "operator-b",
                "--inventory-digest",
                saved["inventory_digest"].as_str().unwrap(),
            ],
            true,
        )
        .await;
        assert!(approved["stage_history"].as_array().unwrap().is_empty());
        let expected = if subject == "demo" {
            let stepped = workflow_cli(&url, &["run", id, "--delete", "--next-step"], true).await;
            assert_eq!(stepped["stage_history"][0]["stage"], "gate_user");
            assert_eq!(stepped["stage_history"][0]["status"], "completed");
            let first_id = stepped["stage_history"][0]["id"].clone();
            let done = workflow_cli(&url, &["run", id, "--delete"], true).await;
            assert_eq!(done["status"], "completed");
            assert_eq!(done["stage_history"][0]["id"], first_id);
            let stages = done["stage_history"].as_array().unwrap();
            assert_eq!(stages.len(), 5);
            assert!(stages
                .iter()
                .all(|s| s["status"] == "completed" && s["is_current"] == false));
            done
        } else {
            // Approval does not turn the default print-only mode into a worker.
            let preview = workflow_cli_records(&url, &["run", id, "--next-step"], true).await;
            assert_eq!(preview[0]["record"], "user_gate_preview");
            assert_eq!(preview[0]["subject"], key);
            assert_eq!(workflow_cli(&url, &["inspect", id], true).await, approved);
            workflow_cli(&url, &["run", id, "--read-only", "--next-step"], false).await;
            workflow_cli(&url, &["run", id, "--delete", "--next-step"], false).await;
            assert_eq!(workflow_cli(&url, &["inspect", id], true).await, approved);

            let read_only = workflow_cli(
                &url,
                &[
                    "submit",
                    "--pubkey",
                    subject,
                    "--read-only",
                    "--requested-by",
                    "operator-a",
                ],
                true,
            )
            .await;
            assert_ne!(read_only["id"], saved["id"]);
            assert_eq!(read_only["target"], format!("rehearsal:{key}"));
            let read_only_id = read_only["id"].as_str().unwrap();
            workflow_cli(
                &url,
                &[
                    "approve",
                    read_only_id,
                    "--approved-by",
                    "operator-b",
                    "--inventory-digest",
                    read_only["inventory_digest"].as_str().unwrap(),
                ],
                true,
            )
            .await;
            let stepped = workflow_cli(
                &url,
                &["run", read_only_id, "--read-only", "--next-step"],
                true,
            )
            .await;
            let stages = stepped["stage_history"].as_array().unwrap();
            assert_eq!(stages.len(), 2);
            let gate = stages
                .iter()
                .find(|stage| stage["stage"] == "gate_user")
                .unwrap();
            assert_eq!(gate["status"], "completed");
            assert_eq!(gate["is_current"], false);
            assert_eq!(gate["target_community_id"], Value::Null);
            assert_eq!(
                gate["metadata_json"]["result"]["source_data_unchanged"],
                true
            );
            let media = stages
                .iter()
                .find(|stage| stage["stage"] == "purge_media")
                .unwrap();
            assert_eq!(media["status"], "pending");
            assert_eq!(media["is_current"], true);
            assert_eq!(media["target_community_id"], Value::Null);
            // The read-only gate must not install a real restriction or change
            // the separately approved destructive request.
            let fences: i64 =
                sqlx::query_scalar("SELECT count(*) FROM user_deletion_fences WHERE pubkey=$1")
                    .bind(&key)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(fences, 0);
            assert_eq!(workflow_cli(&url, &["inspect", id], true).await, approved);
            approved
        };
        let inspected = workflow_cli(&url, &["inspect", id], true).await;
        assert_eq!(inspected, expected);
        // Hex and npub converge on the same saved request without rediscovery.
        let repeat = workflow_cli(
            &url,
            &[
                "submit",
                "--pubkey",
                if subject == "demo" { "demo" } else { &key },
                "--requested-by",
                "operator-a",
            ],
            true,
        )
        .await;
        for (field, value) in repeat.as_object().unwrap() {
            assert_eq!(value, &expected[field]);
        }
        workflow_cli(
            &url,
            &[
                "submit",
                "--pubkey",
                subject,
                "--requested-by",
                "someone-else",
            ],
            false,
        )
        .await;
    }
    // Neither demo execution nor the real read-only gate changes memberships.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members WHERE community_id = $1 AND pubkey = decode($2, 'hex')")
        .bind(community).bind(&key).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn inventory_approval_preserves_account_identity_but_execution_rejects_unsupported_subject() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let saved = workflow_cli(
        &url,
        &[
            "submit",
            "--target",
            "demo:resolved-account",
            "--requested-by",
            "operator-a",
        ],
        true,
    )
    .await;
    let id = saved["id"].as_str().unwrap();
    // Model the output of a future account resolver. This is not CLI support
    // for resolving account IDs: the inventory is already saved and complete.
    let account = "builderlab:account-123";
    sqlx::query("UPDATE entity_deletion_requests SET target = $2 WHERE id = $1")
        .bind(Uuid::parse_str(id).unwrap())
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    workflow_cli(
        &url,
        &[
            "approve",
            id,
            "--approved-by",
            "operator-b",
            "--inventory-digest",
            saved["inventory_digest"].as_str().unwrap(),
        ],
        true,
    )
    .await;
    let approved = workflow_cli(&url, &["inspect", id], true).await;
    assert_eq!(approved["target"], account);
    assert_eq!(approved["inventory_manifest"], saved["inventory_manifest"]);
    assert_eq!(approved["status"], "approved");
    let preview = workflow_cli_records(&url, &["run", id, "--next-step"], true).await;
    assert_eq!(preview[0]["record"], "user_gate_preview");
    assert_eq!(preview[0]["subject"], account);
    assert_eq!(workflow_cli(&url, &["inspect", id], true).await, approved);

    // A future opaque identity may be approved and inspected, but it must not
    // fall back to simulated cleanup when real handlers require a public key.
    workflow_cli(
        &url,
        &["run", id, "--delete", "--offline", "--next-step"],
        false,
    )
    .await;
    let failed = workflow_cli(&url, &["inspect", id], true).await;
    assert_eq!(failed["target"], account);
    assert_eq!(failed["inventory_manifest"], saved["inventory_manifest"]);
    assert_eq!(failed["status"], "retry_wait");
    let stages = failed["stage_history"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0]["stage"], "gate_user");
    assert_eq!(stages[0]["status"], "failed");
    assert_eq!(stages[0]["metadata_json"]["inputs"]["target"], account);
    assert!(stages[0]["metadata_json"].get("result").is_none());
    let fences: i64 = sqlx::query_scalar("SELECT count(*) FROM user_deletion_fences")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fences, 0);
}
