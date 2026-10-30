//! `buzz-admin reconcile-channels --channel <id> --metadata` leaves the newest
//! relay-signed kind:39000 matching the channel row, also when an older
//! snapshot of the row holds the replacement with a newer timestamp.

use buzz_core::CommunityId;
use buzz_db::event::EventQuery;
use buzz_db::{Db, DbConfig};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use tokio::process::Command;
use uuid::Uuid;

fn admin_binary() -> std::ffi::OsString {
    // Nextest relocates binaries when extracting an archive. Cargo's build-time
    // path remains the fallback for local cargo test runs.
    std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into())
}

fn name_tag(event: &nostr::Event) -> Option<&str> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [key, value, ..] if key == "name" => Some(value.as_str()),
        _ => None,
    })
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn metadata_republish_replaces_a_stale_snapshot_that_holds_the_replacement() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("run through scripts/postgres-test-run.sh for an isolated database");
    let db = Db::new(&DbConfig {
        database_url: url.clone(),
        ..DbConfig::default()
    })
    .await
    .expect("connect to test DB");
    let community = Uuid::new_v4();
    let host = format!("reconcile-{}.example", community.simple());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(&host)
        .execute(db.pool())
        .await
        .expect("insert community");
    let community = CommunityId::from_uuid(community);
    let channel = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
         VALUES ($1, $2, 'square-ios-79f8e9d4', 'stream', 'open', $3)",
    )
    .bind(channel)
    .bind(community.as_uuid())
    .bind(Keys::generate().public_key().to_bytes().to_vec())
    .execute(db.pool())
    .await
    .expect("insert renamed channel");

    // The row before the repair renamed it, dated ahead of this command's
    // clock: a relay that read the old row published it, signing later than
    // the event it replaced. The command must still replace it.
    let relay = Keys::generate();
    let mut before_rename = db.get_channel(community, channel).await.unwrap();
    before_rename.name = "square-ios".to_owned();
    let stale = EventBuilder::new(Kind::Custom(39000), "")
        .tags(
            buzz_db::channel::group_metadata_tags(&before_rename, &[])
                .into_iter()
                .map(Tag::parse)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        )
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 60))
        .sign_with_keys(&relay)
        .unwrap();
    let (_, inserted) = db
        .replace_addressable_event(community, &stale, Some(channel))
        .await
        .unwrap();
    assert!(inserted);

    let output = Command::new(admin_binary())
        .args(["reconcile-channels", "--channel", &channel.to_string()])
        .args([
            "--metadata",
            "--relay-key",
            &relay.secret_key().to_secret_hex(),
        ])
        .env("DATABASE_URL", &url)
        .env("RELAY_URL", format!("wss://{host}"))
        .output()
        .await
        .expect("run buzz-admin");
    assert!(
        output.status.success(),
        "reconcile-channels failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let newest = db
        .query_events(&EventQuery {
            kinds: Some(vec![39000]),
            d_tag: Some(channel.to_string()),
            limit: Some(1),
            ..EventQuery::for_community(community)
        })
        .await
        .unwrap();
    let newest = &newest.first().expect("kind:39000 stored").event;
    assert_eq!(name_tag(newest), Some("square-ios-79f8e9d4"));
    assert!(newest.created_at > stale.created_at);
}
