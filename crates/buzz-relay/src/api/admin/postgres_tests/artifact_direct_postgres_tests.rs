//! The direct moderation guard uses a real signed channel artifact fixture.
mod postgres_tests {
    use super::super::*;
    /// An artifact revision is not admin-deletable: open NIP-AR syncs would
    /// get no live signal (they see `op=delete` and kind 9005). The action
    /// fails terminally and the head stays. Mutation: drop the kind check in
    /// `execute_delete_with_marker` → 200 and the head is gone → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn direct_delete_refuses_artifact_revisions() {
        let (pool, community, host, state) = direct_fixture().await;
        // Seed a signed channel revision. A channel-less note with only its
        // database kind changed has no artifact audience and now fails closed
        // before the direct-action guard, so it cannot exercise this contract.
        let home = Uuid::new_v4();
        let keys = test_operator_keys();
        sqlx::query("INSERT INTO channels(community_id,id,name,channel_type,visibility,created_by) VALUES($1,$2,'direct-artifact','stream','open',$3)")
        .bind(community.as_uuid()).bind(home).bind(keys.public_key().as_bytes().as_slice())
        .execute(&pool).await.unwrap();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(45010), "channel artifact")
            .tags([
                nostr::Tag::parse(["ar", "1"]).unwrap(),
                nostr::Tag::parse(["h", &home.to_string()]).unwrap(),
                nostr::Tag::parse(["d", &Uuid::new_v4().to_string()]).unwrap(),
                nostr::Tag::parse(["type", "example.settings"]).unwrap(),
                nostr::Tag::parse(["title", "Direct artifact"]).unwrap(),
                nostr::Tag::parse(["op", "create"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        state
            .db
            .insert_event(community, &event, Some(home))
            .await
            .unwrap();
        let id = event.id.to_hex();
        let path = format!("/events/{id}/delete?communityHost={host}");
        let (status, body) = direct_post(
            &state,
            &path,
            serde_json::json!({ "requestId": Uuid::new_v4() }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(
            body.to_string()
                .contains("artifact revisions cannot be deleted"),
            "{body}"
        );
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE community_id = $1 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(live, 1);
    }
}
