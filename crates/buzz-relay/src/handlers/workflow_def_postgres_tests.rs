//! Postgres-backed tests for kind 30620 workflow definition saves: the
//! definition event, the executable workflow row and the retirement of other
//! stored `d` spellings commit together, or not at all, and the channel
//! trigger cache is invalidated only after that commit.

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
    definition_on(keys, channel_id, d_tag, name, "message_posted", created_at)
}

/// [`definition`] with the trigger `on`.
fn definition_on(
    keys: &Keys,
    channel_id: Uuid,
    d_tag: &str,
    name: &str,
    on: &str,
    created_at: u64,
) -> Event {
    EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF as u16),
        format!(
            "name: {name}\ntrigger:\n  on: {on}\nsteps:\n  - id: pause\n    \
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

/// Run the workflow engine's trigger lookup for a channel event of `kind`, as
/// the relay does after storing one, and return how many runs the workflow
/// then has. The engine creates a matching run before `on_event` returns.
async fn runs_after_channel_event(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    channel_id: Uuid,
    workflow_id: Uuid,
    kind: u32,
) -> usize {
    let event = EventBuilder::new(Kind::Custom(kind as u16), "+")
        .tags([Tag::parse(["h", &channel_id.to_string()]).expect("h tag")])
        .sign_with_keys(keys)
        .expect("sign channel event");
    state
        .workflow_engine
        .on_event(
            tenant.community(),
            &buzz_core::StoredEvent::new(event, Some(channel_id)),
        )
        .await
        .expect("trigger lookup");
    state
        .db
        .list_workflow_runs(tenant.community(), workflow_id, 100)
        .await
        .expect("list runs")
        .len()
}

/// A save invalidates the channel trigger cache after its transaction commits,
/// so an entry filled from the previously committed workflow while the save
/// was open does not outlive the save. The first definition fires on messages,
/// the second only on reactions. A deferred test trigger holds the second
/// save at its commit, after every statement has run. A message arriving then
/// reads the first definition and caches it. Once the save is accepted, the
/// next message must find no matching workflow and a reaction must start a
/// run. Before, the save invalidated the cache ahead of its commit, so the
/// entry cached while it was open kept the first definition firing on
/// messages for the cache TTL. Mutation: invalidate before `tx.commit()` → RED.
///
/// Every lookup here has returned before the save commits. A lookup whose
/// SELECT is still in flight when the save invalidates can cache its older
/// result afterwards; that case is bounded by the TTL and not tested here.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn save_invalidates_the_trigger_cache_after_its_commit() {
    let state = state().await;
    let tenant = community(&state, "workflow-save-cache").await;
    let owner = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let workflow_id = Uuid::new_v4();
    let canonical_d = workflow_id.to_string();
    let now = Timestamp::now().as_secs();
    let message = buzz_core::kind::KIND_STREAM_MESSAGE;
    let reaction = buzz_core::kind::KIND_REACTION;

    let first = definition_on(
        &owner,
        channel_id,
        &canonical_d,
        "first",
        "message_posted",
        now - 10,
    );
    assert_eq!(save(&state, &tenant, &owner, &first).await, Ok(true));

    // Hold the next save of this workflow at its commit: a deferred trigger,
    // scoped to this workflow row, waits for an advisory lock this test holds.
    let suffix = workflow_id.simple();
    let (gate_a, gate_b) = {
        let bytes = workflow_id.as_bytes();
        let half = |at: usize| {
            i32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) & i32::MAX
        };
        (half(0), half(4))
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION workflow_save_gate_{suffix}() RETURNS trigger LANGUAGE plpgsql AS          $$ BEGIN PERFORM pg_advisory_xact_lock({gate_a}, {gate_b}); RETURN NULL; END $$;          CREATE CONSTRAINT TRIGGER workflow_save_gate_{suffix} AFTER INSERT OR UPDATE          ON workflows DEFERRABLE INITIALLY DEFERRED FOR EACH ROW          WHEN (NEW.id = '{workflow_id}'::uuid)          EXECUTE FUNCTION workflow_save_gate_{suffix}()"
    )))
    .execute(state.db.pool())
    .await
    .expect("install commit gate");
    let mut gate = state.db.pool().acquire().await.expect("gate connection");
    sqlx::query("SELECT pg_advisory_lock($1, $2)")
        .bind(gate_a)
        .bind(gate_b)
        .execute(&mut *gate)
        .await
        .expect("close commit gate");

    let second = definition_on(
        &owner,
        channel_id,
        &canonical_d,
        "second",
        "reaction_added",
        now,
    );
    let saving = tokio::spawn({
        let (state, tenant, owner) = (Arc::clone(&state), tenant.clone(), owner.clone());
        async move { save(&state, &tenant, &owner, &second).await }
    });

    // The save is at its commit once its transaction waits on the gate.
    let mut at_commit = false;
    for _ in 0..3000 {
        at_commit = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory'              AND NOT granted AND classid::bigint = $1 AND objid::bigint = $2)",
        )
        .bind(i64::from(gate_a))
        .bind(i64::from(gate_b))
        .fetch_one(state.db.pool())
        .await
        .expect("read gate waiters");
        if at_commit || saving.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // While the save is open, a message reads the committed first definition
    // (and caches it): it starts the workflow's first run.
    let runs_while_open = if at_commit {
        Some(
            runs_after_channel_event(&state, &tenant, &owner, channel_id, workflow_id, message)
                .await,
        )
    } else {
        None
    };

    sqlx::query("SELECT pg_advisory_unlock($1, $2)")
        .bind(gate_a)
        .bind(gate_b)
        .execute(&mut *gate)
        .await
        .expect("open commit gate");
    drop(gate);
    let saved = saving.await.expect("save task");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER workflow_save_gate_{suffix} ON workflows;          DROP FUNCTION workflow_save_gate_{suffix}()"
    )))
    .execute(state.db.pool())
    .await
    .expect("remove commit gate");

    assert!(at_commit, "the save never reached its commit gate");
    assert_eq!(
        runs_while_open,
        Some(1),
        "a message during the open save must run the committed first definition"
    );
    assert_eq!(saved, Ok(true));

    assert_eq!(
        runs_after_channel_event(&state, &tenant, &owner, channel_id, workflow_id, message).await,
        1,
        "a message after the save ran the definition the save replaced"
    );
    assert_eq!(
        runs_after_channel_event(&state, &tenant, &owner, channel_id, workflow_id, reaction).await,
        2,
        "a reaction after the save must run the saved definition"
    );
}
