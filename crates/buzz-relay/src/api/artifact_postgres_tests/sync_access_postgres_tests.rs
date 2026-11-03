use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_redaction_and_query_exit_return_removals() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let id = Uuid::new_v4();
    let first = f.revision(id, "create", None);
    let first = EventBuilder::new(Kind::Custom(45010), first.content.clone())
        .tags(
            first
                .tags
                .iter()
                .cloned()
                .chain([Tag::parse(["project", "alpha"]).unwrap()]),
        )
        .sign_with_keys(&f.owner)
        .unwrap();
    accept(&f, &first).await;
    let scope = json!({"artifact":"current","#type":["buzz.task"],"#project":["alpha"]});
    let (_, page) = f
        .request("/query", json!([{"artifact":"incremental","query":scope}]))
        .await;
    assert_eq!(page["changes"][0]["type"], "upsert");
    let changed = f.revision(id, "update", Some(&first));
    accept(&f, &changed).await;
    let (status, delta) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":scope,"checkpoint":page["checkpoint"]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {delta}");
    assert_eq!(
        delta["changes"],
        json!([{ "type":"remove","artifactId":id }])
    );
    let before = sync(&f, None).await;
    assert!(buzz_db::event::soft_delete_event_and_update_thread(
        &f.pool,
        f.community,
        changed.id.as_bytes(),
        None,
        None
    )
    .await
    .unwrap());
    let after = sync(&f, Some(&before["checkpoint"])).await;
    assert_eq!(
        after["changes"],
        json!([{ "type":"remove","artifactId":id }])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_rejoin_replays_old_heads_without_resetting_on_task_edits() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let id = Uuid::new_v4();
    let first = f.revision_as(&f.owner, f.private, id, "create", None);
    accept(&f, &first).await;
    sqlx::query("UPDATE artifact_heads SET modified_at='2000-01-01' WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
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
    let (_, page) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    buzz_db::channel_members::remove_member(
        &f.pool,
        f.community,
        f.private,
        f.peer.public_key().as_bytes(),
        f.owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
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
    let (status, delta) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":page["checkpoint"]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {delta}");
    assert_eq!(delta["changes"][0]["event"]["id"], first.id.to_hex());
    let changed = f.revision_as(&f.owner, f.private, id, "update", Some(&first));
    accept(&f, &changed).await;
    let (status, delta) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":delta["checkpoint"]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {delta}");
    assert_eq!(delta["changes"][0]["event"]["id"], changed.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_ignores_unchanged_visibility_and_resets_on_ttl_changes() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let permanent_id = Uuid::new_v4();
    let permanent = f.revision(permanent_id, "create", None);
    accept(&f, &permanent).await;
    let first = sync(&f, None).await;
    buzz_db::channel::update_channel(
        &f.pool,
        f.community,
        f.home,
        buzz_db::channel::ChannelUpdate {
            visibility: Some("open".into()),
            name: Some("renamed".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let renamed = sync(&f, Some(&first["checkpoint"])).await;
    assert!(renamed["checkpoint"].is_string());
    buzz_db::channel::update_channel(
        &f.pool,
        f.community,
        f.private,
        buzz_db::channel::ChannelUpdate {
            ttl_seconds: Some(Some(3600)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (status, reset) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":renamed["checkpoint"]}]),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(reset["reason"], "policy_changed");
    let next = sync(&f, None).await;
    assert!(next["checkpoint"].is_string());
    let expiring_id = Uuid::new_v4();
    let expiring = f.revision_as(&f.owner, f.private, expiring_id, "create", None);
    accept(&f, &expiring).await;
    for checkpoint in [None, Some(&next["checkpoint"])] {
        let page = sync(&f, checkpoint).await;
        assert_eq!(page["changes"].as_array().unwrap().len(), 1);
        assert_eq!(page["changes"][0]["artifactId"], permanent_id.to_string());
        assert_eq!(page["changes"][0]["type"], "upsert");
        assert!(page["checkpoint"].is_string());
    }
    let (status, current) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","action":"current","query":query(),
                "artifactIds":[permanent_id,expiring_id]
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {current}");
    assert_eq!(current["changes"][0]["type"], "upsert");
    assert_eq!(
        current["changes"][1],
        json!({"type":"remove","artifactId":expiring_id})
    );
    let (status, named) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":{"artifact":"current","#h":[f.private]}
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {named}");
    assert_eq!(named["changes"], json!([]));
    let moved = f.revision_as(&f.owner, f.private, permanent_id, "move", Some(&permanent));
    accept(&f, &moved).await;
    let delta = sync(&f, Some(&next["checkpoint"])).await;
    assert_eq!(
        delta["changes"],
        json!([{"type":"remove","artifactId":permanent_id}])
    );
    let (status, scoped) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":{"artifact":"current","#h":[f.home]},
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {scoped}");
    assert_eq!(scoped["changes"], json!([]));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_shrinks_large_pages_and_current_batches_fail_explicitly() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let mut ids = Vec::new();
    for _ in 0..40 {
        let id = Uuid::new_v4();
        let base = f.revision(id, "create", None);
        let event = EventBuilder::new(Kind::Custom(45010), "x".repeat(260_000))
            .tags(base.tags.clone())
            .sign_with_keys(&f.owner)
            .unwrap();
        accept(&f, &event).await;
        ids.push(id);
    }
    let first = sync(&f, None).await;
    assert_eq!(first["changes"].as_array().unwrap().len(), 20);
    assert!(first["nextCursor"].is_string());
    let (status, last) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":query(),"cursor":first["nextCursor"]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {last}");
    assert_eq!(last["changes"].as_array().unwrap().len(), 20);
    assert!(last["checkpoint"].is_string());
    let (status, body) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","action":"current","query":query(),"artifactIds":ids}]),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["code"], "incremental_limit");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_http_reads_and_capability_are_available() {
    use tower::ServiceExt;
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let response = crate::router::build_router(f.state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/info")
                .header("host", &f.host)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        info["artifacts"]["sync"],
        json!({"version":1,"max_events":4096})
    );
    assert_eq!(
        info["artifacts"]["incremental"],
        json!({"version":1,"overlapSeconds":120,
        "pageRows":100,"currentRows":100,"responseBytes":8388608,
        "permanentChannelsOnly":true,"checkpointLifetimeSeconds":2592000})
    );
    let (status, _) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn incremental_resets_when_a_move_source_loses_access() {
    let _guard = SERIAL.lock().await;
    for delete_channel in [false, true] {
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
        let heads: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM artifact_heads WHERE community_id=$1 AND channel_id=$2",
        )
        .bind(f.community.as_uuid())
        .bind(f.home)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(heads, 0);
        if delete_channel {
            assert!(
                buzz_db::channel::soft_delete_channel(&f.pool, f.community, f.home)
                    .await
                    .unwrap()
            );
        } else {
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
        }
        let (status, delta) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","query":query(),"checkpoint":read["checkpoint"]}]),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(delta["reason"], "policy_changed");
        assert!(!delta.to_string().contains(&f.home.to_string()));
        let (status, current) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","action":"current","query":query(),"artifactIds":[id]}]),
            )
            .await;
        assert!(status.is_success());
        assert_eq!(
            current["changes"],
            json!([{"type":"remove","artifactId":id}])
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_policy_update_can_pass_a_channel_key_share() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM channels WHERE community_id=$1 AND id=$2 FOR KEY SHARE")
        .bind(f.community.as_uuid())
        .bind(f.home)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        buzz_db::channel::update_channel(
            &f.pool,
            f.community,
            f.home,
            buzz_db::channel::ChannelUpdate {
                visibility: Some("private".into()),
                ..Default::default()
            },
        ),
    )
    .await
    .expect("policy update must permit a foreign-key reader")
    .unwrap();
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_progress_replays_queries_with_escaped_values() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let scope = json!({"artifact":"current","#type":["buzz.task"],"#project":vec!["\u{1}".repeat(4096);15]});
    let (status, first) = f
        .request("/query", json!([{"artifact":"incremental","query":scope}]))
        .await;
    assert!(status.is_success(), "{status}: {first}");
    assert!(first["checkpoint"].as_str().unwrap().len() < 200_000);
    let (status, next) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","query":scope,"checkpoint":first["checkpoint"]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {next}");
    assert_eq!(next["changes"], json!([]));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_current_intersects_known_ids_with_query_scope() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let included = Uuid::new_v4();
    let excluded = Uuid::new_v4();
    let event = f.revision(included, "create", None);
    accept(&f, &event).await;
    accept(&f, &f.revision(excluded, "create", None)).await;
    let (status, body) = f
        .request(
            "/query",
            json!([{"artifact":"incremental","action":"current",
                "query":{"artifact":"current","#type":["buzz.task"],"#d":[included]},
                "artifactIds":[included,excluded]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["changes"][0]["artifactId"], included.to_string());
    assert_eq!(body["changes"][0]["event"]["id"], event.id.to_hex());
    assert_eq!(
        body["changes"][1],
        json!({"type":"remove","artifactId":excluded})
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn incremental_unlisted_channels_require_membership_or_named_scope() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let id = Uuid::new_v4();
    let event = f.revision(id, "create", None);
    accept(&f, &event).await;
    let (_, before) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    assert_eq!(before["changes"][0]["event"]["id"], event.id.to_hex());
    buzz_db::channel::update_channel(
        &f.pool,
        f.community,
        f.home,
        buzz_db::channel::ChannelUpdate {
            unlisted: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (status, reset) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query(),"checkpoint":before["checkpoint"]}]),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(reset["reason"], "policy_changed");
    for (scope, visible) in [
        (query(), false),
        (
            json!({"artifact":"current","#type":["buzz.task"],"#h":[f.home]}),
            true,
        ),
    ] {
        let (status, page) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","query":scope}]),
            )
            .await;
        assert!(status.is_success(), "{page}");
        assert_eq!(
            page["changes"].as_array().unwrap().len(),
            usize::from(visible)
        );
        let (status, current) = f.request_as(&f.peer, "/query", json!([{"artifact":"incremental","action":"current","query":scope,"artifactIds":[id]}])).await;
        assert!(status.is_success(), "{current}");
        assert_eq!(
            current["changes"][0]["type"],
            if visible { "upsert" } else { "remove" }
        );
    }
    assert_eq!(
        sync(&f, None).await["changes"][0]["event"]["id"],
        event.id.to_hex()
    );
    buzz_db::channel::update_channel(
        &f.pool,
        f.community,
        f.home,
        buzz_db::channel::ChannelUpdate {
            unlisted: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, listed) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    assert_eq!(listed["changes"][0]["event"]["id"], event.id.to_hex());
}

/// An incremental poll's access line counts its changes, removals included:
/// an empty poll is DEBUG and a poll with changes is INFO. Mutation: count only array responses in `query_events` → the
/// empty poll has no `result_count` and logs INFO → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn incremental_poll_access_level_follows_its_change_count() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    accept(&f, &f.revision(Uuid::new_v4(), "create", None)).await;
    let poll = |types: &str| json!([{"artifact":"incremental","query":{"artifact":"current","#type":[types]}}]);
    let line = |out: &crate::access::tests::Captured| {
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "one access line: {lines:?}");
        lines[0].clone()
    };

    let (_capture, out) = crate::access::tests::capture_scoped();
    let (status, body) = f.request("/query", poll("buzz.none")).await;
    assert!(status.is_success(), "{status}: {body}");
    let empty = line(&out);
    assert_eq!(
        (&empty["level"], &empty["result_count"]),
        (&"DEBUG".into(), &0.into()),
        "{empty}"
    );

    let (_capture, out) = crate::access::tests::capture_scoped();
    let (status, body) = f.request("/query", poll("buzz.task")).await;
    assert!(status.is_success(), "{status}: {body}");
    let served = line(&out);
    assert_eq!(
        (&served["level"], &served["result_count"]),
        (&"INFO".into(), &1.into()),
        "{served}"
    );
}

/// A wrapped incremental read dropped during its database work still
/// writes its one WARN aborted line with the standard filter summary, and
/// without its artifact predicates. Mutation: record the artifact summary
/// after `artifact::query` returns → the line has no `kinds` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn dropped_incremental_read_keeps_its_filter_summary() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let marker = "private-predicate-marker";
    let body = json!([{"artifact":"incremental","query":{
        "artifact":"current","kinds":[45010],"#h":[f.home],"#project":[marker]}}]);
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE artifact_heads IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let (capture, out) = crate::access::tests::capture_access_only(tracing::Level::WARN);
    let mut request = Box::pin(f.request("/query", body));
    // Poll the request until its candidate read waits on the lock.
    loop {
        tokio::select! {
            _ = &mut request => panic!("the read must wait on the locked table"),
            () = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
        }
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
        )
        .bind(blocker_pid)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        if blocked {
            break;
        }
    }
    drop(request);
    drop(capture);
    lock.rollback().await.unwrap();
    let lines = out.lines();
    assert_eq!(lines.len(), 1, "exactly one access line: {lines:?}");
    let line = &lines[0];
    assert_eq!(
        (&line["request"], &line["level"]),
        (&"aborted".into(), &"WARN".into()),
        "{line}"
    );
    assert_eq!(line["route"], "/query", "{line}");
    assert_eq!(line["kinds"], "45010", "{line}");
    assert_eq!(line["channel_ids"], f.home.to_string(), "{line}");
    assert!(!out.raw().contains(marker), "{}", out.raw());
}
