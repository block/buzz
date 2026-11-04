use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn durable_certification_resets_existing_checkpoints_after_the_overlap() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    sqlx::query("UPDATE channel_members SET sync_modified_at=clock_timestamp()-interval '1 day' WHERE community_id=$1")
        .bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    let first = sync(&f, None).await;
    let id = Uuid::new_v4();
    let event = f.revision(id, "create", None);
    accept(&f, &event).await;
    // Model a commit whose original stamp predates the checkpoint overlap.
    sqlx::query("UPDATE artifact_heads SET modified_at=$1::timestamptz-interval '121 seconds' WHERE community_id=$2 AND artifact_id=$3")
        .bind(first["startedAt"].as_str().unwrap()).bind(f.community.as_uuid()).bind(id).execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE artifact_sync_pending SET earliest_at=$1::timestamptz-interval '121 seconds' WHERE community_id=$2")
        .bind(first["startedAt"].as_str().unwrap()).bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    let missed = sync(&f, Some(&first["checkpoint"])).await;
    assert_eq!(missed["changes"], json!([]));
    assert_eq!(
        f.state.db.recover_artifact_certifications().await.unwrap(),
        1
    );
    for checkpoint in [&first["checkpoint"], &missed["checkpoint"]] {
        let (status, body) = f
            .request(
                "/query",
                json!([{
                    "artifact":"incremental", "query":query(), "checkpoint":checkpoint
                }]),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["reason"], "policy_changed");
    }
    let recovered = sync(&f, None).await;
    assert_eq!(recovered["changes"][0]["event"]["id"], event.id.to_hex());
    let (status, ordinary) = f.request("/query", json!([query()])).await;
    assert!(status.is_success());
    assert!(ordinary.to_string().contains(&event.id.to_hex()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn incremental_resets_for_first_artifact_during_channel_transition() {
    let _guard = SERIAL.lock().await;
    for delete_channel in [false, true] {
        let f = fixture().await;
        let mut blocker = f.pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM communities WHERE id=$1 FOR NO KEY UPDATE")
            .bind(f.community.as_uuid())
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let pool = f.pool.clone();
        let community = f.community;
        let home = f.home;
        let transition = tokio::spawn(async move {
            if delete_channel {
                assert!(
                    buzz_db::channel::soft_delete_channel(&pool, community, home)
                        .await
                        .unwrap()
                );
            } else {
                buzz_db::channel::update_channel(
                    &pool,
                    community,
                    home,
                    buzz_db::channel::ChannelUpdate {
                        visibility: Some("private".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                assert!(!transition.is_finished(), "an empty channel must still advance the recovery marker");
                let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)) AND query LIKE 'UPDATE communities SET artifact_policy_modified_at=%')")
                    .bind(blocker_pid).fetch_one(&f.pool).await.unwrap();
                if blocked { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("channel transition reaches the recovery marker");
        let id = Uuid::new_v4();
        let event = f.revision(id, "create", None);
        tokio::time::timeout(std::time::Duration::from_secs(2), accept(&f, &event))
            .await
            .expect("first artifact commits while the channel transition is uncommitted");
        let (status, first) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","query":query()}]),
            )
            .await;
        assert!(status.is_success(), "{status}: {first}");
        assert_eq!(first["changes"][0]["event"]["id"], event.id.to_hex());
        blocker.commit().await.unwrap();
        transition.await.unwrap();
        let (status, next) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","query":query(),"checkpoint":first["checkpoint"]}]),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(next["reason"], "policy_changed");
        assert!(!next.to_string().contains(&f.home.to_string()));
    }
}
