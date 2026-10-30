use super::user_deletion_test_support::Fixture;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn http_and_websocket_messages_honor_user_and_agent_fences() {
    let f = Fixture::new().await;
    let root = f.message(&f.peer, 9, None);
    assert!(f.send(&f.peer, &root, false).await.0);
    for websocket in [false, true] {
        for key in [&f.user, &f.agent, &f.peer] {
            assert!(
                f.send(key, &f.message(key, 9, Some(&root)), websocket)
                    .await
                    .0
            );
        }
    }
    let counters: (i64, i64) = sqlx::query_as("SELECT reply_count::bigint, descendant_count::bigint FROM thread_metadata WHERE community_id = $1 AND event_id = $2").bind(f.tenant.community().as_uuid()).bind(root.id.as_bytes().as_slice()).fetch_one(&f.pool).await.unwrap();
    assert_eq!(counters, (6, 6), "fixture must create real replies");
    f.fence().await;
    for websocket in [false, true] {
        for key in [&f.user, &f.agent] {
            for kind in [9, 40002, 45001, 45003] {
                let event = f.message(key, kind, Some(&root));
                let (accepted, reason) = f.send(key, &event, websocket).await;
                assert!(!accepted, "fenced event accepted");
                assert_eq!(
                    reason,
                    "restricted: identity is restricted for user deletion"
                );
                assert!(f
                    .state
                    .db
                    .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                    .await
                    .unwrap()
                    .is_none());
            }
        }
    }
    let after: (i64, i64) = sqlx::query_as("SELECT reply_count::bigint, descendant_count::bigint FROM thread_metadata WHERE community_id = $1 AND event_id = $2").bind(f.tenant.community().as_uuid()).bind(root.id.as_bytes().as_slice()).fetch_one(&f.pool).await.unwrap();
    assert_eq!(after, counters, "rejected replies must not update counters");
    for websocket in [false, true] {
        assert!(
            f.send(&f.peer, &f.message(&f.peer, 9, Some(&root)), websocket)
                .await
                .0
        );
    }
}
