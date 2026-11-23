//! Postgres-backed tests for kind 5 / 9005 deletions of `e` targets: the
//! deletion and its target's soft-delete commit together, or not at all.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

/// Ingest a kind 9 message from `keys` in `channel_id`.
async fn post_message(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    channel_id: Uuid,
) -> Event {
    let event = EventBuilder::new(Kind::Custom(9), "delete me")
        .tags([Tag::parse(["h", &channel_id.to_string()]).unwrap()])
        .sign_with_keys(keys)
        .expect("sign message");
    let result = ingest_event(state, tenant, event.clone(), http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("post message: {error:?}"));
    assert!(result.accepted, "{:?}", result.message);
    event
}

/// A deletion of `target` of `kind` (5 or 9005), signed by `keys`.
fn deletion(kind: u16, keys: &Keys, channel_id: Uuid, target: &Event) -> Event {
    let mut tags = vec![Tag::parse(["e", &target.id.to_hex()]).unwrap()];
    if kind == 9005 {
        tags.push(Tag::parse(["h", &channel_id.to_string()]).unwrap());
    }
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign deletion")
}

/// `(stored, soft-deleted)` for event `id`.
async fn row_state(state: &AppState, tenant: &TenantContext, id: &Event) -> (bool, bool) {
    let deleted_at: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT deleted_at FROM events WHERE community_id = $1 AND id = $2")
            .bind(tenant.community().as_uuid())
            .bind(id.id.as_bytes().as_slice())
            .fetch_optional(state.db.pool())
            .await
            .expect("read event row");
    match deleted_at {
        None => (false, false),
        Some(deleted_at) => (true, deleted_at.is_some()),
    }
}

/// A target soft-delete that fails inside the write stores nothing: the
/// deletion is refused, the target stays live, and the same signed deletion
/// applies once the failure is gone. A test trigger makes the target's
/// soft-delete fail. Before, the relay stored the deletion, logged the failed
/// side effect, reported success, and answered a resend with `duplicate:`,
/// leaving the target live for good. Mutation: route the deletion through the
/// generic insert → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_target_soft_delete_stores_nothing() {
    for kind in [5u16, 9005] {
        let state = state().await;
        let tenant = community(&state, &format!("deletion-atomic-fail-{kind}")).await;
        let owner = Keys::generate();
        let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
        let target = post_message(&state, &tenant, &owner, channel_id).await;
        let deletion = deletion(kind, &owner, channel_id, &target);

        // Scoped to this target, so no other test's writes can trip it.
        for statement in [
            "CREATE OR REPLACE FUNCTION deletion_atomic_fail() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected soft-delete failure'; END $$"
                .to_owned(),
            format!(
                "CREATE TRIGGER deletion_atomic_fail BEFORE UPDATE ON events FOR EACH ROW \
                 WHEN (OLD.id = '\\x{}'::bytea AND OLD.deleted_at IS NULL \
                 AND NEW.deleted_at IS NOT NULL) EXECUTE FUNCTION deletion_atomic_fail()",
                target.id.to_hex()
            ),
        ] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
                .execute(state.db.pool())
                .await
                .expect("install failure trigger");
        }

        for attempt in ["first send", "resend"] {
            match ingest_event(&state, &tenant, deletion.clone(), http_auth(&owner)).await {
                Ok(result) => assert!(
                    !result.accepted,
                    "kind {kind} {attempt}: a failed deletion was reported as accepted: {:?}",
                    result.message
                ),
                Err(error) => assert!(
                    format!("{error:?}").contains("deletion failed"),
                    "kind {kind} {attempt}: {error:?}"
                ),
            }
            assert_eq!(
                row_state(&state, &tenant, &deletion).await,
                (false, false),
                "kind {kind} {attempt}: the deletion is not stored"
            );
            assert_eq!(
                row_state(&state, &tenant, &target).await,
                (true, false),
                "kind {kind} {attempt}: the target stays live"
            );
        }

        sqlx::query("DROP TRIGGER deletion_atomic_fail ON events")
            .execute(state.db.pool())
            .await
            .expect("drop failure trigger");
        let retry = ingest_event(&state, &tenant, deletion.clone(), http_auth(&owner))
            .await
            .unwrap_or_else(|error| panic!("kind {kind} retry: {error:?}"));
        assert!(retry.accepted, "kind {kind}: {:?}", retry.message);
        assert!(
            !retry.message.starts_with("duplicate:"),
            "kind {kind}: {:?}",
            retry.message
        );
        assert_eq!(row_state(&state, &tenant, &deletion).await, (true, false));
        assert_eq!(
            row_state(&state, &tenant, &target).await,
            (true, true),
            "kind {kind}: the target is soft-deleted with the deletion"
        );
    }
}

/// A resend of a deletion stored without its target's soft-delete (a row an
/// earlier, non-atomic write left torn) applies it and is not answered as a
/// bare duplicate; a further resend changes nothing. Only a resend inside the
/// ingest timestamp window gets here; older torn rows rely on the read path
/// (`torn_deletions_still_cancel_live_targets_in_both_windows`). Mutation: skip the
/// target soft-delete for a duplicate deletion → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn resent_deletion_repairs_a_live_target() {
    for kind in [5u16, 9005] {
        let state = state().await;
        let tenant = community(&state, &format!("deletion-repair-{kind}")).await;
        let owner = Keys::generate();
        let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
        let target = post_message(&state, &tenant, &owner, channel_id).await;
        let deletion = deletion(kind, &owner, channel_id, &target);
        state
            .db
            .insert_event(tenant.community(), &deletion, Some(channel_id))
            .await
            .expect("store a torn deletion");
        assert_eq!(row_state(&state, &tenant, &target).await, (true, false));

        let repair = ingest_event(&state, &tenant, deletion.clone(), http_auth(&owner))
            .await
            .unwrap_or_else(|error| panic!("kind {kind} repair: {error:?}"));
        assert!(repair.accepted, "kind {kind}: {:?}", repair.message);
        assert!(
            !repair.message.starts_with("duplicate:"),
            "kind {kind}: a repairing resend reaches dispatch: {:?}",
            repair.message
        );
        assert_eq!(
            row_state(&state, &tenant, &target).await,
            (true, true),
            "kind {kind}: the resend soft-deletes the live target"
        );

        // Once applied, a kind 5 resend is a duplicate. Kind 9005 validation
        // reads a tombstoned target as missing, so its resend is refused
        // before storage, as it was before deletions became atomic.
        let again = ingest_event(&state, &tenant, deletion.clone(), http_auth(&owner)).await;
        match (kind, again) {
            (5, Ok(result)) => assert!(
                result.accepted && result.message.starts_with("duplicate:"),
                "an applied kind 5 deletion is a duplicate: {:?}",
                result.message
            ),
            (9005, Err(error)) => assert!(
                format!("{error:?}").contains("deletion target not found"),
                "{error:?}"
            ),
            (kind, Ok(result)) => panic!("kind {kind} resend: {:?}", result.message),
            (kind, Err(error)) => panic!("kind {kind} resend: {error:?}"),
        }
        assert_eq!(row_state(&state, &tenant, &deletion).await, (true, false));
        assert_eq!(row_state(&state, &tenant, &target).await, (true, true));
    }
}
