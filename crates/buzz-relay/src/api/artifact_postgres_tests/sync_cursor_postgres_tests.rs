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
