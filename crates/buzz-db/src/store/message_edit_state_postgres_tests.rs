use super::media::latest_message_edit_sql;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn effective_edit_uses_target_index_and_fresh_channel_authorization() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community("effective-edit.example")
        .await
        .unwrap()
        .id;
    let owner = Keys::generate();
    let outsider = Keys::generate();
    db.ensure_user(community, owner.public_key().as_bytes())
        .await
        .unwrap();
    let channel = db
        .create_channel(
            community,
            "edit",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let original = EventBuilder::new(Kind::Custom(45001), "original")
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    db.insert_event(community, &original, Some(channel))
        .await
        .unwrap();
    let tags = vec![
        Tag::parse(["h", &channel.to_string()]).unwrap(),
        Tag::parse(["e", &original.id.to_hex().to_uppercase()]).unwrap(),
    ];
    let edit = EventBuilder::new(Kind::Custom(40003), "replacement")
        .tags(tags)
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 1))
        .sign_with_keys(&owner)
        .unwrap();
    db.insert_event(community, &edit, Some(channel))
        .await
        .unwrap();
    // More unrelated edits than the old channel-wide cap; they must not
    // affect lookup cost or prevent editing this message. Synthetic IDs are
    // intentional: these rows are unrelated fixtures and must never be read.
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,channel_id) SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2,now(),40003,$3,'unrelated',$4,$5 FROM generate_series(1,10001) n")
        .bind(community.as_uuid()).bind(owner.public_key().as_bytes().as_slice())
        .bind(serde_json::json!([["h",channel.to_string()],["e","f".repeat(64)]]))
        .bind(edit.sig.serialize().as_slice()).bind(channel).execute(&pool).await.unwrap();
    sqlx::query("ANALYZE events").execute(&pool).await.unwrap();
    let snapshot = db
        .message_edit_state(
            community,
            channel,
            original.id.as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.original.id, original.id);
    assert_eq!(snapshot.edit.unwrap().id, edit.id);
    assert!(db
        .message_edit_state(
            community,
            channel,
            original.id.as_bytes(),
            outsider.public_key().as_bytes()
        )
        .await
        .unwrap()
        .is_none());
    db.ensure_user(community, outsider.public_key().as_bytes())
        .await
        .unwrap();
    db.add_member(
        community,
        channel,
        outsider.public_key().as_bytes(),
        crate::channel::MemberRole::Member,
        Some(owner.public_key().as_bytes()),
    )
    .await
    .unwrap();
    assert!(db
        .message_edit_state(
            community,
            channel,
            original.id.as_bytes(),
            outsider.public_key().as_bytes()
        )
        .await
        .unwrap()
        .is_some());
    db.remove_member(
        community,
        channel,
        outsider.public_key().as_bytes(),
        owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
    assert!(db
        .message_edit_state(
            community,
            channel,
            original.id.as_bytes(),
            outsider.public_key().as_bytes()
        )
        .await
        .unwrap()
        .is_none());
    let sql = format!("EXPLAIN (FORMAT JSON) {}", latest_message_edit_sql());
    let plan: serde_json::Value = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(community.as_uuid())
        .bind(channel)
        .bind(original.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .unwrap();
    let plan = plan.to_string();
    let child_indexes: Vec<String> = sqlx::query_scalar("SELECT child.relname FROM pg_inherits inheritance JOIN pg_class child ON child.oid=inheritance.inhrelid WHERE inheritance.inhparent='idx_events_media_edit_target'::regclass")
        .fetch_all(&pool).await.unwrap();
    assert!(
        child_indexes.iter().any(|index| plan.contains(index)),
        "{plan}"
    );
    assert!(
        plan.contains("decode(lower("),
        "target must be an index condition: {plan}"
    );
    assert!(!plan.contains("Seq Scan"), "{plan}");
    db.soft_delete_event_and_update_thread(community, edit.id.as_bytes(), None, None)
        .await
        .unwrap();
    assert!(db
        .message_edit_state(
            community,
            channel,
            original.id.as_bytes(),
            owner.public_key().as_bytes()
        )
        .await
        .unwrap()
        .unwrap()
        .edit
        .is_none());
    db.soft_delete_event_and_update_thread(community, original.id.as_bytes(), None, None)
        .await
        .unwrap();
    assert!(db
        .message_edit_state(
            community,
            channel,
            original.id.as_bytes(),
            owner.public_key().as_bytes()
        )
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn historical_edit_preflight_matches_selected_live_edit() {
    let sql = include_str!("../../../../docs/sql/private-media-edit-preflight.sql");
    // The checked-in operator query uses the exact normalized target expression.
    assert!(sql.contains(super::media::MEDIA_EDIT_TARGET_SQL));
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community("preflight.example")
        .await
        .unwrap()
        .id;
    let owner = Keys::generate();
    db.ensure_user(community, owner.public_key().as_bytes())
        .await
        .unwrap();
    let mut channels = Vec::new();
    for name in ["originals", "decoys"] {
        channels.push(
            db.create_channel(
                community,
                name,
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                owner.public_key().as_bytes(),
                None,
            )
            .await
            .unwrap()
            .id,
        );
    }
    let now = Timestamp::now().as_secs();
    for (index, kind, scenario, denied) in [
        (1, 45001, "empty", true),
        (2, 45003, "later-retained", false),
        (3, 40008, "deleted", false),
        (4, 45001, "mixed", true),
        (5, 45003, "cross-channel", false),
        (6, 9, "control", true),
        (7, 45001, "deleted-original", true),
    ] {
        let hash = format!("{index:064x}");
        let media = [vec![
            "imeta".into(),
            format!("url https://preflight.example/media/{hash}"),
            format!("x {hash}"),
        ]];
        let original = EventBuilder::new(Kind::Custom(kind), scenario)
            .tags([
                Tag::parse(["h", &channels[0].to_string()]).unwrap(),
                Tag::parse(media[0].clone()).unwrap(),
            ])
            .custom_created_at(Timestamp::from(now - 20))
            .sign_with_keys(&owner)
            .unwrap();
        db.insert_event(community, &original, Some(channels[0]))
            .await
            .unwrap();
        let channel = if scenario == "cross-channel" {
            channels[1]
        } else {
            channels[0]
        };
        let target = if scenario == "mixed" {
            original.id.to_hex().to_uppercase()
        } else {
            original.id.to_hex()
        };
        let empty = EventBuilder::new(Kind::Custom(40003), "empty")
            .tags([
                Tag::parse(["h", &channel.to_string()]).unwrap(),
                Tag::parse(["e", &target]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(now - 2))
            .sign_with_keys(&owner)
            .unwrap();
        db.insert_event(community, &empty, Some(channel))
            .await
            .unwrap();
        if scenario == "later-retained" {
            let retained = EventBuilder::new(Kind::Custom(40003), "retained")
                .tags([
                    Tag::parse(["h", &channels[0].to_string()]).unwrap(),
                    Tag::parse(["e", &original.id.to_hex()]).unwrap(),
                    Tag::parse(media[0].clone()).unwrap(),
                ])
                .custom_created_at(Timestamp::from(now - 1))
                .sign_with_keys(&owner)
                .unwrap();
            db.insert_event(community, &retained, Some(channels[0]))
                .await
                .unwrap();
        }
        if scenario == "deleted" {
            db.soft_delete_event_and_update_thread(community, empty.id.as_bytes(), None, None)
                .await
                .unwrap();
        }
        if scenario == "deleted-original" {
            db.soft_delete_event_and_update_thread(community, original.id.as_bytes(), None, None)
                .await
                .unwrap();
        }
        let access = db
            .media_access(
                community,
                &hash,
                "",
                "https://preflight.example/media",
                owner.public_key().as_bytes(),
                false,
                |_, _| true,
            )
            .await
            .unwrap();
        assert_eq!(
            access == super::media::MediaAccess::Allowed,
            !denied,
            "{scenario}"
        );
    }
    let counts: Vec<(i32, i64)> = sqlx::query_as(sql)
        .bind(community.as_uuid())
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(counts, vec![(45001, 2)]);
}
