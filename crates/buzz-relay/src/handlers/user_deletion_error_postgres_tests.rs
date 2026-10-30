use super::group_state_postgres_tests::create_channel;
use super::user_deletion_test_support::Fixture;
use buzz_db::user_deletion_fences::{self as fences, FenceIdentity};
use nostr::{EventBuilder, Kind, Tag};
use std::{sync::Arc, time::Duration};

#[test]
fn identity_error_mapping_preserves_unrelated_failures() {
    use crate::handlers::ingest::{event_write_error, IngestError};
    assert!(
        matches!(event_write_error(&buzz_db::DbError::UserDeletionRestricted),
        Some(IngestError::Rejected(message)) if message.starts_with("restricted:"))
    );
    assert!(
        matches!(event_write_error(&buzz_db::DbError::EventAuthorOwnershipChanged),
        Some(IngestError::Internal(message)) if message.starts_with("error:"))
    );
    for error in [
        buzz_db::DbError::AccessDenied("community is write-fenced (deleting)".into()),
        buzz_db::DbError::AccessDenied(
            "deletion-lock admission requires read committed isolation".into(),
        ),
        buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut),
    ] {
        assert!(event_write_error(&error).is_none());
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn ownership_races_are_retryable_over_http_and_websocket() {
    for websocket in [false, true] {
        for kind in [9, 9000, 9002] {
            let f = Arc::new(Fixture::new().await);
            f.state
                .db
                .ensure_user(f.tenant.community(), f.peer.public_key().as_bytes())
                .await
                .unwrap();
            let channel = create_channel(&f.state, &f.tenant, &f.agent, &[]).await;
            let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
            if kind == 9000 {
                tags.push(Tag::parse(["p", &f.peer.public_key().to_hex(), "member"]).unwrap());
            } else if kind == 9002 {
                tags.push(Tag::parse(["name", "renamed after retry"]).unwrap());
            }
            let event = EventBuilder::new(Kind::Custom(kind), "owner race")
                .tags(tags)
                .sign_with_keys(&f.agent)
                .unwrap();
            let mut assignment = f.pool.begin().await.unwrap();
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *assignment)
                .await
                .unwrap();
            fences::lock_exclusive(
                &mut assignment,
                &[FenceIdentity::agent(
                    f.tenant.community(),
                    f.agent.public_key(),
                )],
            )
            .await
            .unwrap();
            let writer_fixture = f.clone();
            let pending = event.clone();
            let sending = tokio::spawn(async move {
                writer_fixture
                    .submit(&writer_fixture.agent, &pending, websocket)
                    .await
            });
            tokio::time::timeout(Duration::from_secs(4), async {
                loop {
                    let blocked: bool = sqlx::query_scalar(
                        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))")
                        .bind(pid).fetch_one(&f.pool).await.unwrap();
                    if blocked { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("writer must reach the production identity guard");
            sqlx::query(
                "UPDATE users SET agent_owner_pubkey = $1 WHERE community_id = $2 AND pubkey = $3",
            )
            .bind(f.peer.public_key().to_bytes().as_slice())
            .bind(f.tenant.community().as_uuid())
            .bind(f.agent.public_key().to_bytes().as_slice())
            .execute(&mut *assignment)
            .await
            .unwrap();
            assignment.commit().await.unwrap();
            let (accepted, message, status) = sending.await.unwrap();
            assert!(!accepted);
            if websocket {
                assert_eq!(message, "error: internal server error");
            } else {
                assert_eq!(status, Some(axum::http::StatusCode::INTERNAL_SERVER_ERROR));
                assert_eq!(message, "internal server error");
            }
            assert!(f
                .state
                .db
                .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                .await
                .unwrap()
                .is_none());
            let (accepted, message) = f.send(&f.agent, &event, websocket).await;
            assert!(accepted, "kind {kind}: {message}");
        }
    }
}
