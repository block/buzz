#[path = "support/approval_fixtures.rs"]
#[allow(dead_code)]
mod platform;
use buzz_db::{
    workflow::{CreateApprovalParams, RunStatus},
    Db,
};
use nostr::{EventBuilder, Keys, Kind, Tag};
use platform::post_event;
use serde_json::json;
use std::{process::Stdio, time::Duration};
use uuid::Uuid;

#[tokio::test]
async fn signed_digest_decisions_resume_once_and_survive_restart() {
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    buzz_db::migration::run_migrations(&pool).await.unwrap();
    let db = Db::from_pool(pool.clone());
    let owner = Keys::generate();
    let wrong = Keys::generate();
    let host = format!("s39-{}.test", Uuid::new_v4());
    let tenant = match db
        .create_community_with_owner(&host, &owner.public_key().to_hex())
        .await
        .unwrap()
    {
        buzz_db::CreateCommunityWithOwnerResult::Created(row) => row.id,
        other => panic!("{other:?}"),
    };
    db.ensure_user(tenant, &owner.public_key().to_bytes())
        .await
        .unwrap();
    let room = Uuid::new_v4();
    db.create_channel_with_id(
        tenant,
        room,
        "approval-room",
        buzz_db::channel::ChannelType::Stream,
        buzz_db::channel::ChannelVisibility::Open,
        None,
        &owner.public_key().to_bytes(),
        None,
    )
    .await
    .unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let scratch = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_buzz-relay"));
    let mut child = command
        .env(
            "BUZZ_RELAY_PRIVATE_KEY",
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .env("BUZZ_BIND_ADDR", address.to_string())
        .env("BUZZ_HEALTH_BIND_ADDR", "127.0.0.1:0")
        .env("BUZZ_METRICS_BIND_ADDR", "127.0.0.1:0")
        .env("BUZZ_REQUIRE_RELAY_MEMBERSHIP", "false")
        .env("BUZZ_REQUIRE_AUTH_TOKEN", "false")
        .env("BUZZ_GIT_CONFORMANCE_PROBE", "false")
        .env("BUZZ_GIT_REPO_PATH", scratch.path().join("repos"))
        .env("BUZZ_GIT_PACK_CACHE_PATH", scratch.path().join("packs"))
        .env("RUST_LOG", "error")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("http://{address}");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            assert!(child.try_wait().unwrap().is_none());
            if client.get(format!("{url}/health")).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let mut outcomes = Vec::new();
    for decision in ["grant", "deny", "expired"] {
        let (def,definition)=buzz_workflow::WorkflowEngine::parse_yaml(&format!("name: approval\ntrigger:\n  on: message_posted\nsteps:\n  - id: gate\n    action: request_approval\n    from: {}\n    message: confirm\n  - id: after\n    action: send_message\n    text: continued-{decision}\n",owner.public_key().to_hex())).unwrap();
        let wf = db
            .create_workflow(
                tenant,
                Some(room),
                &owner.public_key().to_bytes(),
                decision,
                &definition,
                &[0; 32],
            )
            .await
            .unwrap();
        let context = buzz_workflow::executor::TriggerContext {
            channel_id: room.to_string(),
            ..Default::default()
        };
        let run = db
            .create_workflow_run(
                tenant,
                wf,
                None,
                Some(&serde_json::to_value(context).unwrap()),
            )
            .await
            .unwrap();
        db.update_workflow_run(tenant, run, RunStatus::Running, 0, &json!([]), None)
            .await
            .unwrap();
        buzz_db::workflow::suspend_for_approval(
            &pool,
            CreateApprovalParams {
                community_id: tenant,
                token: decision,
                workflow_id: wf,
                run_id: run,
                step_id: &def.steps[0].id,
                step_index: 0,
                approver_spec: &owner.public_key().to_hex(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            },
            &json!([]),
        )
        .await
        .unwrap();
        let approval = db.get_approval(tenant, decision).await.unwrap();
        if decision == "expired" {
            sqlx::query("UPDATE workflow_approvals SET expires_at=now()-interval '1 second' WHERE community_id=$1 AND token=$2").bind(tenant.as_uuid()).bind(&approval.token).execute(&pool).await.unwrap();
        }
        let read: serde_json::Value = client
            .get(format!("{url}/workflows/{wf}/runs/{run}/approvals"))
            .header("host", &host)
            .header("X-Pubkey", owner.public_key().to_hex())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let digest = read["approvals"][0]["action_digest"]
            .as_str()
            .expect("approval read must expose bound action_digest")
            .to_owned();
        let make = |keys: &Keys, digest: &str| {
            EventBuilder::new(
                Kind::Custom(if decision == "deny" { 46031 } else { 46030 }),
                "",
            )
            .tags([
                Tag::parse(["d", &hex::encode(&approval.token)]).unwrap(),
                Tag::parse(["action-digest", digest]).unwrap(),
                Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap()
        };
        for (keys, d) in [(&wrong, digest.as_str()), (&owner, "wrong-digest")] {
            let result = post_event(&client, &url, &host, keys, &make(keys, d)).await;
            assert_ne!(result["accepted"], true, "{result}");
        }
        let result = post_event(&client, &url, &host, &owner, &make(&owner, &digest)).await;
        assert_eq!(
            result["accepted"].as_bool().unwrap_or(false),
            decision != "expired",
            "{result}"
        );
        if decision == "grant" {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if db.get_workflow_run(tenant, run).await.unwrap().status
                        == RunStatus::Completed
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap();
        }
        let replay = post_event(&client, &url, &host, &owner, &make(&owner, &digest)).await;
        assert_ne!(replay["accepted"], true, "fresh replay must fail: {replay}");
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND content=$2")
                .bind(tenant.as_uuid())
                .bind(format!("continued-{decision}"))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, if decision == "grant" { 1 } else { 0 });
        outcomes.push((
            run,
            db.get_workflow_run(tenant, run).await.unwrap().status,
            make(&owner, &digest),
        ));
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    let mut child = command.spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            assert!(child.try_wait().unwrap().is_none());
            if client.get(format!("{url}/health")).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let reopened = Db::from_pool(
        sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap(),
    );
    for (run, status, event) in outcomes {
        assert_ne!(
            post_event(&client, &url, &host, &owner, &event).await["accepted"],
            true
        );
        assert_eq!(
            reopened.get_workflow_run(tenant, run).await.unwrap().status,
            status
        );
    }

    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
