//! Live probe of channel parents against a running relay: live delivery,
//! the mention feed, push matching and revocation for a reader who can read
//! a private channel only through its parent.
//!
//! Needs a running relay (`RELAY_URL`) with `BUZZ_PUSH_ENABLED=true`, and its
//! database (`DATABASE_URL`) for push leases and the wake outbox.

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{EventBuilder, Filter, Keys, Kind, Tag};
use uuid::Uuid;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

async fn send(client: &mut BuzzTestClient, keys: &Keys, kind: u16, tags: &[&[&str]]) -> String {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|t| Tag::parse(t.iter().copied()).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .allow_self_tagging()
        .sign_with_keys(keys)
        .unwrap();
    let ok = client.send_event(event).await.expect("send");
    assert!(ok.accepted, "kind:{kind} refused: {}", ok.message);
    ok.message
}

async fn private_channel(client: &mut BuzzTestClient, keys: &Keys) -> String {
    let id = Uuid::new_v4().to_string();
    let name = format!("live-parent-{}", Uuid::new_v4().simple());
    send(
        client,
        keys,
        9007,
        &[
            &["h", &id],
            &["name", &name],
            &["channel_type", "stream"],
            &["visibility", "private"],
        ],
    )
    .await;
    id
}

/// Frames for `sub` within `wait`: events by id, and a CLOSED reason.
async fn drain(
    client: &mut BuzzTestClient,
    sub: &str,
    wait: Duration,
) -> (Vec<String>, Option<String>) {
    let deadline = tokio::time::Instant::now() + wait;
    let (mut events, mut closed) = (Vec::new(), None);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return (events, closed);
        }
        match client.recv_event(left).await {
            Ok(RelayMessage::Event {
                subscription_id,
                event,
            }) if subscription_id == sub => events.push(event.id.to_hex()),
            Ok(RelayMessage::Closed {
                subscription_id,
                message,
            }) if subscription_id == sub => closed = Some(message),
            Ok(_) => {}
            Err(_) => return (events, closed),
        }
    }
}

async fn insert_lease(pool: &sqlx::PgPool, community: Uuid, who: &Keys) {
    let me = who.public_key().to_hex();
    let subscriptions = serde_json::json!([{
        "filter": {"kinds": [9], "#p": [me]},
        "class": "default"
    }]);
    sqlx::query(
        "INSERT INTO push_leases (community_id, author, installation_id, source_event_id, \
         source_created_at, generation, active, endpoint_hash, endpoint_grant, max_class, \
         subscriptions, expires_at) \
         VALUES ($1, $2, 'probe', $3, 1, 1, true, $4, 'probe-grant', 'default', $5, 4102444800)",
    )
    .bind(community)
    .bind(who.public_key().to_bytes().to_vec())
    .bind(rand::random::<[u8; 32]>().to_vec())
    .bind(rand::random::<[u8; 32]>().to_vec())
    .bind(subscriptions)
    .execute(pool)
    .await
    .expect("insert lease");
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn live_parent_access_probe() {
    let url = relay_url();
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (owner, reader, member, outsider) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let mut o = BuzzTestClient::connect(&url, &owner).await.unwrap();
    let project = private_channel(&mut o, &owner).await;
    let child = private_channel(&mut o, &owner).await;
    send(
        &mut o,
        &owner,
        9002,
        &[&["h", &child], &["parent", &project]],
    )
    .await;
    let r_hex = reader.public_key().to_hex();
    let m_hex = member.public_key().to_hex();
    send(&mut o, &owner, 9000, &[&["h", &project], &["p", &r_hex]]).await;
    send(&mut o, &owner, 9000, &[&["h", &child], &["p", &m_hex]]).await;

    let community: Uuid =
        sqlx::query_scalar("SELECT community_id FROM channels WHERE id = $1::uuid")
            .bind(&child)
            .fetch_one(&pool)
            .await
            .unwrap();
    insert_lease(&pool, community, &reader).await;
    insert_lease(&pool, community, &member).await;

    let mut r = BuzzTestClient::connect(&url, &reader).await.unwrap();
    let mut r_feed = BuzzTestClient::connect(&url, &reader).await.unwrap();
    let mut x = BuzzTestClient::connect(&url, &outsider).await.unwrap();
    let now = nostr::Timestamp::now();
    let live = Filter::new()
        .kind(Kind::Custom(9))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            child.clone(),
        )
        .since(now);
    let mentions = Filter::new()
        .kind(Kind::Custom(9))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::P),
            r_hex.clone(),
        )
        .since(now);
    r.subscribe("child", vec![live.clone()]).await.unwrap();
    r_feed.subscribe("mentions", vec![mentions]).await.unwrap();
    let mut m_feed = BuzzTestClient::connect(&url, &member).await.unwrap();
    m_feed
        .subscribe(
            "mentions",
            vec![Filter::new()
                .kind(Kind::Custom(9))
                .custom_tag(
                    nostr::SingleLetterTag::lowercase(nostr::Alphabet::P),
                    m_hex.clone(),
                )
                .since(now)],
        )
        .await
        .unwrap();
    x.subscribe("child", vec![live]).await.unwrap();
    let _ = drain(&mut r, "none", Duration::from_millis(500)).await;
    let (_, x_closed) = drain(&mut x, "child", Duration::from_millis(500)).await;
    println!("outsider subscribe to child: CLOSED={x_closed:?}");

    let mention = EventBuilder::new(Kind::Custom(9), "hi @reader @member")
        .tags([
            Tag::parse(["h", &child]).unwrap(),
            Tag::parse(["p", &r_hex]).unwrap(),
            Tag::parse(["p", &m_hex]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let mention_id = mention.id.to_hex();
    assert!(o.send_event(mention).await.unwrap().accepted);

    let (got, closed) = drain(&mut r, "child", Duration::from_secs(2)).await;
    println!(
        "reader live on child sub: {} closed={closed:?}",
        got.contains(&mention_id)
    );
    r.subscribe(
        "feed",
        vec![Filter::new().kind(Kind::Custom(9)).custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::P),
            r_hex.clone(),
        )],
    )
    .await
    .unwrap();
    let (feed, _) = drain(&mut r, "feed", Duration::from_secs(1)).await;
    let (mentions_live, _) = drain(&mut r_feed, "mentions", Duration::from_secs(1)).await;
    let (member_live, _) = drain(&mut m_feed, "mentions", Duration::from_secs(1)).await;
    println!(
        "control: direct member live #p sub got it: {}",
        member_live.contains(&mention_id)
    );
    println!(
        "reader mention feed: historical={} live (#p sub, no channel named)={}",
        feed.contains(&mention_id),
        mentions_live.contains(&mention_id)
    );
    let (x_got, _) = drain(&mut x, "child", Duration::from_millis(500)).await;
    println!("outsider got it: {}", x_got.contains(&mention_id));

    // Push: give the matcher time, then read the wake outbox.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let wakes: Vec<(Vec<u8>,)> = sqlx::query_as(
        "SELECT author FROM push_wake_outbox WHERE community_id = $1 AND event_id = $2",
    )
    .bind(community)
    .bind(hex::decode(&mention_id).unwrap())
    .fetch_all(&pool)
    .await
    .unwrap();
    let woke = |k: &Keys| {
        wakes
            .iter()
            .any(|(a,)| a == &k.public_key().to_bytes().to_vec())
    };
    let queued: Option<String> = sqlx::query_scalar(
        "SELECT state FROM push_match_queue WHERE community_id = $1 AND event_id = $2",
    )
    .bind(community)
    .bind(hex::decode(&mention_id).unwrap())
    .fetch_optional(&pool)
    .await
    .unwrap();
    println!(
        "push wake queued: member={} reader={} (match queue row left: {queued:?})",
        woke(&member),
        woke(&reader)
    );

    // Revocation: removal from the project closes the child subscription.
    send(&mut o, &owner, 9001, &[&["h", &project], &["p", &r_hex]]).await;
    let (_, closed) = drain(&mut r, "child", Duration::from_secs(2)).await;
    println!("reader child sub after removal: CLOSED={closed:?}");
    let after = EventBuilder::new(Kind::Custom(9), "after removal")
        .tags([Tag::parse(["h", &child]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    let after_id = after.id.to_hex();
    assert!(o.send_event(after).await.unwrap().accepted);
    r.subscribe(
        "child2",
        vec![Filter::new().kind(Kind::Custom(9)).custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            child.clone(),
        )],
    )
    .await
    .unwrap();
    let (got, closed) = drain(&mut r, "child2", Duration::from_secs(1)).await;
    println!(
        "reader re-REQ after removal: sees post={} closed={closed:?}",
        got.contains(&after_id)
    );
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channel_members WHERE channel_id = $1::uuid AND pubkey = $2",
    )
    .bind(&child)
    .bind(reader.public_key().to_bytes().to_vec())
    .fetch_one(&pool)
    .await
    .unwrap();
    println!("reader member rows in child (any state): {rows}");
}
