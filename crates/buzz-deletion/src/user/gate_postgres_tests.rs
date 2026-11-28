//! Exercise the real deletion worker and its durable identity gate over HTTP.

use super::super::worker::{run, RunOptions};
use super::IdentityGateConfig;
use buzz_db::entity_deletion::{EntityDeletionStore, EntityInspection, EntityStage};
use buzz_db::Db;
use nostr::ToBech32;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

struct Fixture {
    db: Db,
    store: EntityDeletionStore,
    id: Uuid,
    pubkey: String,
    listener: TcpListener,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap();
        let db = Db::from_pool(sqlx::PgPool::connect(&url).await.unwrap());
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            db.migrate().await.unwrap();
        }
        let store = db.entity_deletion_store();
        let subject = nostr::Keys::generate().public_key();
        let request = db
            .user_deletion_store()
            .submit(&subject.to_bech32().unwrap(), "submitter", None)
            .await
            .unwrap();
        db.user_deletion_store()
            .approve(
                request.id,
                "approver",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self {
            db,
            store,
            id: request.id,
            pubkey: subject.to_hex(),
            listener,
        }
    }

    fn options(&self) -> RunOptions {
        let origin = format!("http://{}", self.listener.local_addr().unwrap());
        let identity = IdentityGateConfig::new(
            &format!("{origin}/api/goose/v1/identity/block-user-auth-login"),
            &format!("{origin}/api/goose/v1/identity/delete-user-auth-login"),
            &format!("{origin}/api/goose/v1/identity/delete-builderlab-account"),
            HTTP_TIMEOUT,
        )
        .unwrap();
        RunOptions {
            db: Some(Db::from_pool(self.db.pool().clone())),
            media: None,
            identity: Some(identity),
            cleanup_verified: false,
            delete: true,
            relay_pubkey: None,
            batch_size: 100,
            max_items: 1_000_000,
            next_step: false,
            fail_stage: None,
            stage_delay: Duration::ZERO,
            no_agents_in: Vec::new(),
        }
    }

    async fn run(&self, worker: &str, options: &RunOptions) -> anyhow::Result<i32> {
        timeout(
            Duration::from_secs(15),
            run(
                &self.store,
                self.id,
                worker,
                options,
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("identity gate run exceeded its test deadline")
    }

    async fn inspect(&self) -> EntityInspection {
        self.store.inspect(self.id).await.unwrap()
    }

    fn subject(&self) -> Value {
        json!({"request_id": self.id, "pubkey": self.pubkey})
    }

    async fn accept_post(&self, path: &str, expected_body: &Value) -> TcpStream {
        let mut stream = self.accept().await;
        assert_post(&mut stream, path, expected_body).await;
        stream
    }

    async fn accept(&self) -> TcpStream {
        let (stream, _) = timeout(HTTP_TIMEOUT, self.listener.accept())
            .await
            .expect("expected HTTP operation did not arrive")
            .unwrap();
        stream
    }

    async fn assert_no_more_http(&self) {
        assert!(timeout(Duration::from_millis(100), self.listener.accept())
            .await
            .is_err());
    }
}

async fn assert_post(stream: &mut TcpStream, path: &str, expected_body: &Value) {
    timeout(HTTP_TIMEOUT, async {
        let mut request = BufReader::new(stream).take(16 * 1024);
        let mut line = String::new();
        request.read_line(&mut line).await.unwrap();
        assert_eq!(line.trim_end(), format!("POST {path} HTTP/1.1"));
        let mut length = None;
        let mut authorization = None;
        loop {
            line.clear();
            assert_ne!(request.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
            if name.eq_ignore_ascii_case("authorization") {
                authorization = Some(value.trim().to_owned());
            }
        }
        assert!(
            authorization.is_none(),
            "worker must use transport identity"
        );
        let length = length.unwrap();
        assert!(length <= 16 * 1024);
        let mut body = vec![0; length];
        request.read_exact(&mut body).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            *expected_body
        );
    })
    .await
    .expect("HTTP request was incomplete");
}

async fn respond(stream: TcpStream, body: &Value) {
    respond_with_status(stream, "200 OK", body).await;
}

async fn respond_with_status(mut stream: TcpStream, status: &str, body: &Value) {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

fn receipt(subject: &Value) -> Value {
    let mut receipt = subject.clone();
    receipt["identity_blocked"] = json!(true);
    receipt["identity_revoked"] = json!(true);
    receipt["revoked_at"] = json!("2026-11-18T23:00:00Z");
    receipt["message"] = json!("Blocked example login");
    receipt["blocked_login"] = json!({"provider": "example", "subject": "example-login"});
    receipt
}

fn only_current_gate(inspection: &EntityInspection) -> &EntityStage {
    let [stage] = inspection.stage_history.as_slice() else {
        panic!(
            "expected only a gate stage, got {:?}",
            inspection.stage_history
        );
    };
    assert_eq!(stage.stage, "gate_user");
    assert!(stage.is_current);
    assert!(stage.target_community_id.is_none());
    assert!(stage.finished_at.is_none());
    assert!(stage.metadata_json.get("result").is_none());
    assert!(stage.metadata_json.get("transition").is_none());
    stage
}

fn assert_receipt_saved(inspection: &EntityInspection, receipt: &Value) -> Uuid {
    assert_eq!(inspection.request.status, "running");
    assert!(inspection.execution.lease_owner.is_none());
    assert!(inspection.execution.next_attempt_at.is_none());
    let stage = only_current_gate(inspection);
    assert_eq!(stage.status, "running");
    assert!(stage.metadata_json["waiting"]
        .get("identity_binding")
        .is_none());
    let mut expected = receipt.clone();
    let fields = expected.as_object_mut().unwrap();
    fields.remove("message");
    fields.remove("blocked_login");
    assert_eq!(stage.metadata_json["waiting"]["identity_receipt"], expected);
    stage.id
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn identity_revocation_is_saved_without_starting_cleanup_or_repeating_http() {
    let f = Fixture::new().await;
    let mut options = f.options();
    let receipt = receipt(&f.subject());
    let (result, ()) = tokio::join!(f.run("worker-a", &options), async {
        let revoke = f
            .accept_post("/api/goose/v1/identity/block-user-auth-login", &f.subject())
            .await;
        let saved = f.inspect().await;
        assert_eq!(
            only_current_gate(&saved).metadata_json["waiting"]["reason"],
            "inspect_block_outcome"
        );
        respond(revoke, &receipt).await;
    });
    assert_eq!(result.unwrap(), 1);
    let stage_id = assert_receipt_saved(&f.inspect().await, &receipt);

    // A restart can verify the saved receipt without credentials.
    options.identity = None;
    assert_eq!(f.run("worker-b", &options).await.unwrap(), 1);
    assert_eq!(assert_receipt_saved(&f.inspect().await, &receipt), stage_id);
    f.assert_no_more_http().await;

    // A corrupted saved receipt must fail rather than retrying the mutation.
    sqlx::query(
        r#"UPDATE entity_deletion_stage_history SET metadata_json = jsonb_set(metadata_json, '{waiting,identity_receipt,pubkey}', '"different-key"') WHERE id = $1"#,
    )
    .bind(stage_id)
    .execute(f.db.pool())
    .await
    .unwrap();
    options.identity = Some(f.options().identity.unwrap());
    assert!(f
        .run("worker-c", &options)
        .await
        .unwrap_err()
        .to_string()
        .contains("identity revocation receipt does not match this request"));
    assert!(f.inspect().await.execution.lease_owner.is_none());
    f.assert_no_more_http().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn lost_revocation_response_requires_inspection_after_restart() {
    let f = Fixture::new().await;
    let options = f.options();
    let (result, ()) = tokio::join!(f.run("worker-a", &options), async {
        let stream = f
            .accept_post("/api/goose/v1/identity/block-user-auth-login", &f.subject())
            .await;
        // The server can have acted even though its response is lost.
        drop(stream);
    });
    assert!(result.is_err());
    let failed = f.inspect().await;
    assert_eq!(
        only_current_gate(&failed).metadata_json["waiting"]["reason"],
        "inspect_block_outcome"
    );
    assert!(f
        .run("worker-b", &options)
        .await
        .unwrap_err()
        .to_string()
        .contains("inspect the previous block outcome"));
    f.assert_no_more_http().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unconfirmed_revocation_stays_on_the_gate_without_saving_a_receipt() {
    for (case, status, changed_field) in [
        (
            "false flag",
            "200 OK",
            Some(("identity_revoked", json!(false))),
        ),
        (
            "wrong public key",
            "200 OK",
            Some(("pubkey", json!("different-key"))),
        ),
        (
            "wrong request",
            "200 OK",
            Some(("request_id", json!(Uuid::new_v4()))),
        ),
        (
            "invalid timestamp",
            "200 OK",
            Some(("revoked_at", json!("not-a-timestamp"))),
        ),
        (
            "missing timestamp",
            "200 OK",
            Some(("revoked_at", Value::Null)),
        ),
        (
            "legacy receipt",
            "200 OK",
            Some(("identity_blocked", Value::Null)),
        ),
        ("accepted only", "202 Accepted", None),
        ("mesh denied", "403 Forbidden", None),
    ] {
        let f = Fixture::new().await;
        let options = f.options();
        let mut invalid = receipt(&f.subject());
        if let Some((field, value)) = changed_field {
            invalid[field] = value;
        }
        let (result, ()) = tokio::join!(f.run("worker-a", &options), async {
            let revoke = f
                .accept_post("/api/goose/v1/identity/block-user-auth-login", &f.subject())
                .await;
            respond_with_status(revoke, status, &invalid).await;
        });
        assert!(result.is_err(), "{case} reported a confirmed revocation");
        let failed = f.inspect().await;
        assert_eq!(failed.request.status, "running", "{case}");
        assert!(failed.execution.lease_owner.is_none(), "{case}");
        let stage = only_current_gate(&failed);
        assert_eq!(stage.status, "running", "{case}");
        assert_eq!(
            stage.metadata_json["waiting"]["reason"], "inspect_block_outcome",
            "{case}"
        );
        f.assert_no_more_http().await;
    }
}

async fn at_final_stage(login_deleted: bool) -> Fixture {
    use crate::user::transition::UserDeletionTransition;
    use buzz_db::entity_deletion::NextStage;
    let f = Fixture::new().await;
    // Supply completed predecessors to isolate the final stage from unfinished cleanup handlers.
    let lease = f
        .store
        .claim(f.id, "fixture", crate::DEFAULT_LEASE_DURATION)
        .await
        .unwrap()
        .unwrap();
    let transitions = UserDeletionTransition::new(&f.store);
    let mut previous = None;
    for name in ["gate_user", "purge_media"] {
        let NextStage::Stage(row) = transitions.get_next_stage(&lease, previous).await.unwrap()
        else {
            panic!("missing {name}")
        };
        assert_eq!(row.stage, name);
        f.store.start_stage(&lease, row.id).await.unwrap();
        f.store
            .save_stage_success(&lease, row.id, json!({"fixture":true}))
            .await
            .unwrap();
        previous = Some(row.id);
    }
    let NextStage::Stage(row) = transitions.get_next_stage(&lease, previous).await.unwrap() else {
        panic!("missing final stage")
    };
    assert_eq!(row.stage, "purge_builderlab");
    assert_eq!(row.target_community_id, None);
    if login_deleted {
        let mut login = f.subject();
        login["auth0_deleted"] = json!(true);
        f.store.start_stage(&lease, row.id).await.unwrap();
        f.store
            .save_stage_waiting(
                &lease,
                row.id,
                json!({"reason":"login_deleted", "login_receipt":login}),
            )
            .await
            .unwrap();
    }
    assert!(f.store.release(&lease).await.unwrap());
    f
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn independent_deletions_require_ordered_matching_confirmations() {
    for case in [
        "login_failed",
        "login_wrong_target",
        "account_lost",
        "account_conflict",
        "success",
    ] {
        let f = at_final_stage(false).await;
        let mut options = f.options();
        if case != "success" {
            assert_eq!(f.run("waiting", &options).await.unwrap(), 1);
            f.assert_no_more_http().await;
        }
        options.cleanup_verified = true;
        if case == "success" {
            options.identity = None;
            assert!(f.run("missing-config", &options).await.is_err());
            f.assert_no_more_http().await;
            options.identity = f.options().identity;
        }
        let mut body = f.subject();
        body["cleanup_verified"] = json!(true);
        let (result, ()) = tokio::join!(f.run("delete", &options), async {
            let stream = f
                .accept_post("/api/goose/v1/identity/delete-user-auth-login", &body)
                .await;
            if case == "login_failed" {
                drop(stream);
                return;
            }
            let mut login = f.subject();
            login["auth0_deleted"] = json!(true);
            if case == "login_wrong_target" {
                login["pubkey"] = json!("another-user");
            }
            respond(stream, &login).await;
            if case == "login_wrong_target" {
                return;
            }
            let stream = f
                .accept_post("/api/goose/v1/identity/delete-builderlab-account", &body)
                .await;
            if case == "account_lost" {
                drop(stream);
                return;
            }
            if case == "account_conflict" {
                respond_with_status(stream, "409 Conflict", &json!({})).await;
                return;
            }
            let mut account = f.subject();
            account["account_deleted"] = json!(true);
            respond(stream, &account).await;
        });
        if case == "success" {
            assert_eq!(result.unwrap(), 0);
            assert_eq!(f.inspect().await.request.status, "completed");
            assert_eq!(f.run("again", &options).await.unwrap(), 0);
        } else {
            assert!(result.is_err());
            assert_ne!(f.inspect().await.request.status, "completed");
            assert!(f.run("again", &options).await.is_err());
        }
        f.assert_no_more_http().await;
    }
}

#[tokio::test]
async fn swapped_deletion_routes_are_rejected_before_http() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/custom-prefix/v1/identity",
        listener.local_addr().unwrap()
    );
    assert!(IdentityGateConfig::new(
        &format!("{base}/block-user-auth-login"),
        &format!("{base}/delete-user-auth-login"),
        &format!("{base}/delete-builderlab-account"),
        HTTP_TIMEOUT,
    )
    .is_ok());
    assert!(IdentityGateConfig::new(
        &format!("{base}/block-user-auth-login"),
        &format!("{base}/delete-builderlab-account"),
        &format!("{base}/delete-user-auth-login"),
        HTTP_TIMEOUT,
    )
    .is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn restart_after_confirmed_login_deletion_only_sends_account_request() {
    let f = at_final_stage(true).await;
    let mut options = f.options();
    options.cleanup_verified = true;
    let mut body = f.subject();
    body["cleanup_verified"] = json!(true);
    let (result, ()) = tokio::join!(f.run("restart", &options), async {
        let stream = f
            .accept_post("/api/goose/v1/identity/delete-builderlab-account", &body)
            .await;
        let mut account = f.subject();
        account["account_deleted"] = json!(true);
        respond(stream, &account).await;
    });
    assert_eq!(result.unwrap(), 0);
    f.assert_no_more_http().await;
}

#[test]
fn cleartext_remote_endpoints_require_a_cluster_service_address() {
    for (host, valid) in [
        ("identity.accounts.svc.cluster.local", true),
        ("identity.example.org", false),
        ("identity.svc.cluster.local", false),
        ("identity.accounts.svc.cluster.local.example.org", false),
    ] {
        let base = format!("http://{host}/v1/identity");
        let config = IdentityGateConfig::new(
            &format!("{base}/block-user-auth-login"),
            &format!("{base}/delete-user-auth-login"),
            &format!("{base}/delete-builderlab-account"),
            HTTP_TIMEOUT,
        );
        assert_eq!(config.is_ok(), valid, "{host}");
    }
}
