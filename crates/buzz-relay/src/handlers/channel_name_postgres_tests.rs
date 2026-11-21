//! Channel name validation at the signed command ingestion boundary.

use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::group_state_postgres_tests::{community, create_channel, http_auth, state};
use super::ingest::{ingest_event, IngestError};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invisible_names_are_refused_before_create_or_rename_is_stored() {
    let state = state().await;
    let tenant = community(&state, "visible-channel-names").await;
    let owner = Keys::generate();
    let existing = create_channel(&state, &tenant, &owner, &[]).await;
    let before = state
        .db
        .get_channel(tenant.community(), existing)
        .await
        .unwrap();

    for name in ["\u{200b}", "\u{200c}\u{200d}", " \u{fe0f}\u{feff} "] {
        for kind in [9007, 9002] {
            let id = if kind == 9007 {
                Uuid::new_v4()
            } else {
                existing
            };
            let event = EventBuilder::new(Kind::Custom(kind), "")
                .tags([
                    Tag::parse(["h", &id.to_string()]).unwrap(),
                    Tag::parse(["name", name]).unwrap(),
                    Tag::parse(["about", "must not be written"]).unwrap(),
                ])
                .sign_with_keys(&owner)
                .unwrap();
            let event_id = event.id;
            match ingest_event(&state, &tenant, event, http_auth(&owner)).await {
                Err(IngestError::Rejected(message)) => {
                    assert!(message.contains("visible character"), "{message}")
                }
                Err(other) => panic!("unexpected error: {other:?}"),
                Ok(result) => panic!("accepted invisible name: {:?}", result.message),
            }
            assert!(state
                .db
                .get_event_by_id(tenant.community(), event_id.as_bytes())
                .await
                .unwrap()
                .is_none());
            if kind == 9007 {
                let count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM channels WHERE community_id = $1 AND id = $2",
                )
                .bind(tenant.community().as_uuid())
                .bind(id)
                .fetch_one(state.db.pool())
                .await
                .unwrap();
                assert_eq!(count, 0);
            } else {
                let after = state
                    .db
                    .get_channel(tenant.community(), existing)
                    .await
                    .unwrap();
                assert_eq!(after.name, before.name);
                assert_eq!(after.description, before.description);
            }
        }
    }

    // Both writes preserve valid joiners and selectors exactly.
    for kind in [9007, 9002] {
        let id = if kind == 9007 {
            Uuid::new_v4()
        } else {
            existing
        };
        let name = if kind == 9007 {
            "👩‍💻"
        } else {
            "می‌روم"
        };
        let event = EventBuilder::new(Kind::Custom(kind), "")
            .tags([
                Tag::parse(["h", &id.to_string()]).unwrap(),
                Tag::parse(["name", name]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        let result = ingest_event(&state, &tenant, event, http_auth(&owner))
            .await
            .unwrap();
        assert!(result.accepted);
        assert_eq!(
            state
                .db
                .get_channel(tenant.community(), id)
                .await
                .unwrap()
                .name,
            name
        );
    }
}
