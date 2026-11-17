//! A removal's receipt, membership write and publication must agree.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use buzz_db::channel::MemberRole;
use nostr::{EventBuilder, Keys, Kind, Tag};

use super::group_state_postgres_tests::{community, create_channel, http_auth};
use super::ingest::{ingest_event, IngestError};
use crate::nip_fi_test_hooks::event_publish_counter;

/// Hold the membership lock until ingest has passed admission, then promote
/// its target. Refusal must roll back the command and never reach dispatch.
/// The same command can subsequently remove a member, but its replay cannot
/// remove someone who rejoined. Restoring the old post-insert path fails here.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn removal_receipt_tracks_the_locked_membership_write() {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let tenant = community(&state, "remove-user-locked-refusal").await;
    let (owner, admin, target) = (Keys::generate(), Keys::generate(), Keys::generate());
    let owner_key = owner.public_key().to_bytes();
    let admin_key = admin.public_key().to_bytes();
    let target_key = target.public_key().to_bytes();
    let channel = create_channel(
        &state,
        &tenant,
        &owner,
        &[
            &["visibility", "private"],
            &["p", &admin.public_key().to_hex()],
            &["p", &target.public_key().to_hex()],
        ],
    )
    .await;
    let community = tenant.community();
    state
        .db
        .add_member(
            community,
            channel,
            &admin_key,
            MemberRole::Admin,
            Some(&owner_key),
        )
        .await
        .expect("promote admin");
    let removal = EventBuilder::new(Kind::Custom(9001), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["p", &target.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&admin)
        .expect("sign removal");
    let published = event_publish_counter::register(community);
    let lock_key = format!("buzz_channel_membership:{}:{channel}", community.as_uuid());
    let pool = state.db.pool();
    let mut holder = pool.begin().await.expect("begin promotion");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *holder)
        .await
        .expect("hold membership lock");
    let task = tokio::spawn({
        let state = Arc::clone(&state);
        let tenant = tenant.clone();
        let event = removal.clone();
        let auth = http_auth(&admin);
        async move { ingest_event(&state, &tenant, event, auth).await }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
                 AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                 AND ((classid::bigint << 32) | objid::bigint) = hashtextextended($1, 0))",
            )
            .bind(&lock_key)
            .fetch_one(pool)
            .await
            .expect("observe removal waiting after admission");
            if waiting {
                break;
            }
            assert!(
                !task.is_finished(),
                "removal never waited on the membership lock"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("removal reaches membership lock");
    sqlx::query("UPDATE channel_members SET role = 'owner' WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3")
        .bind(community.as_uuid())
        .bind(channel)
        .bind(target_key.as_slice())
        .execute(&mut *holder)
        .await
        .expect("promote target after admission");
    holder.commit().await.expect("commit promotion");
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("removal returns")
        .expect("removal task");
    match result {
        Err(IngestError::Rejected(reason)) => {
            assert_eq!(reason, "invalid: only owners may remove another owner")
        }
        Err(error) => panic!("expected locked refusal, got {error:?}"),
        Ok(result) => panic!(
            "locked refusal reported accepted:{} {}",
            result.accepted, result.message
        ),
    }
    assert_eq!(
        state
            .db
            .get_member_role(community, channel, &target_key)
            .await
            .unwrap()
            .as_deref(),
        Some("owner")
    );
    assert!(state
        .db
        .get_event_by_id(community, &removal.id.to_bytes())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        published.load(Ordering::Relaxed),
        0,
        "refusal must never reach publication"
    );

    // A refused command was not stored: it can succeed once authorized.
    state
        .db
        .add_member(
            community,
            channel,
            &target_key,
            MemberRole::Member,
            Some(&owner_key),
        )
        .await
        .unwrap();
    let accepted = ingest_event(&state, &tenant, removal.clone(), http_auth(&admin))
        .await
        .unwrap();
    assert!(accepted.accepted);
    assert!(state
        .db
        .get_member_role(community, channel, &target_key)
        .await
        .unwrap()
        .is_none());
    assert!(state
        .db
        .get_event_by_id(community, &removal.id.to_bytes())
        .await
        .unwrap()
        .is_some());
    let after_removal = published.load(Ordering::Relaxed);
    assert!(
        after_removal > 0,
        "a committed removal must reach publication"
    );

    // A recorded command remains complete even after its target rejoins.
    state
        .db
        .add_member(
            community,
            channel,
            &target_key,
            MemberRole::Member,
            Some(&owner_key),
        )
        .await
        .unwrap();
    let duplicate = ingest_event(&state, &tenant, removal, http_auth(&admin))
        .await
        .unwrap();
    assert!(duplicate.accepted);
    assert_eq!(duplicate.message, "duplicate:");
    assert_eq!(
        state
            .db
            .get_member_role(community, channel, &target_key)
            .await
            .unwrap()
            .as_deref(),
        Some("member")
    );
    assert_eq!(
        published.load(Ordering::Relaxed),
        after_removal,
        "duplicate must not publish again"
    );
    event_publish_counter::deregister(community);
}
