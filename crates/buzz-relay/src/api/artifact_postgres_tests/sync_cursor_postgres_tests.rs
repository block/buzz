use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_move_cursor_hides_private_and_redacted_revision_ids() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let (_, empty) = f
        .request_as(
            &f.peer,
            "/query",
            json!([{"artifact":"incremental","query":query()}]),
        )
        .await;
    let mut current = std::collections::HashMap::new();
    let mut private_ids = Vec::new();
    for _ in 0..101 {
        let id = Uuid::new_v4();
        let created = f.revision(id, "create", None);
        accept(&f, &created).await;
        let moved = f.revision_as(&f.owner, f.private, id, "move", Some(&created));
        accept(&f, &moved).await;
        private_ids.push(moved.id.to_hex());
        current.insert(id, moved);
    }
    let (&id, previous) = current.iter().next().unwrap();
    let back = f.revision(id, "move", Some(previous));
    accept(&f, &back).await;
    let moved = f.revision_as(&f.owner, f.private, id, "move", Some(&back));
    accept(&f, &moved).await;
    private_ids.push(moved.id.to_hex());
    current.insert(id, moved);
    let stamp: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE artifact_revisions SET modified_at=$2 WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .bind(stamp)
        .execute(&f.pool)
        .await
        .unwrap();
    for redacted in [false, true] {
        let (status, first) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","query":query(),"checkpoint":empty["checkpoint"]}]),
            )
            .await;
        assert!(status.is_success(), "{status}: {first}");
        let cursor = first["nextCursor"].as_str().unwrap();
        let payload = cursor.split_once('.').unwrap().0;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        for private_id in &private_ids {
            assert!(
                !decoded.contains(private_id),
                "cursor exposes a private revision ID (redacted={redacted})"
            );
        }
        assert_eq!(first["changes"].as_array().unwrap().len(), 100);
        let (status, last) = f
            .request_as(
                &f.peer,
                "/query",
                json!([{"artifact":"incremental","query":query(),"cursor":cursor}]),
            )
            .await;
        assert!(status.is_success(), "{status}: {last}");
        assert!(last["checkpoint"].is_string());
        let mut ids = std::collections::HashSet::new();
        for change in first["changes"]
            .as_array()
            .unwrap()
            .iter()
            .chain(last["changes"].as_array().unwrap())
        {
            assert_eq!(change["type"], "remove");
            assert!(ids.insert(change["artifactId"].as_str().unwrap().to_owned()));
        }
        assert_eq!(ids.len(), 101);
        if !redacted {
            let boundary = first["changes"].as_array().unwrap().last().unwrap()["artifactId"]
                .as_str()
                .unwrap()
                .parse::<Uuid>()
                .unwrap();
            assert!(buzz_db::event::soft_delete_event_and_update_thread(
                &f.pool,
                f.community,
                current[&boundary].id.as_bytes(),
                None,
                None
            )
            .await
            .unwrap());
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_channel_scope_limits_candidates_and_keeps_source_removals() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
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
    for _ in 0..101 {
        accept(
            &f,
            &f.revision_as(&f.owner, f.private, Uuid::new_v4(), "create", None),
        )
        .await;
    }
    let deleted_id = Uuid::new_v4();
    let deleted = f.revision(deleted_id, "create", None);
    accept(&f, &deleted).await;
    accept(&f, &f.revision(deleted_id, "delete", Some(&deleted))).await;
    let id = Uuid::new_v4();
    let created = f.revision(id, "create", None);
    accept(&f, &created).await;
    let scoped_query = json!({"artifact":"current","#type":["buzz.task"],"#h":[f.home]});
    let (status, initial) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":scoped_query
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {initial}");
    assert_eq!(initial["changes"].as_array().unwrap().len(), 1);
    assert_eq!(initial["changes"][0]["event"]["id"], created.id.to_hex());
    assert!(initial["nextCursor"].is_null());
    assert!(initial["checkpoint"].is_string());
    let moved = f.revision_as(&f.owner, f.private, id, "move", Some(&created));
    accept(&f, &moved).await;
    let (status, delta) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":scoped_query,"checkpoint":initial["checkpoint"]
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {delta}");
    assert!(delta["changes"]
        .as_array()
        .unwrap()
        .contains(&json!({"type":"remove","artifactId":id})));
    assert!(!delta.to_string().contains(&f.private.to_string()));
    let restored = f.revision(id, "move", Some(&moved));
    accept(&f, &restored).await;
    for channel in [f.home, f.private] {
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')")
            .bind(f.community.as_uuid()).bind(channel).bind(f.peer.public_key().as_bytes().as_slice())
            .execute(&f.pool).await.unwrap();
    }
    buzz_db::channel_members::remove_member(
        &f.pool,
        f.community,
        f.private,
        f.owner.public_key().as_bytes(),
        f.owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
    let (status, returned) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":scoped_query,"checkpoint":delta["checkpoint"]
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {returned}");
    assert!(returned["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["event"]["id"] == restored.id.to_hex()));
    assert!(returned["changes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["type"] != "removeChannel"));
    buzz_db::channel_members::remove_member(
        &f.pool,
        f.community,
        f.home,
        f.owner.public_key().as_bytes(),
        f.owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
    let (status, removed) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":scoped_query,"checkpoint":returned["checkpoint"]
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {removed}");
    assert_eq!(
        removed["changes"],
        json!([{"type":"removeChannel","channelId":f.home}])
    );
    let (status, invalid_scope) = f
        .request(
            "/query",
            json!([{
                "artifact":"incremental","query":{"artifact":"current","#h":["not-a-uuid"]}
            }]),
        )
        .await;
    assert!(status.is_success(), "{status}: {invalid_scope}");
    assert_eq!(invalid_scope["changes"], json!([]));
}
