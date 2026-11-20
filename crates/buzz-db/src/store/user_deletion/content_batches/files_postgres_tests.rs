use super::scrub_postgres_tests::{options, setup};
use super::*;
use crate::user_deletion::ExecutionMode;
use nostr::{EventBuilder, Kind};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn historical_file_publications_follow_source_scope() {
    let (db, c, _, alice, bob) = setup().await;
    let other = db
        .ensure_configured_community(&format!("files-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let source = EventBuilder::new(Kind::Custom(1063), "personal file description")
        .sign_with_keys(&alice)
        .unwrap();
    let survivor = EventBuilder::new(Kind::Custom(1063), "another publisher's description")
        .sign_with_keys(&bob)
        .unwrap();
    for community in [c, other] {
        db.ensure_user(community, bob.public_key().as_bytes())
            .await
            .unwrap();
        db.insert_event(community, &source, None).await.unwrap();
        db.insert_event(community, &survivor, None).await.unwrap();
    }
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let result = db
            .user_deletion_store()
            .run_message_batches(&options(c, &alice), mode, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(result.items, 1);
    }
    for (community, expected) in [(c, 1_i64), (other, 2)] {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, expected);
    }
    let preserved: String =
        sqlx::query_scalar("SELECT content FROM events WHERE community_id=$1 AND id=$2")
            .bind(c.as_uuid())
            .bind(survivor.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(preserved, survivor.content);
    assert_eq!(
        db.user_deletion_store()
            .run_message_batches(
                &options(c, &alice),
                ExecutionMode::Delete,
                None,
                &mut Vec::new()
            )
            .await
            .unwrap()
            .items,
        0
    );
}
