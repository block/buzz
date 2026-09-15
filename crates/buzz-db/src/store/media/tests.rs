use super::*;
use crate::channel::{ChannelType, ChannelVisibility, MemberRole};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::json;
use uuid::Uuid;

async fn fixture() -> (Db, CommunityId, String, Keys, Keys, Uuid) {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("test database");
    crate::migration::run_migrations(&pool)
        .await
        .expect("migrate");
    let db = Db::from_pool(pool);
    let host = format!("media-{}.example", Uuid::new_v4());
    let community = db
        .ensure_configured_community(&host)
        .await
        .expect("community")
        .id;
    let owner = Keys::generate();
    let member = Keys::generate();
    for keys in [&owner, &member] {
        db.ensure_user(community, keys.public_key().as_bytes())
            .await
            .expect("user");
    }
    let channel = db
        .create_channel(
            community,
            "private",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .expect("channel")
        .id;
    db.add_member(
        community,
        channel,
        member.public_key().as_bytes(),
        MemberRole::Member,
        Some(owner.public_key().as_bytes()),
    )
    .await
    .expect("member");
    (db, community, host, owner, member, channel)
}

async fn publish(
    db: &Db,
    community: CommunityId,
    owner: &Keys,
    channel: Option<Uuid>,
    kind: u16,
    content: &str,
    tags: Vec<Tag>,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .allow_self_tagging()
        .sign_with_keys(owner)
        .expect("event");
    db.insert_event(community, &event, channel)
        .await
        .expect("insert event");
    event
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn existing_attachments_follow_live_membership_and_tenant() {
    let (db, community, host, owner, member, channel) = fixture().await;
    let sha = "a".repeat(64);
    // No uploader row: this also covers attachments published before upgrade.
    publish(
        &db,
        community,
        &owner,
        Some(channel),
        9,
        &format!("![attachment](https://{host}/media/{sha}.jpg)"),
        vec![],
    )
    .await;
    let pk = member.public_key().to_bytes();
    assert!(db
        .can_read_media(community, &sha, &pk)
        .await
        .expect("member read"));
    assert!(!db
        .can_read_media(community, &sha, Keys::generate().public_key().as_bytes())
        .await
        .expect("stranger read"));
    db.remove_member(community, channel, &pk, owner.public_key().as_bytes())
        .await
        .expect("remove");
    assert!(!db
        .can_read_media(community, &sha, &pk)
        .await
        .expect("removed read"));
    assert!(!db
        .can_reference_media(community, &sha, &pk)
        .await
        .expect("removed publication"));
    assert!(db
        .can_read_media(community, &sha, owner.public_key().as_bytes())
        .await
        .expect("owner read"));
    let (other_db, other, other_host, other_owner, _, other_channel) = fixture().await;
    publish(
        &other_db,
        other,
        &other_owner,
        Some(other_channel),
        9,
        &format!("https://{other_host}/media/{sha}.jpg"),
        vec![],
    )
    .await;
    assert!(!db
        .can_read_media(other, &sha, owner.public_key().as_bytes())
        .await
        .expect("other tenant"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn published_attachments_follow_references_without_changing_unposted_uploads() {
    let (db, community, host, owner, member, channel) = fixture().await;
    let sha = "b".repeat(64);
    let pk = member.public_key().to_bytes();
    db.record_media_upload(community, &sha, &pk)
        .await
        .expect("upload");
    assert!(db
        .can_read_media(community, &sha, &pk)
        .await
        .expect("draft"));
    assert!(db
        .can_read_media(community, &sha, owner.public_key().as_bytes())
        .await
        .expect("unposted media retains community access"));
    let url = format!("https://{host}/media/{sha}.txt");
    publish(&db, community, &member, Some(channel), 9, &url, vec![]).await;
    db.remove_member(community, channel, &pk, owner.public_key().as_bytes())
        .await
        .expect("remove uploader");
    assert!(!db
        .can_read_media(community, &sha, &pk)
        .await
        .expect("removed uploader"));
    // Supplying the bytes permits a new publication, even after leaving a channel.
    assert!(db
        .can_reference_media(community, &sha, &pk)
        .await
        .expect("possessor publication"));
    let open = db
        .create_channel(
            community,
            "open",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .expect("open channel")
        .id;
    publish(&db, community, &owner, Some(open), 9, &url, vec![]).await;
    assert!(db
        .can_read_media(community, &sha, &pk)
        .await
        .expect("shared in open channel"));
    db.soft_delete_channel(community, open)
        .await
        .expect("delete shared channel");
    assert!(!db
        .can_read_media(community, &sha, &pk)
        .await
        .expect("deleted channel"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn extractor_matches_index_and_rejects_foreign_hosts() {
    let (db, community, host, owner, _, channel) = fixture().await;
    let sha = "c".repeat(64);
    let url = format!("https://{host}/media/{sha}.thumb.jpg");
    for content in [
        url.clone(),
        format!("![image]({url})"),
        json!({"picture":url}).to_string(),
        url.replace('/', "\\/"),
    ] {
        assert_eq!(
            db.local_media_hashes(&content, &json!([]), &host)
                .await
                .expect("extract"),
            vec![sha.clone()]
        );
    }
    let tags = json!([["imeta", format!("url {url}"), format!("x {sha}")]]);
    assert_eq!(
        db.local_media_hashes("", &tags, &host).await.expect("tags"),
        vec![sha.clone()]
    );
    for content in [
        format!("https://foreign.example/media/{sha}.jpg"),
        format!("https://{host}.evil/media/{sha}.jpg"),
        format!("/media/{sha}a.jpg"),
    ] {
        assert!(db
            .local_media_hashes(&content, &json!([]), &host)
            .await
            .expect("foreign")
            .is_empty());
    }
    let event = publish(&db, community, &owner, Some(channel), 9, &url, vec![]).await;
    let indexed: Vec<String> = sqlx::query_scalar(
        "SELECT buzz_media_hashes(content, tags) FROM events WHERE community_id = $1 AND id = $2",
    )
    .bind(community.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .fetch_one(&db.pool)
    .await
    .expect("indexed extractor");
    assert_eq!(indexed, vec![sha]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn private_event_kinds_do_not_grant_community_wide_media_access() {
    let (db, community, host, owner, member, _) = fixture().await;
    let mut cases = vec![
        (0_u16, vec![], true),
        (KIND_AGENT_ENGRAM as u16, vec![], false),
    ];
    cases.extend(AUTHOR_ONLY_KINDS.iter().map(|k| (*k as u16, vec![], false)));
    cases.push((
        buzz_core::kind::KIND_EVENT_REMINDER as u16,
        vec![Tag::parse([
            "not_before",
            &(nostr::Timestamp::now().as_secs() + 300).to_string(),
        ])
        .expect("schedule")],
        false,
    ));
    cases.extend(
        P_GATED_KINDS
            .iter()
            .filter(|k| !buzz_core::kind::is_ephemeral(**k))
            .map(|k| {
                (
                    *k as u16,
                    vec![Tag::parse(["p", &owner.public_key().to_hex()]).expect("p")],
                    false,
                )
            }),
    );
    for kind in SHARED_GATED_KINDS {
        cases.push((*kind as u16, vec![], false));
        cases.push((
            *kind as u16,
            vec![Tag::parse(["shared", "true"]).expect("shared")],
            true,
        ));
        cases.push((
            *kind as u16,
            vec![Tag::parse(["shared", "true", "extra"]).expect("malformed shared")],
            false,
        ));
    }
    for (i, (kind, tags, allowed)) in cases.into_iter().enumerate() {
        let sha = format!("{i:064x}");
        publish(
            &db,
            community,
            &owner,
            None,
            kind,
            &format!("https://{host}/media/{sha}.png"),
            tags,
        )
        .await;
        assert!(db
            .can_read_media(community, &sha, owner.public_key().as_bytes())
            .await
            .expect("author read"));
        assert_eq!(
            db.can_read_media(community, &sha, member.public_key().as_bytes())
                .await
                .expect("kind read"),
            allowed,
            "kind {kind}, case {i}"
        );
    }
}
