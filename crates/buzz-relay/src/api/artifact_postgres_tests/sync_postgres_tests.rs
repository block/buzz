use super::*;
use axum::http::StatusCode;
#[path = "sync_access_postgres_tests.rs"]
mod access_postgres_tests;
#[path = "sync_cursor_postgres_tests.rs"]
mod cursor_postgres_tests;
#[path = "sync_policy_postgres_tests.rs"]
mod policy_postgres_tests;
#[path = "sync_scale_postgres_tests.rs"]
mod scale_postgres_tests;

async fn fixture() -> Fixture {
    let mut f = Fixture::new().await;
    let mut state = (*f.state).clone();
    Arc::make_mut(&mut state.config).artifact_incremental_enabled = true;
    f.state = Arc::new(state);
    f
}
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn query() -> Value {
    json!({"artifact":"current","#type":["buzz.task"]})
}
async fn sync(f: &Fixture, checkpoint: Option<&Value>) -> Value {
    let mut request = json!({"artifact":"incremental","query":query()});
    if let Some(checkpoint) = checkpoint {
        request["checkpoint"] = checkpoint.clone();
    }
    let (status, body) = f.request("/query", json!([request])).await;
    assert!(status.is_success(), "{status}: {body}");
    body
}
async fn accept(f: &Fixture, event: &Event) {
    let envelope = buzz_core::artifact::validate(event).unwrap();
    let source = if envelope.op == buzz_core::artifact::ArtifactOp::Move {
        f.state
            .db
            .artifact_home(f.community, envelope.id)
            .await
            .unwrap()
    } else {
        None
    };
    assert!(matches!(
        f.state
            .db
            .accept_artifact(
                f.community,
                event,
                &envelope,
                source,
                &f.state.relay_keypair
            )
            .await
            .unwrap(),
        buzz_db::artifact::ArtifactOutcome::Accepted(_)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_recovers_late_commit_and_returns_explicit_deletion() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let first = sync(&f, None).await;
    assert_eq!(first["changes"], json!([]));
    let id = Uuid::new_v4();
    let event = f.revision(id, "create", None);
    accept(&f, &event).await;
    // Model a stamp assigned before the first read, followed by a later commit.
    sqlx::query("UPDATE artifact_heads SET modified_at=$1::timestamptz-interval '119 seconds' WHERE community_id=$2 AND artifact_id=$3")
        .bind(first["startedAt"].as_str().unwrap()).bind(f.community.as_uuid()).bind(id).execute(&f.pool).await.unwrap();
    let second = sync(&f, Some(&first["checkpoint"])).await;
    assert!(second["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["event"]["id"] == event.id.to_hex()));
    let deleted = f.revision(id, "delete", Some(&event));
    accept(&f, &deleted).await;
    let third = sync(&f, Some(&second["checkpoint"])).await;
    assert!(third["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == &json!({"type":"remove","artifactId":id})));
    assert_eq!(third["overlapSeconds"], 120);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_pages_equal_timestamps_and_replays_the_same_cursor() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    for _ in 0..103 {
        accept(&f, &f.revision(Uuid::new_v4(), "create", None)).await;
    }
    sqlx::query("UPDATE artifact_heads SET modified_at='2000-01-01' WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    let first = sync(&f, None).await;
    assert_eq!(first["changes"].as_array().unwrap().len(), 100);
    assert!(first["checkpoint"].is_null());
    let request = json!([{"artifact":"incremental","query":query(),"cursor":first["nextCursor"]}]);
    let (status, last) = f.request("/query", request.clone()).await;
    assert!(status.is_success(), "{status}: {last}");
    assert_eq!(last["changes"].as_array().unwrap().len(), 3);
    assert!(last["nextCursor"].is_null());
    assert!(last["checkpoint"].is_string());
    let (_, replay) = f.request("/query", request).await;
    assert_eq!(last, replay);
    let ids = first["changes"]
        .as_array()
        .unwrap()
        .iter()
        .chain(last["changes"].as_array().unwrap())
        .map(|c| c["artifactId"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 103);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_current_has_uniform_unavailable_and_rejects_duplicate_ids() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let id = Uuid::new_v4();
    let event = f.revision_as(&f.owner, f.private, id, "create", None);
    accept(&f, &event).await;
    let missing = Uuid::new_v4();
    let body = json!([{"artifact":"incremental","action":"current","query":query(),"artifactIds":[id,missing,"local-only"]}]);
    let (status, body) = f.request_as(&f.peer, "/query", body).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(
        body["changes"],
        json!([
            {"type":"remove","artifactId":id},{"type":"remove","artifactId":missing},{"type":"remove","artifactId":"local-only"}
        ])
    );
    let (status, _) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","action":"current","query":query(),"artifactIds":[id,id]}]),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","action":"current","query":query(),"artifactIds":[id]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["changes"][0]["event"]["id"], event.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_grant_finds_old_heads_and_removal_names_only_own_membership() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let id = Uuid::new_v4();
    let event = f.revision_as(&f.owner, f.private, id, "create", None);
    accept(&f, &event).await;
    sqlx::query("UPDATE artifact_heads SET modified_at='2000-01-01' WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    let (_, first) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    assert_eq!(first["changes"], json!([]));
    buzz_db::channel_members::add_member(
        &f.pool,
        f.community,
        f.private,
        f.peer.public_key().as_bytes(),
        buzz_db::channel_members::MemberRole::Member,
        Some(f.owner.public_key().as_bytes()),
    )
    .await
    .unwrap();
    let (_, granted) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":first["checkpoint"]}]),
        )
        .await;
    assert!(
        granted["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["event"]["id"] == event.id.to_hex()),
        "{granted}"
    );
    buzz_db::channel_members::remove_member(
        &f.pool,
        f.community,
        f.private,
        f.peer.public_key().as_bytes(),
        f.owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
    let (_, removed) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":granted["checkpoint"]}]),
        )
        .await;
    assert_eq!(
        removed["changes"],
        json!([{ "type":"removeChannel","channelId":f.private }])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_move_to_hidden_destination_removes_the_source_view() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let id = Uuid::new_v4();
    let first = f.revision(id, "create", None);
    accept(&f, &first).await;
    let (_, read) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    let moved = f.revision_as(&f.owner, f.private, id, "move", Some(&first));
    accept(&f, &moved).await;
    let (_, delta) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":read["checkpoint"]}]),
        )
        .await;
    assert_eq!(
        delta["changes"],
        json!([{ "type":"remove","artifactId":id }])
    );
    assert!(!delta.to_string().contains(&f.private.to_string()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_policy_changes_reset_without_hidden_channel_ids() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    for _ in 0..101 {
        accept(&f, &f.revision(Uuid::new_v4(), "create", None)).await;
    }
    let first = sync(&f, None).await;
    assert!(first["nextCursor"].is_string());
    buzz_db::channel::update_channel(
        &f.pool,
        f.community,
        f.home,
        buzz_db::channel::ChannelUpdate {
            visibility: Some("private".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (status, body) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":query(),"cursor":first["nextCursor"]}]),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body,
        json!({"code":"incremental_reset","error":"incremental_reset","reason":"policy_changed"})
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_progress_binds_reader_query_and_community() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let first = sync(&f, None).await;
    for (reader, scope) in [
        (&f.peer, query()),
        (
            &f.owner,
            json!({"artifact":"current","#type":["other.task"]}),
        ),
    ] {
        let (status, _) = f
            .request_as(
                reader,
                "/query",
                json!([{"artifact":"incremental","query":scope,"checkpoint":first["checkpoint"]}]),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let mut other = fixture().await;
    Arc::make_mut(&mut other.state).relay_keypair = f.state.relay_keypair.clone();
    let (status, _) = other
        .request_as(
            &f.owner,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":first["checkpoint"]}]),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let expired = crate::api::artifact_sync::expired_checkpoint_for_test(
        &f.state,
        first["checkpoint"].as_str().unwrap(),
    )
    .unwrap();
    let (status, body) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":expired}]),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "checkpoint_expired");
}
