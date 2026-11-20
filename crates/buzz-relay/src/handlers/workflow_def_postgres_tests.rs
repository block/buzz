//! Postgres-backed tests for kind 30620 workflow definition saves: the
//! definition event, the executable workflow row and the retirement of other
//! stored `d` spellings commit together, or not at all.

use std::sync::Arc;

use buzz_core::kind::KIND_WORKFLOW_DEF;
use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

/// A signed definition named `name`, stored under the `d` spelling `d_tag`.
fn definition(keys: &Keys, channel_id: Uuid, d_tag: &str, name: &str, created_at: u64) -> Event {
    EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF as u16),
        format!(
            "name: {name}\ntrigger:\n  on: message_posted\nsteps:\n  - id: pause\n    \
             action: delay\n    duration: 1s\n"
        ),
    )
    .tags([
        Tag::parse(["d", d_tag]).expect("d tag"),
        Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
    ])
    .custom_created_at(Timestamp::from(created_at))
    .sign_with_keys(keys)
    .expect("sign definition")
}

/// IDs of the owner's live definitions stored under exactly `d_tag`.
async fn live_definitions(
    state: &AppState,
    tenant: &TenantContext,
    keys: &Keys,
    d_tag: &str,
) -> Vec<Vec<u8>> {
    sqlx::query_scalar(
        "SELECT id FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
         AND d_tag = $4 AND deleted_at IS NULL",
    )
    .bind(tenant.community().as_uuid())
    .bind(KIND_WORKFLOW_DEF as i32)
    .bind(keys.public_key().to_bytes().as_slice())
    .bind(d_tag)
    .fetch_all(state.db.pool())
    .await
    .expect("read live definitions")
}

/// The three representations of one workflow that a save writes.
#[derive(Debug, PartialEq)]
struct Saved {
    /// Live definition events at the canonical coordinate.
    canonical: Vec<Vec<u8>>,
    /// Live definition events under the other stored spelling.
    alias: Vec<Vec<u8>>,
    /// The executable workflow row: name, definition, hash, `updated_at`.
    workflow: (
        String,
        serde_json::Value,
        Vec<u8>,
        chrono::DateTime<chrono::Utc>,
    ),
}

async fn saved(
    state: &AppState,
    tenant: &TenantContext,
    keys: &Keys,
    workflow_id: Uuid,
    alias_d: &str,
) -> Saved {
    let workflow = state
        .db
        .get_workflow(tenant.community(), workflow_id)
        .await
        .expect("read workflow row");
    Saved {
        canonical: live_definitions(state, tenant, keys, &workflow_id.to_string()).await,
        alias: live_definitions(state, tenant, keys, alias_d).await,
        workflow: (
            workflow.name,
            workflow.definition,
            workflow.definition_hash,
            workflow.updated_at,
        ),
    }
}

async fn save(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    event: &Event,
) -> Result<bool, String> {
    ingest_event(state, tenant, event.clone(), http_auth(keys))
        .await
        .map(|result| result.accepted)
        .map_err(|error| format!("{error:?}"))
}

/// A save that fails after the workflow upsert changes nothing: the definition
/// event, the executable workflow row and the stored alias spelling are all as
/// they were, and the same signed save applies once the failure is gone. A
/// test trigger makes the alias retirement, the step after the upsert, fail.
/// Before, the upsert committed in its own transaction, so the failed save
/// left the workflow running YAML that no definition showed. Mutation: upsert
/// on the pool instead of in the event transaction → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_save_changes_no_representation() {
    let state = state().await;
    let tenant = community(&state, "workflow-save-atomic").await;
    let owner = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let workflow_id = Uuid::new_v4();
    let canonical_d = workflow_id.to_string();
    let alias_d = canonical_d.to_uppercase();
    let now = Timestamp::now().as_secs();

    let first = definition(&owner, channel_id, &canonical_d, "first", now - 20);
    assert_eq!(save(&state, &tenant, &owner, &first).await, Ok(true));
    // An uppercase definition, stored as a relay before the canonical-`d` rule
    // stored it: the save handler now refuses this spelling.
    let alias = definition(&owner, channel_id, &alias_d, "alias", now - 10);
    state
        .db
        .insert_event(tenant.community(), &alias, Some(channel_id))
        .await
        .expect("store alias definition");

    let before = saved(&state, &tenant, &owner, workflow_id, &alias_d).await;
    assert_eq!(before.canonical, vec![first.id.as_bytes().to_vec()]);
    assert_eq!(before.alias, vec![alias.id.as_bytes().to_vec()]);
    assert_eq!(before.workflow.0, "first");

    // Scoped to this alias, so no other test's writes can trip it.
    let suffix = workflow_id.simple();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION workflow_save_fail_{suffix}() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'injected alias retirement failure'; END $$; \
         CREATE TRIGGER workflow_save_fail_{suffix} BEFORE UPDATE ON events FOR EACH ROW \
         WHEN (OLD.id = '\\x{}'::bytea AND OLD.deleted_at IS NULL \
         AND NEW.deleted_at IS NOT NULL) EXECUTE FUNCTION workflow_save_fail_{suffix}()",
        alias.id.to_hex()
    )))
    .execute(state.db.pool())
    .await
    .expect("install failure trigger");

    let second = definition(&owner, channel_id, &canonical_d, "second", now);
    let failed = save(&state, &tenant, &owner, &second).await;

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER workflow_save_fail_{suffix} ON events; \
         DROP FUNCTION workflow_save_fail_{suffix}()"
    )))
    .execute(state.db.pool())
    .await
    .expect("remove failure trigger");

    let error = failed.expect_err("a save whose alias retirement fails must not be accepted");
    assert!(error.contains("retire definitions"), "{error}");
    assert_eq!(
        saved(&state, &tenant, &owner, workflow_id, &alias_d).await,
        before,
        "a failed save changed the definition, the workflow row or the alias"
    );

    // The identical signed save applies once the failure is gone.
    assert_eq!(save(&state, &tenant, &owner, &second).await, Ok(true));
    let after = saved(&state, &tenant, &owner, workflow_id, &alias_d).await;
    assert_eq!(after.canonical, vec![second.id.as_bytes().to_vec()]);
    assert_eq!(after.alias, Vec::<Vec<u8>>::new());
    assert_eq!(after.workflow.0, "second");
    assert_ne!(after.workflow.2, before.workflow.2);
}
