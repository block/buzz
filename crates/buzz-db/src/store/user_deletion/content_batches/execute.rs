//! Execution of an internally generated plan, never an imported JSON manifest.
use super::*;
use crate::entity_deletion::EntityLease;

impl UserDeletionStore {
    /// Shared entry point for direct commands and workflow content cleanup.
    ///
    /// Read-only mode prints the plan. Delete mode commits bounded batches;
    /// when a workflow lease and stage ID are supplied, each batch also checks
    /// the existing request ownership and approved scope. Reruns select remaining rows.
    pub async fn run_message_batches(
        &self,
        options: &ContentBatchOptions,
        mode: crate::user_deletion::ExecutionMode,
        stage: Option<(&EntityLease, Uuid)>,
        writer: &mut impl Write,
    ) -> Result<ContentBatchSummary> {
        match mode {
            crate::user_deletion::ExecutionMode::ReadOnly => {
                self.preview_message_batches(options, writer).await
            }
            crate::user_deletion::ExecutionMode::Delete => {
                self.execute_message_batches(options, writer, stage).await
            }
        }
    }

    async fn execute_message_batches(
        &self,
        options: &ContentBatchOptions,
        writer: &mut impl Write,
        stage: Option<(&EntityLease, Uuid)>,
    ) -> Result<ContentBatchSummary> {
        // A private one-connection pool pins the temporary worklist's session
        // while allowing each batch to own an admitted transaction. It never
        // returns temporary tables or session settings to the serving pool.
        let session = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect_with((*self.pool.connect_options()).clone())
            .await?;
        let result = execute_session(&session, options, writer, stage).await;
        session.close().await;
        result
    }
}

async fn execute_session(
    session: &sqlx::PgPool,
    options: &ContentBatchOptions,
    writer: &mut impl Write,
    stage: Option<(&EntityLease, Uuid)>,
) -> Result<ContentBatchSummary> {
    let mut planning = session.begin().await?;
    sqlx::query("SET statement_timeout='30s'")
        .execute(&mut *planning)
        .await?;
    sqlx::query("SET lock_timeout='3s'")
        .execute(&mut *planning)
        .await?;
    sqlx::query("SET TIME ZONE 'UTC'")
        .execute(&mut *planning)
        .await?;
    // Selection, retention and fingerprints share one snapshot. The temporary
    // worklist belongs to this detached session and disappears on disconnect.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *planning)
        .await?;
    let mut plan = plan(&mut planning, options, &mut std::io::sink(), true).await?;
    if plan.summary.warnings != 0 {
        return Err(invalid("selected records have active-work warnings; nothing deleted; inspect --read-only output"));
    }
    grouped::prepare(&mut planning, options.community, &plan).await?;
    planning.commit().await?;
    plan.start["record"] = json!("delete_start");
    plan.start["read_only"] = json!(false);
    plan.start["execution"] =
        json!("print each batch, commit, then print batch_committed; rerun after interruption");
    line(writer, &plan.start)?;
    for batch in &plan.batches {
        execute_owned_batch(session, options, batch, writer, stage).await?;
    }
    grouped::verify_remaining(&mut *session.acquire().await?, options, &plan.start).await?;
    plan.summary.record = "delete_complete";
    line(writer, &plan.summary)?;
    Ok(plan.summary)
}

pub(super) fn items(batch: &Value) -> Result<&Vec<Value>> {
    batch["items"]
        .as_array()
        .ok_or_else(|| invalid("invalid internal batch"))
}

// SQL identifiers and key expressions are exclusively from this static list.
// JSON keys are bound parameters, never interpolated SQL or caller input.
pub(super) fn selection(item: &Value) -> Result<(&'static str, &'static str)> {
    Ok(match item["table"].as_str() {
        Some("events") => ("events", "id=decode($2->>'id','hex') AND created_at=($2->>'created_at')::timestamptz"),
        Some("reactions") => ("reactions", "event_id=decode($2->>'event_id','hex') AND event_created_at=($2->>'event_created_at')::timestamptz AND pubkey=decode($2->>'pubkey','hex') AND emoji=$2->>'emoji'"),
        Some("event_mentions") => ("event_mentions", "event_id=decode($2->>'event_id','hex') AND pubkey_hex=$2->>'pubkey_hex'"),
        Some("push_match_queue") => ("push_match_queue", "event_id=decode($2->>'event_id','hex')"),
        Some("push_wake_outbox") => ("push_wake_outbox", "id=($2->>'id')::uuid"),
        Some("operator_listener_outbox") => ("operator_listener_outbox", "id=($2->>'id')::uuid"),
        Some("delivery_log") => ("delivery_log", "id=($2->>'id')::bigint AND delivered_at=($2->>'delivered_at')::timestamptz"),
        Some("subscriptions") => ("subscriptions", "id=$2->>'id'"),
        Some("push_leases") => ("push_leases", "author=decode($2->>'author','hex') AND installation_id=$2->>'installation_id'"),
        _ => return Err(invalid("unknown internal batch table")),
    })
}

async fn execute_owned_batch(
    session: &sqlx::PgPool,
    options: &ContentBatchOptions,
    batch: &Value,
    writer: &mut impl Write,
    stage: Option<(&EntityLease, Uuid)>,
) -> Result<()> {
    let community = options.community;
    let mut tx = session.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await?;
    // Check schema before blocking community writers, then keep the schema
    // lock until this batch commits. Never fall back to scanning history.
    reference_index::ensure(&mut tx).await?;
    // Serving writers already hold the shared form of this lock. Keep the
    // exclusive section short: at most one bounded batch, never the whole run.
    sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
        .bind(community)
        .execute(&mut *tx)
        .await?;
    crate::deletion::guard_community_write(&mut tx, crate::CommunityId::from_uuid(community))
        .await?;
    if let Some((lease, stage_id)) = stage {
        let request = crate::store::entity_deletion::lock_owned(&mut tx, lease).await?;
        crate::store::entity_deletion::validate_approved_inventory(&mut tx, &request).await?;
        let expected = crate::user_deletion::UserDeletionInventory::read(&request)?
            .inputs(&request.target, Some(community))?;
        let inputs: Option<Value> = sqlx::query_scalar("SELECT metadata_json->'inputs' FROM entity_deletion_stage_history WHERE id=$1 AND request_id=$2 AND stage='purge_content' AND target_community_id=$3 AND is_current AND status='running' FOR UPDATE")
            .bind(stage_id).bind(lease.request_id).bind(community).fetch_optional(&mut *tx).await?;
        if request.target != normalize_user_deletion_pubkey(&options.pubkey)?
            || inputs.as_ref() != Some(&expected)
        {
            return Err(invalid("content stage differs from approved scope"));
        }
    }
    let mut tx = crate::AdmittedTx::admit(
        tx,
        &crate::deletion::DeletionStore::new(session.clone()),
        crate::CommunityId::from_uuid(community),
    )
    .await?;
    // Output must succeed before any mutation in this transaction.
    line(writer, batch)?;
    grouped::execute(&mut tx, batch).await?;
    if let Some((lease, _)) = stage {
        crate::store::entity_deletion::lock_owned(tx.conn(), lease).await?;
    }
    tx.commit().await?;
    line(
        writer,
        &json!({"record":"batch_committed", "number":batch["number"], "items":items(batch)?.len()}),
    )?;
    Ok(())
}
