use super::*;
use chrono::Duration;

const ISSUER: &str = "https://issuer.test";

async fn fixture() -> (Db, CommunityId, Keys, String) {
    let db = Db::new(&crate::DbConfig {
        database_url: crate::test_support::database_url(),
        ..crate::DbConfig::default()
    })
    .await
    .unwrap();
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        db.migrate().await.unwrap();
    }
    let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("profile-{}.test", community.as_uuid()))
        .execute(&db.pool)
        .await
        .unwrap();
    let key = Keys::generate().public_key().to_hex();
    set_member(&db, community, &key, true).await;
    (db, community, Keys::generate(), key)
}

async fn set_member(db: &Db, community: CommunityId, key: &str, member: bool) {
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    let sql = if member {
        "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')"
    } else {
        "DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2"
    };
    sqlx::query(sql)
        .bind(community.as_uuid())
        .bind(key)
        .execute(tx.conn())
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

fn input<'a>(
    community: CommunityId,
    key: &'a str,
    issued_at: DateTime<Utc>,
    admitted_at: DateTime<Utc>,
    full_name: Option<&'a str>,
) -> ProfileAdmission<'a> {
    ProfileAdmission {
        community,
        issuer: ISSUER,
        pubkey: key,
        issued_at,
        admitted_at,
        display_name: "Alice",
        username: "alice",
        full_name,
        label: "Example",
        refresh_seconds: 3600,
        membership_required: true,
    }
}

async fn current(db: &Db, community: CommunityId, key: &str) -> Option<Event> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT event FROM nip_fi_profile_heads WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(key)
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    value.map(|v| Event::from_json(v.to_string()).unwrap())
}

async fn lifecycle() {
    let (db, community, keys, key) = fixture().await;
    let start = Utc::now() - Duration::hours(3);
    let iat = start - Duration::seconds(10);
    let publish = |input| db.publish_verified_profile(input, &keys);

    let first = publish(input(community, &key, iat, start, Some("Alice Example")))
        .await
        .unwrap()
        .expect("first admission publishes");
    assert_eq!(current(&db, community, &key).await.unwrap().id, first.id);
    assert_eq!(first.pubkey, keys.public_key());
    assert_eq!(first.kind, Kind::Custom(30382));
    assert_eq!(first.tags.len(), 8);
    assert_eq!(tag(&first, "d"), Some(key.as_str()));
    assert_eq!(tag(&first, "active"), Some("true"));
    assert_eq!(tag(&first, "display_name"), Some("Alice"));
    assert_eq!(tag(&first, "full_name"), Some("Alice Example"));
    assert_eq!(tag(&first, "issuer_label"), Some("Example"));
    assert_eq!(verified_at(&first), Some(start.timestamp()));
    assert!(first.verify().is_ok());

    let step = |n: i64| start + Duration::seconds(n);
    assert!(
        publish(input(
            community,
            &key,
            iat + Duration::nanoseconds(2),
            step(1),
            Some("Alice Example"),
        ))
        .await
        .unwrap()
        .is_none(),
        "unchanged admission does not refresh early"
    );

    let renamed = publish(input(
        community,
        &key,
        iat + Duration::nanoseconds(3),
        step(2),
        Some("Alice Renamed"),
    ))
    .await
    .unwrap()
    .expect("full-name changes publish immediately");
    assert_eq!(tag(&renamed, "full_name"), Some("Alice Renamed"));
    assert!(renamed.created_at > first.created_at);

    let without_full_name = publish(input(
        community,
        &key,
        iat + Duration::nanoseconds(4),
        step(3),
        None,
    ))
    .await
    .unwrap()
    .expect("dropping the full name publishes");
    assert_eq!(tag(&without_full_name, "full_name"), None);
    assert_eq!(without_full_name.tags.len(), 7);

    // An assertion older than the last accepted one cannot overwrite.
    assert!(
        publish(input(
            community,
            &key,
            iat + Duration::nanoseconds(1),
            step(4),
            Some("Old"),
        ))
        .await
        .unwrap()
        .is_none(),
        "older fractional iat cannot overwrite"
    );
    assert!(
        publish(input(
            community,
            &key,
            iat + Duration::nanoseconds(5),
            step(2),
            Some("Delayed"),
        ))
        .await
        .unwrap()
        .is_none(),
        "public verified_at cannot move backwards"
    );
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        without_full_name.id
    );

    let refreshed = publish(input(
        community,
        &key,
        iat + Duration::nanoseconds(6),
        step(3 + 3600),
        None,
    ))
    .await
    .unwrap()
    .expect("unchanged names refresh after the interval");
    assert_eq!(verified_at(&refreshed), Some(step(3 + 3600).timestamp()));
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        refreshed.id
    );

    // Each public field change publishes immediately.
    let alicia = |ns: i64, n: i64| ProfileAdmission {
        display_name: "Alicia",
        username: "alicia",
        label: "Example Corp",
        ..input(
            community,
            &key,
            iat + Duration::nanoseconds(ns),
            step(n),
            None,
        )
    };
    let mut last = refreshed;
    for (field, admission) in [
        (
            "display_name",
            ProfileAdmission {
                username: "alice",
                label: "Example",
                ..alicia(7, 3604)
            },
        ),
        (
            "username",
            ProfileAdmission {
                label: "Example",
                ..alicia(8, 3605)
            },
        ),
        ("issuer_label", alicia(9, 3606)),
    ] {
        let event = publish(admission)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{field} change publishes"));
        assert_ne!(tag(&event, field), tag(&last, field));
        last = event;
    }

    // A no-op login still advances the issuer's assertion ordering.
    assert!(publish(alicia(20, 3607)).await.unwrap().is_none());
    assert!(
        publish(ProfileAdmission {
            display_name: "Stale",
            ..alicia(15, 3608)
        })
        .await
        .unwrap()
        .is_none(),
        "an assertion older than an unchanged one cannot overwrite"
    );

    // The latest verification from any issuer wins; assertion times are not
    // compared across issuers.
    let other = |n: i64| ProfileAdmission {
        issuer: "https://other-issuer.test",
        display_name: "Other",
        ..input(community, &key, iat - Duration::hours(1), step(n), None)
    };
    assert!(
        publish(other(3605)).await.unwrap().is_none(),
        "an earlier verification from another issuer cannot replace"
    );
    let taken = publish(other(3609))
        .await
        .unwrap()
        .expect("a newer verification from another issuer replaces");
    assert_eq!(tag(&taken, "display_name"), Some("Other"));
    let back = publish(alicia(21, 3610))
        .await
        .unwrap()
        .expect("the original issuer takes the profile back on a newer verification");
    assert_eq!(tag(&back, "display_name"), Some("Alicia"));
    assert_eq!(current(&db, community, &key).await.unwrap().id, back.id);
    let live: Vec<String> = sqlx::query_scalar(
        "SELECT encode(id, 'hex') FROM events WHERE community_id = $1 AND kind = 30382 AND d_tag = $2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(&key)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(live, vec![back.id.to_hex()], "one live replaceable event");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn profile_lifecycle_publishes_updates_and_orders() {
    lifecycle().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_profile_lifecycle_publishes_updates_and_orders() {
    lifecycle().await;
}

async fn eligibility() {
    let (db, community, keys, key) = fixture().await;
    set_member(&db, community, &key, false).await;
    let iat = Utc::now() - Duration::seconds(60);
    let at = |n: i64, membership_required| ProfileAdmission {
        membership_required,
        ..input(community, &key, iat, iat + Duration::seconds(n), None)
    };
    assert!(
        db.publish_verified_profile(at(1, true), &keys)
            .await
            .unwrap()
            .is_none(),
        "members-required relays need an explicit member row"
    );
    assert!(current(&db, community, &key).await.is_none());
    assert!(
        db.publish_verified_profile(at(2, false), &keys)
            .await
            .unwrap()
            .is_some(),
        "open relays publish for enforced admissions without a member row"
    );

    // A banned key never publishes, and its existing profile stays as a
    // historical record.
    let published = current(&db, community, &key).await.unwrap();
    let bytes = hex::decode(&key).unwrap();
    crate::moderation::ban_member(&db.pool, community, &bytes, &bytes, None, None)
        .await
        .unwrap();
    set_member(&db, community, &key, true).await;
    for membership_required in [true, false] {
        assert!(
            db.publish_verified_profile(
                ProfileAdmission {
                    display_name: "Banned Alice",
                    ..at(3, membership_required)
                },
                &keys,
            )
            .await
            .unwrap()
            .is_none(),
            "banned keys do not publish"
        );
    }
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        published.id
    );
    crate::moderation::unban_member(&db.pool, community, &bytes, &bytes)
        .await
        .unwrap();
    assert!(db
        .publish_verified_profile(
            ProfileAdmission {
                display_name: "Restored Alice",
                ..at(4, true)
            },
            &keys,
        )
        .await
        .unwrap()
        .is_some());
    crate::moderation::ban_member(
        &db.pool,
        community,
        &bytes,
        &bytes,
        None,
        Some(Utc::now() - Duration::minutes(1)),
    )
    .await
    .unwrap();
    assert!(
        db.publish_verified_profile(
            ProfileAdmission {
                display_name: "Expired-ban Alice",
                ..at(5, true)
            },
            &keys,
        )
        .await
        .unwrap()
        .is_some(),
        "an expired ban does not block"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn profiles_publish_only_for_eligible_keys() {
    eligibility().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_profiles_publish_only_for_eligible_keys() {
    eligibility().await;
}
