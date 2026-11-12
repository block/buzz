//! Exercise the real CLI with SELECT-only access and no workflow tables.

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
    workflow_cli_at(url, args, success, "http://127.0.0.1:1").await
}

async fn workflow_cli_at(url: &str, args: &[&str], success: bool, endpoint: &str) -> Value {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        Command::new(admin_binary())
            .arg("user-deletions")
            .args(args)
            .env("DATABASE_URL", url)
            .env("BUZZ_DB_POOL_SIZE", "1")
            .env("BUZZ_S3_ENDPOINT", endpoint)
            .env("BUZZ_S3_BUCKET", "test")
            .env("BUZZ_S3_ACCESS_KEY", "test")
            .env("BUZZ_S3_SECRET_KEY", "test")
            .env("BUZZ_S3_ADDRESSING_STYLE", "path")
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
        serde_json::from_slice(&output.stdout).unwrap()
    } else {
        Value::String(String::from_utf8(output.stderr).unwrap())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn inventory_submission_routes_all_targets_through_gate_then_real_media() {
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
        workflow_cli(&url, &["run", id], false).await;
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
        let stepped = workflow_cli(&url, &["run", id, "--next-step"], true).await;
        assert_eq!(stepped["stage_history"][0]["stage"], "gate_user");
        assert_eq!(stepped["stage_history"][0]["status"], "completed");
        assert_eq!(
            stepped["stage_history"][0]["metadata_json"]["result"]["dummy"],
            true
        );
        assert_eq!(stepped["stage_history"][1]["stage"], "purge_media");
        assert_eq!(stepped["stage_history"][1]["status"], "pending");
        let gate = stepped["stage_history"][0].clone();
        let error = workflow_cli(&url, &["run", id], false).await;
        assert!(error
            .as_str()
            .unwrap()
            .contains("media deletion requires --delete"));
        let done = if subject == "demo" {
            let error = workflow_cli(&url, &["run", id, "--delete"], false).await;
            assert!(
                error
                    .as_str()
                    .unwrap()
                    .contains("expected a 64-character hex public key or an npub"),
                "{error}"
            );
            let blocked = workflow_cli(&url, &["inspect", id], true).await;
            assert_eq!(blocked["status"], "retry_wait");
            assert_eq!(blocked["stage_history"].as_array().unwrap().len(), 2);
            assert_eq!(blocked["stage_history"][1]["status"], "failed");
            blocked
        } else {
            let storage = EmptyStorage::start().await;
            let done =
                workflow_cli_at(&url, &["run", id, "--delete"], true, &storage.endpoint).await;
            assert_eq!(done["status"], "completed");
            let stages = done["stage_history"].as_array().unwrap();
            assert_eq!(stages.len(), 4);
            assert_eq!(stages[1]["metadata_json"]["result"]["deleted_versions"], 0);
            assert_eq!(
                stages[1]["metadata_json"]["result"]["account_erasure_complete"],
                false
            );
            assert!(stages
                .iter()
                .all(|s| s["status"] == "completed" && s["is_current"] == false));
            let repeated =
                workflow_cli_at(&url, &["run", id, "--delete"], true, &storage.endpoint).await;
            assert_eq!(repeated, done);
            assert_eq!(
                storage.requests.load(std::sync::atomic::Ordering::SeqCst),
                2
            );
            done
        };
        assert_eq!(done["stage_history"][0], gate);
        let inspected = workflow_cli(&url, &["inspect", id], true).await;
        assert_eq!(inspected, done);
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
            assert_eq!(value, &done[field]);
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
    // Community handlers remain placeholders; real membership is unchanged.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members WHERE community_id = $1 AND pubkey = decode($2, 'hex')")
        .bind(community).bind(&key).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn inventory_approval_and_resume_use_saved_account_identity_without_resolution() {
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
    let stepped = workflow_cli(&url, &["run", id, "--next-step"], true).await;
    let gate = stepped["stage_history"][0].clone();
    assert_eq!(gate["status"], "completed");
    assert_eq!(stepped["stage_history"][1]["stage"], "purge_media");
    let error = workflow_cli(&url, &["run", id, "--delete"], false).await;
    assert!(
        error
            .as_str()
            .unwrap()
            .contains("expected a 64-character hex public key or an npub"),
        "{error}"
    );
    let blocked = workflow_cli(&url, &["inspect", id], true).await;
    assert_eq!(blocked["target"], account);
    assert_eq!(blocked["inventory_manifest"], saved["inventory_manifest"]);
    assert_eq!(blocked["status"], "retry_wait");
    assert_eq!(blocked["stage_history"][0], gate);
    assert_eq!(blocked["stage_history"].as_array().unwrap().len(), 2);
    assert_eq!(blocked["stage_history"][1]["status"], "failed");
    for stage in blocked["stage_history"].as_array().unwrap() {
        assert_eq!(stage["metadata_json"]["inputs"]["target"], account);
    }
}

struct EmptyStorage {
    endpoint: String,
    requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for EmptyStorage {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl EmptyStorage {
    async fn start() -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    let mut request = Vec::new();
                    while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                        let mut chunk = [0; 4096];
                        let n = socket.read(&mut chunk).await.unwrap();
                        assert_ne!(n, 0);
                        request.extend_from_slice(&chunk[..n]);
                        assert!(request.len() < 65536);
                    }
                    assert!(request.starts_with(b"GET "));
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let body =
                        "<ListVersionsResult><IsTruncated>false</IsTruncated></ListVersionsResult>";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                })
                .await
                .unwrap();
            }
        });
        Self {
            endpoint,
            requests,
            server,
        }
    }
}
