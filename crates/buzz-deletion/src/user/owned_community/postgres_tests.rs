use super::*;
use crate::user::summary::summarize_request;
use buzz_db::entity_deletion::{inventory_digest, NewEntityStage, NextStage, StageTransition};
use buzz_db::Db;
use buzz_media::{MediaConfig, MediaStorage, S3AddressingStyle};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

struct Fixture {
    db: Db,
    store: EntityDeletionStore,
    lease: EntityLease,
    row: EntityStage,
    community: CommunityId,
}

async fn fixture() -> Fixture {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let db = Db::from_pool(PgPool::connect(&url).await.unwrap());
    let community = db
        .ensure_configured_community("owned.example")
        .await
        .unwrap()
        .id;
    let store = db.entity_deletion_store();
    // Owned intake/routing intentionally remain disabled. Seed that future
    // approved input, then use the real claim, stage and handler operations.
    let target = "11".repeat(32);
    let item = json!({"community_id": community.as_uuid(), "relationship": "owner", "label": "owned.example"});
    let inventory = json!({"communities": [item, {
        "community_id": Uuid::new_v4(), "relationship": "participant", "label": "not-reached"
    }]});
    let digest = inventory_digest(&inventory).unwrap();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO entity_deletion_requests (target, requested_by, status, inventory_manifest, inventory_digest)
         VALUES ($1, 'submitter', 'inventoried', $2, $3) RETURNING id",
    ).bind(&target).bind(&inventory).bind(&digest).fetch_one(db.pool()).await.unwrap();
    store.approve(id, "approver", &digest).await.unwrap();
    let lease = store
        .claim(id, "worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let next = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "delete_owned_community".into(),
                community: Some(*community.as_uuid()),
                inputs: json!({"target": target, "community": item}),
            }))
        })
        .await
        .unwrap();
    let NextStage::Stage(row) = next else {
        panic!("expected stage")
    };
    Fixture {
        db,
        store,
        lease,
        row,
        community,
    }
}

fn media(endpoint: String) -> Arc<MediaStorage> {
    Arc::new(
        MediaStorage::new(&MediaConfig {
            s3_endpoint: endpoint,
            s3_access_key: "test".into(),
            s3_secret_key: "test".into(),
            s3_bucket: "test".into(),
            s3_region: "us-east-1".into(),
            s3_addressing_style: S3AddressingStyle::Path,
            max_image_bytes: 1,
            max_gif_bytes: 1,
            max_video_bytes: 1,
            max_file_bytes: 1,
            public_base_url: "http://localhost/media".into(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        })
        .unwrap(),
    )
}

fn lookup_only_media() -> Arc<MediaStorage> {
    // Any accidental attempt to inventory an existing child fails immediately.
    media("http://127.0.0.1:1".into())
}

async fn submission_media() -> (Arc<MediaStorage>, tokio::task::JoinHandle<()>) {
    submission_media_with_pause(None).await
}

async fn submission_media_with_pause(
    mut pause: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
) -> (Arc<MediaStorage>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        // Exercise the real S3 adapter and inventory builder, with one empty
        // version listing per tenant prefix. No external storage is needed.
        for _ in 0..3 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut headers = [0; 8192];
            let size = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut headers))
                .await
                .unwrap()
                .unwrap();
            assert!(std::str::from_utf8(&headers[..size])
                .unwrap()
                .starts_with("GET "));
            if let Some((started, resume)) = pause.take() {
                started.send(()).unwrap();
                tokio::time::timeout(Duration::from_secs(10), resume)
                    .await
                    .unwrap()
                    .unwrap();
            }
            let body = "<ListVersionsResult><Name>test</Name><IsTruncated>false</IsTruncated></ListVersionsResult>";
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (media(endpoint), server)
}

async fn child(f: &Fixture) -> DeletionRequest {
    f.db.deletion_store()
        .find_for_community(f.community)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stage_follows_replacement_child_for_the_same_community() {
    let f = fixture().await;
    let store = f.db.deletion_store();
    let (media, server) = submission_media().await;
    handle(&f.store, &f.lease, &f.row, &store, &media)
        .await
        .unwrap();
    server.await.unwrap();
    let a = child(&f).await;
    let saved = f.store.inspect(f.row.request_id).await.unwrap();
    assert_eq!(
        saved.stage_history[0].metadata_json["waiting"]["child_request_id"],
        a.id.to_string()
    );
    store
        .abort(a.id, "operator", "replace request")
        .await
        .unwrap();
    // Use the same submission/preparation entry point as the community CLI.
    let (media, server) = submission_media().await;
    let b = submit_and_prepare(
        &store,
        &media,
        Some("owned.example"),
        "operator",
        None,
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(b.community_id, a.community_id);
    assert_eq!(b.stage, DeletionStage::Inventoried);
    let media = lookup_only_media();
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &store, &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Waiting
    );
    let saved = f.store.inspect(f.row.request_id).await.unwrap();
    assert_eq!(saved.stage_history.len(), 1);
    let row = &saved.stage_history[0];
    assert_eq!(row.id, f.row.id);
    assert_eq!(row.status, "running");
    assert_eq!(
        row.metadata_json["waiting"]["child_request_id"],
        b.id.to_string()
    );
    assert_eq!(row.metadata_json["waiting"]["child_stage"], "inventoried");
    assert_eq!(saved.execution.retry_count, 0);
    assert!(saved.execution.last_error.is_none());
    assert_eq!(store.get(a.id).await.unwrap().stage, DeletionStage::Aborted);
    assert_eq!(store.get(b.id).await.unwrap(), b);
    assert!(store.inspect(b.id).await.unwrap().approval.is_none());
    // Model the separate community worker completing B, as in the main flow test.
    sqlx::query("UPDATE community_deletion_requests SET stage = 'retention_pending', completed_at = now() WHERE id = $1")
        .bind(b.id).execute(f.db.pool()).await.unwrap();
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &store, &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Completed
    );
    let saved = f.store.inspect(f.row.request_id).await.unwrap();
    assert_eq!(saved.stage_history.len(), 1);
    assert_eq!(saved.stage_history[0].id, f.row.id);
    assert_eq!(saved.stage_history[0].status, "completed");
    assert_eq!(
        saved.stage_history[0].metadata_json["result"]["child_request_id"],
        b.id.to_string()
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn submission_preserves_operator_transition_during_inventory_listing() {
    let f = fixture().await;
    let store = f.db.deletion_store();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let (media, server) = submission_media_with_pause(Some((started_tx, resume_rx))).await;
    let submission = submit_and_prepare(&store, &media, None, "operator", None, Some(f.community));
    let intervention = async {
        tokio::time::timeout(Duration::from_secs(10), started_rx)
            .await
            .unwrap()
            .unwrap();
        let request = child(&f).await;
        assert_eq!(request.stage, DeletionStage::Submitted);
        let aborted = store
            .abort(request.id, "operator", "cancel during inventory")
            .await
            .unwrap();
        resume_tx.send(()).unwrap();
        aborted
    };
    let (result, aborted) = tokio::join!(submission, intervention);
    server.await.unwrap();
    assert_eq!(result.unwrap(), aborted);
    assert_eq!(store.get(aborted.id).await.unwrap(), aborted);
    assert!(store.inspect(aborted.id).await.unwrap().approval.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn operator_guidance_distinguishes_blocked_aborted_and_owner_submitted() {
    let f = fixture().await;
    let mut child =
        f.db.deletion_store()
            .submit(Some("owned.example"), "operator", None, None)
            .await
            .unwrap();
    let observed = observation(&child, false);
    assert_eq!(observed["reason"], "preparation_required");
    assert_eq!(observed["commands"][1][2], "submit");
    child.request_origin = DeletionRequestOrigin::Owner;
    let observed = observation(&child, false);
    assert_eq!(observed["reason"], "owner_preparation_pending");
    assert_eq!(observed["commands"].as_array().unwrap().len(), 1);
    child.blocked_reason = Some("unsafe inventory".into());
    let observed = observation(&child, false);
    assert_eq!(
        observed["commands"][1],
        json!([
            "buzz-admin",
            "deletions",
            "unblock",
            child.id.to_string(),
            "--unblocked-by",
            "<operator>",
            "--reason",
            "<remediation>"
        ])
    );
    child.stage = DeletionStage::Aborted;
    let observed = observation(&child, false);
    assert_eq!(
        observed["commands"][1],
        json!([
            "buzz-admin",
            "deletions",
            "submit",
            "--host",
            child.community_host,
            "--requested-by",
            "<operator>"
        ])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn submit_wait_approve_observe_and_complete_without_advancing_parent() {
    let f = fixture().await;
    let (media, server) = submission_media().await;
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Waiting
    );
    server.await.unwrap();
    let c = child(&f).await;
    assert_eq!(c.stage, DeletionStage::Inventoried);
    assert_eq!(c.requested_by, "submitter");
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    let wait = &summary.unfinished_stages[0];
    assert_eq!(wait.state, "waiting_external");
    let observation = wait.external_observation.as_ref().unwrap();
    assert_eq!(observation["child_request_id"], c.id.to_string());
    assert_eq!(observation["reason"], "approval_required");
    assert!(observation["observed_at"].is_string());
    assert!(observation["commands"].as_array().unwrap().len() >= 2);
    assert!(!summary.complete);
    assert_eq!(summary.communities_not_reached.len(), 1);
    assert_eq!(summary.saved.execution.retry_count, 0);
    assert_eq!(summary.saved.request.status, "running");
    assert_eq!(summary.saved.stage_history.len(), 1);
    assert!(f
        .db
        .deletion_store()
        .inspect(c.id)
        .await
        .unwrap()
        .approval
        .is_none());
    let media = lookup_only_media();
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Waiting
    );
    assert_eq!(child(&f).await, c); // Read-only repeat: no child writes.
    f.db.deletion_store()
        .approve(c.id, "community-approver", None)
        .await
        .unwrap();
    // Summary never refreshes child state, even though the DB now says approved.
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    assert_eq!(
        summary.unfinished_stages[0]
            .external_observation
            .as_ref()
            .unwrap()["child_stage"],
        "inventoried"
    );
    handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
        .await
        .unwrap();
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    assert_eq!(
        summary.unfinished_stages[0]
            .external_observation
            .as_ref()
            .unwrap()["child_stage"],
        "approved"
    );
    // Fixture the independent community worker's terminal result; this test
    // does not execute destructive community deletion.
    sqlx::query("UPDATE community_deletion_requests SET stage = 'retention_pending', completed_at = now() WHERE id = $1")
        .bind(c.id).execute(f.db.pool()).await.unwrap();
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Completed
    );
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    assert_eq!(summary.completed_stages, 1);
    assert!(summary.unfinished_stages.is_empty());
    assert!(!summary.complete); // Only routing may finish the parent.
    let row = &summary.saved.stage_history[0];
    assert!(row.is_current);
    assert!(row.metadata_json.get("waiting").is_none());
    assert_eq!(row.metadata_json["result"]["retention_pending"], true);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preparation_failure_is_recoverable_and_does_not_duplicate_child() {
    let f = fixture().await;
    let media = lookup_only_media();
    assert!(
        handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
            .await
            .is_err()
    );
    let c = child(&f).await;
    assert_eq!(c.stage, DeletionStage::Submitted);
    let before = f.store.inspect(f.row.request_id).await.unwrap();
    assert_eq!(before.request.status, "retry_wait");
    assert_eq!(before.execution.retry_count, 1);
    assert!(before.stage_history[0].last_error.is_some());
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = now() - interval '1 second' WHERE id = $1")
        .bind(f.row.request_id).execute(f.db.pool()).await.unwrap();
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Waiting
    );
    assert_eq!(child(&f).await, c);
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    assert_eq!(
        summary.unfinished_stages[0]
            .external_observation
            .as_ref()
            .unwrap()["reason"],
        "preparation_required"
    );
    assert_eq!(summary.saved.execution.retry_count, 1);
    assert!(summary.saved.execution.last_error.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_submission_reuses_request_and_preserves_terminal_states() {
    let f = fixture().await;
    let store = f.db.deletion_store();
    let (media, server) = submission_media().await;
    let (a, b) = tokio::join!(
        submit_and_prepare(
            &store,
            &media,
            None,
            "operator-a",
            Some("a"),
            Some(f.community)
        ),
        submit_and_prepare(
            &store,
            &media,
            None,
            "operator-b",
            Some("b"),
            Some(f.community)
        )
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.id, b.id);
    server.await.unwrap(); // Exactly one caller inventories the child.
    let inventoried = child(&f).await;
    assert_eq!(inventoried.stage, DeletionStage::Inventoried);
    let media = lookup_only_media();
    assert_eq!(
        submit_and_prepare(&store, &media, None, "another", None, Some(f.community))
            .await
            .unwrap(),
        inventoried
    );
    store.abort(a.id, "operator", "cancelled").await.unwrap();
    let aborted = child(&f).await;
    assert_eq!(aborted.stage, DeletionStage::Aborted);
    assert_eq!(
        submit_and_prepare(&store, &media, None, "another", None, Some(f.community))
            .await
            .unwrap(),
        aborted
    );
    assert_eq!(
        handle(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
            .await
            .unwrap(),
        OwnedCommunityOutcome::Waiting
    );
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    assert_eq!(
        summary.unfinished_stages[0]
            .external_observation
            .as_ref()
            .unwrap()["reason"],
        "operator_intervention"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stale_generation_cannot_save_wait_or_execute_handler() {
    let f = fixture().await;
    f.store.start_stage(&f.lease, f.row.id).await.unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(f.row.request_id).execute(f.db.pool()).await.unwrap();
    let new = f
        .store
        .claim(f.row.request_id, &f.lease.owner, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new.generation, f.lease.generation + 1);
    let before = serde_json::to_value(f.store.inspect(f.row.request_id).await.unwrap()).unwrap();
    let error = f
        .store
        .save_stage_waiting(&f.lease, f.row.id, json!({"kind": "test"}))
        .await
        .unwrap_err();
    assert!(buzz_db::deletion::is_stale_deletion_lease(&error));
    assert!(handle(
        &f.store,
        &f.lease,
        &f.row,
        &f.db.deletion_store(),
        &lookup_only_media()
    )
    .await
    .is_err());
    assert!(f
        .db
        .deletion_store()
        .find_for_community(f.community)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        serde_json::to_value(f.store.inspect(f.row.request_id).await.unwrap()).unwrap(),
        before
    );
    f.store
        .save_stage_waiting(
            &new,
            f.row.id,
            json!({"kind": "other_external_system", "operator_action": "wait for export"}),
        )
        .await
        .unwrap();
    let summary = summarize_request(&f.store, f.row.request_id).await.unwrap();
    assert_eq!(summary.unfinished_stages[0].state, "waiting_external");
    assert_eq!(
        summary.unfinished_stages[0]
            .external_observation
            .as_ref()
            .unwrap()["kind"],
        "other_external_system"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn wrong_community_or_inputs_cannot_submit_a_child() {
    let f = fixture().await;
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json = jsonb_set(metadata_json, '{inputs,target}', '\"different-user\"') WHERE id = $1")
        .bind(f.row.id).execute(f.db.pool()).await.unwrap();
    // The caller's original row is stale; validation must reload persisted inputs.
    assert!(handle(
        &f.store,
        &f.lease,
        &f.row,
        &f.db.deletion_store(),
        &lookup_only_media()
    )
    .await
    .is_err());
    assert!(f
        .db
        .deletion_store()
        .find_for_community(f.community)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cli_submission_preparation_can_resume_but_cannot_reset_approved_request() {
    let f = fixture().await;
    let store = f.db.deletion_store();
    // The same function used by the CLI leaves a submitted request when storage fails.
    assert!(submit_and_prepare(
        &store,
        &lookup_only_media(),
        Some("owned.example"),
        "operator",
        None,
        None
    )
    .await
    .is_err());
    let submitted = child(&f).await;
    assert_eq!(submitted.stage, DeletionStage::Submitted);
    let (media, server) = submission_media().await;
    let prepared = submit_and_prepare(
        &store,
        &media,
        Some("owned.example"),
        "operator",
        None,
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(prepared.id, submitted.id);
    assert_eq!(prepared.stage, DeletionStage::Inventoried);
    store.approve(prepared.id, "approver", None).await.unwrap();
    let approved = child(&f).await;
    assert!(submit_and_prepare(
        &store,
        &lookup_only_media(),
        Some("owned.example"),
        "operator",
        None,
        None
    )
    .await
    .is_err());
    assert_eq!(child(&f).await, approved);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn submission_checks_community_identity_before_inserting() {
    let f = fixture().await;
    let other =
        f.db.ensure_configured_community("other.example")
            .await
            .unwrap()
            .id;
    let store = f.db.deletion_store();
    assert!(submit_and_prepare(
        &store,
        &lookup_only_media(),
        Some("owned.example"),
        "operator",
        None,
        Some(other)
    )
    .await
    .is_err());
    assert!(store
        .find_for_community(f.community)
        .await
        .unwrap()
        .is_none());
    assert!(store.find_for_community(other).await.unwrap().is_none());
    assert!(store.submit(None, "operator", None, None).await.is_err());
    // A request for the pinned ID must not hide a mismatched supplied host.
    store
        .submit(Some("other.example"), "operator", None, None)
        .await
        .unwrap();
    assert!(submit_and_prepare(
        &store,
        &lookup_only_media(),
        Some("owned.example"),
        "operator",
        None,
        Some(other)
    )
    .await
    .is_err());
    assert!(store
        .find_for_community(f.community)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn host_submission_and_parent_submission_share_one_admission_path() {
    let f = fixture().await;
    let store = f.db.deletion_store();
    // The operator submits first; parent submission must return this exact
    // unprepared row without accessing storage or changing its provenance.
    let submitted = store
        .submit(Some("owned.example"), "operator", Some("manual"), None)
        .await
        .unwrap();
    assert_eq!(
        submit_and_prepare(
            &store,
            &lookup_only_media(),
            None,
            "parent",
            None,
            Some(f.community)
        )
        .await
        .unwrap(),
        submitted
    );
    // Once aborted, only the explicit host-based operator path may replace it.
    store
        .abort(submitted.id, "operator", "cancel")
        .await
        .unwrap();
    let aborted = child(&f).await;
    assert_eq!(
        submit_and_prepare(
            &store,
            &lookup_only_media(),
            None,
            "parent",
            None,
            Some(f.community)
        )
        .await
        .unwrap(),
        aborted
    );
    let replacement = store
        .submit(Some("owned.example"), "operator", None, None)
        .await
        .unwrap();
    assert_ne!(replacement.id, submitted.id);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_cli_and_parent_submission_converge_without_repreparing() {
    let f = fixture().await;
    let store = f.db.deletion_store();
    let (media, server) = submission_media().await;
    let (cli, parent) = tokio::join!(
        submit_and_prepare(&store, &media, Some("owned.example"), "cli", None, None),
        submit_and_prepare(&store, &media, None, "parent", None, Some(f.community))
    );
    let parent = parent.unwrap();
    match cli {
        Ok(cli) => assert_eq!(cli.id, parent.id),
        Err(error) => assert!(matches!(
            error.downcast_ref::<buzz_db::DbError>(),
            Some(buzz_db::DbError::DeletionSafety(_))
        )),
    }
    server.await.unwrap(); // Only the creator prepares; three listings total.
    let saved = child(&f).await;
    assert_eq!(saved.id, parent.id);
    assert_eq!(saved.stage, DeletionStage::Inventoried);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM community_deletion_requests WHERE community_id = $1",
    )
    .bind(f.community.as_uuid())
    .fetch_one(f.db.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn database_failure_is_not_hidden_by_an_existing_child() {
    let f = fixture().await;
    f.db.deletion_store()
        .submit(Some("owned.example"), "cli", None, None)
        .await
        .unwrap();
    let mut lock = f.db.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM communities WHERE id = $1 FOR UPDATE")
        .bind(f.community.as_uuid())
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET lock_timeout = '50ms'")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let store = Db::from_pool(pool).deletion_store();
    let error = submit_and_prepare(
        &store,
        &lookup_only_media(),
        None,
        "parent",
        None,
        Some(f.community),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<buzz_db::DbError>(),
        Some(buzz_db::DbError::Sqlx(_))
    ));
    lock.rollback().await.unwrap();
}
