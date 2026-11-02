//! Signed HTTP coverage against a relay process and an isolated database.

mod external_infra_process {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use nostr::{Event, EventBuilder, Keys, Kind, Tag};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{net::TcpListener, process::Command, time::Duration};
    use uuid::Uuid;

    struct Relay(std::process::Child);
    impl Drop for Relay {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    async fn post(
        client: &reqwest::Client,
        base: &str,
        keys: &Keys,
        path: &str,
        body: Value,
    ) -> (u16, Value) {
        let url = format!("{base}{path}");
        let body = serde_json::to_vec(&body).unwrap();
        let auth = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url.as_str()]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
                Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let response = client
            .post(url)
            .header(
                "authorization",
                format!(
                    "Nostr {}",
                    STANDARD.encode(serde_json::to_vec(&auth).unwrap())
                ),
            )
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body = response.json().await.unwrap();
        (status, body)
    }

    async fn publish(client: &reqwest::Client, base: &str, keys: &Keys, event: &Event) {
        let (status, body) = post(client, base, keys, "/events", json!(event)).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["accepted"], true, "{body}");
    }

    #[tokio::test]
    #[ignore = "requires a local relay network process, empty PostgreSQL database, and Redis"]
    async fn relay_recovers_certification_after_restart_without_a_writer_retry() {
        let database = std::env::var("BUZZ_CERTIFICATION_E2E_DATABASE_URL").unwrap();
        let pool = sqlx::PgPool::connect(&database).await.unwrap();
        let name: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            name.starts_with("buzz_cert_"),
            "requires an isolated test database"
        );
        let empty: bool = sqlx::query_scalar("SELECT to_regclass('public.communities') IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(empty, "requires an empty test database");
        let listeners: Vec<_> = (0..3)
            .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let ports: Vec<_> = listeners
            .iter()
            .map(|s| s.local_addr().unwrap().port())
            .collect();
        drop(listeners);
        let base = format!("http://127.0.0.1:{}", ports[0]);
        let relay_keys = Keys::generate();
        let owner = Keys::generate();
        let log = tempfile::NamedTempFile::new().unwrap();
        let spawn = |enabled: bool| {
            Relay(
                Command::new(env!("CARGO_BIN_EXE_buzz-relay"))
                    .env("DATABASE_URL", &database)
                    .env("BUZZ_BIND_ADDR", format!("127.0.0.1:{}", ports[0]))
                    .env("BUZZ_HEALTH_PORT", ports[1].to_string())
                    .env("BUZZ_METRICS_PORT", ports[2].to_string())
                    .env("RELAY_URL", format!("ws://127.0.0.1:{}", ports[0]))
                    .env(
                        "BUZZ_RELAY_PRIVATE_KEY",
                        relay_keys.secret_key().to_secret_hex(),
                    )
                    .env("RELAY_OWNER_PUBKEY", owner.public_key().to_hex())
                    .env("BUZZ_REQUIRE_AUTH_TOKEN", "true")
                    .env("BUZZ_REQUIRE_RELAY_MEMBERSHIP", "false")
                    .env("BUZZ_AUTO_MIGRATE", "true")
                    .env("BUZZ_ARTIFACT_INCREMENTAL_ENABLED", enabled.to_string())
                    .env("BUZZ_PUSH_ENABLED", "false")
                    .env("BUZZ_AUDIT_ENABLED", "false")
                    .stdout(log.reopen().unwrap())
                    .stderr(log.reopen().unwrap())
                    .spawn()
                    .unwrap(),
            )
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let mut relay = spawn(true);
        ready(&client, &base, &log).await;
        let channel = Uuid::new_v4();
        let create = EventBuilder::new(Kind::Custom(9007), "")
            .tags([
                Tag::parse(["h", &channel.to_string()]).unwrap(),
                Tag::parse(["name", &format!("certification-{channel}")]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        publish(&client, &base, &owner, &create).await;
        let query = json!({"artifact":"current","#type":["buzz.task"]});
        let (_, first) = post(
            &client,
            &base,
            &owner,
            "/query",
            json!([{
                "artifact":"incremental", "query":query
            }]),
        )
        .await;
        assert!(first["checkpoint"].is_string(), "{first}");
        let artifact = Uuid::new_v4();
        let event = EventBuilder::new(Kind::Custom(45010), "task")
            .tags([
                Tag::parse(["ar", "1"]).unwrap(),
                Tag::parse(["d", &artifact.to_string()]).unwrap(),
                Tag::parse(["h", &channel.to_string()]).unwrap(),
                Tag::parse(["type", "buzz.task"]).unwrap(),
                Tag::parse(["title", "Certification"]).unwrap(),
                Tag::parse(["op", "create"]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        publish(&client, &base, &owner, &event).await;
        drop(relay);

        // The fixture models a committed write outside the overlap, with no caller.
        let mut tx = pool.begin().await.unwrap();
        let community: Uuid =
            sqlx::query_scalar("SELECT community_id FROM artifact_heads WHERE artifact_id=$1")
                .bind(artifact)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        sqlx::query("UPDATE artifact_heads SET modified_at=$1::timestamptz-interval '121 seconds' WHERE community_id=$2 AND artifact_id=$3")
        .bind(first["startedAt"].as_str().unwrap()).bind(community).bind(artifact).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO artifact_sync_pending(community_id,operation_id,earliest_at) SELECT $1,gen_random_uuid(),$2::timestamptz-interval '121 seconds' FROM generate_series(1,500)")
        .bind(community).bind(first["startedAt"].as_str().unwrap()).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();

        relay = spawn(true);
        ready(&client, &base, &log).await;
        let mut reset = false;
        for _ in 0..30 {
            let (status, body) = post(
                &client,
                &base,
                &owner,
                "/query",
                json!([{
                    "artifact":"incremental", "query":query, "checkpoint":first["checkpoint"]
                }]),
            )
            .await;
            if status == 409 {
                assert_eq!(body["reason"], "policy_changed");
                reset = true;
                break;
            }
            assert_eq!(status, 200, "{body}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(reset, "recovery must reset without a writer retry");
        let (_, recovered) = post(
            &client,
            &base,
            &owner,
            "/query",
            json!([{
                "artifact":"incremental", "query":query
            }]),
        )
        .await;
        assert_eq!(recovered["changes"][0]["event"]["id"], event.id.to_hex());
        let (status, ordinary) = post(&client, &base, &owner, "/query", json!([query])).await;
        assert_eq!(status, 200);
        assert!(ordinary.to_string().contains(&event.id.to_hex()));
        let mut remaining = 500;
        for _ in 0..15 {
            remaining = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM artifact_sync_pending WHERE community_id=$1",
            )
            .bind(community)
            .fetch_one(&pool)
            .await
            .unwrap();
            if remaining == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(remaining, 0);
        drop(relay);

        let _relay = spawn(false);
        ready(&client, &base, &log).await;
        let (status, body) = post(
            &client,
            &base,
            &owner,
            "/query",
            json!([{
                "artifact":"incremental", "query":query
            }]),
        )
        .await;
        assert_eq!(status, 503);
        assert_eq!(body["code"], "incremental_unavailable");
        let (status, ordinary) = post(&client, &base, &owner, "/query", json!([query])).await;
        assert_eq!(status, 200);
        assert!(ordinary.to_string().contains(&event.id.to_hex()));
        println!("PASS signed HTTP, durable restart recovery, existing checkpoints, ordinary reads, gate off");
    }

    async fn ready(client: &reqwest::Client, base: &str, log: &tempfile::NamedTempFile) {
        for _ in 0..150 {
            if client
                .get(format!("{base}/health"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "relay startup failed: {}",
            std::fs::read_to_string(log.path()).unwrap()
        );
    }
}
