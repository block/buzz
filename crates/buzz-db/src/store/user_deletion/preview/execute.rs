//! Execution of an internally generated plan, never an imported JSON manifest.
use super::*;
use sqlx::Connection;
use std::collections::HashMap;

mod guards;
mod repair;

impl UserDeletionStore {
    /// Print and execute the supported cleanup in transactions of at most 100
    /// explicit row operations. Operators must stop external workers first.
    ///
    /// Holds the community write lock across planning and commits. Any error
    /// leaves earlier commits intact; rerunning selects the remaining rows.
    /// No durable account-deletion request or stage is completed by this API.
    pub async fn delete_message_batches(
        &self,
        options: &PreviewOptions,
        writer: &mut impl Write,
    ) -> Result<PreviewSummary> {
        // Own the physical connection: cancellation/error must close it rather
        // than return session advisory locks to a connection pool.
        let mut conn = self.pool.acquire().await?.detach();
        sqlx::query("SET statement_timeout='30s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET lock_timeout='3s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET TIME ZONE 'UTC'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SELECT pg_advisory_lock_shared($1)")
            .bind(crate::store::deletion::SCHEMA_DESTRUCTION_LOCK_KEY)
            .execute(&mut conn)
            .await?;
        sqlx::query("SELECT pg_advisory_lock(community_deletion_lock_key($1))")
            .bind(options.community)
            .execute(&mut conn)
            .await?;
        crate::store::deletion::validate_catalog_on(&mut conn).await?;
        guards::community(&mut conn, options.community).await?;
        let mut plan = plan(&mut conn, options, &mut std::io::sink(), true).await?;
        plan.start["record"] = json!("delete_start");
        plan.start["read_only"] = json!(false);
        plan.start["execution"] =
            json!("print each batch, commit, then print batch_committed; rerun after interruption");
        line(writer, &plan.start)?;
        if plan.summary.warnings != 0 {
            return Err(invalid("selected records have media or active-work warnings; nothing deleted; inspect --read-only output"));
        }
        // Validate *all* source payloads before deleting even the first child.
        let mut fingerprints = HashMap::new();
        for batch in &plan.batches {
            for item in items(batch)? {
                let (table, predicate) = selection(item)?;
                let hash = guards::inspect(
                    &mut conn,
                    options.community,
                    table,
                    predicate,
                    &item["key"],
                    true,
                )
                .await?;
                fingerprints.insert(identity(item), hash);
            }
        }
        for batch in &plan.batches {
            execute_batch(&mut conn, options.community, batch, &fingerprints, writer).await?;
        }
        let remaining = plan_remaining(&mut conn, options).await?;
        if remaining != 0 {
            return Err(invalid(
                "supported rows remain after execution; deletion incomplete",
            ));
        }
        plan.summary.record = "delete_complete";
        // Closing releases session locks on success as well as on cancellation.
        conn.close().await?;
        line(writer, &plan.summary)?;
        Ok(plan.summary)
    }
}

async fn plan_remaining(conn: &mut PgConnection, options: &PreviewOptions) -> Result<usize> {
    Ok(plan(conn, options, &mut std::io::sink(), false)
        .await?
        .summary
        .items)
}

fn identity(item: &Value) -> String {
    format!("{}:{}", item["table"], item["key"])
}
fn items(batch: &Value) -> Result<&Vec<Value>> {
    batch["items"]
        .as_array()
        .ok_or_else(|| invalid("invalid internal batch"))
}

// SQL identifiers and key expressions are exclusively from this static list.
// JSON keys are bound parameters, never interpolated SQL or caller input.
fn selection(item: &Value) -> Result<(&'static str, &'static str)> {
    Ok(match item["table"].as_str() {
        Some("events") => ("events", "id=decode($2->>'id','hex') AND created_at=($2->>'created_at')::timestamptz"),
        Some("reactions") => ("reactions", "event_id=decode($2->>'event_id','hex') AND event_created_at=($2->>'event_created_at')::timestamptz AND pubkey=decode($2->>'pubkey','hex') AND emoji=$2->>'emoji'"),
        Some("event_mentions") => ("event_mentions", "event_id=decode($2->>'event_id','hex') AND pubkey_hex=$2->>'pubkey_hex'"),
        Some("push_match_queue") => ("push_match_queue", "event_id=decode($2->>'event_id','hex')"),
        Some("push_wake_outbox") => ("push_wake_outbox", "id=($2->>'id')::uuid"),
        Some("operator_listener_outbox") => ("operator_listener_outbox", "id=($2->>'id')::uuid"),
        Some("delivery_log") => ("delivery_log", "id=($2->>'id')::bigint AND delivered_at=($2->>'delivered_at')::timestamptz"),
        Some("workflow_approvals") => ("workflow_approvals", "token=decode($2->>'token_hash','hex')"),
        Some("scheduled_workflow_fires") => ("scheduled_workflow_fires", "workflow_id=($2->>'workflow_id')::uuid AND scheduled_for=($2->>'scheduled_for')::timestamptz"),
        Some("workflow_runs") => ("workflow_runs", "id=($2->>'id')::uuid"),
        Some("workflows") => ("workflows", "id=($2->>'id')::uuid"),
        Some("subscriptions") => ("subscriptions", "id=$2->>'id'"),
        Some("push_leases") => ("push_leases", "author=decode($2->>'author','hex') AND installation_id=$2->>'installation_id'"),
        _ => return Err(invalid("unknown internal batch table")),
    })
}

pub(super) async fn execute_batch(
    conn: &mut PgConnection,
    community: Uuid,
    batch: &Value,
    fingerprints: &HashMap<String, String>,
    writer: &mut impl Write,
) -> Result<()> {
    let mut tx = conn.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await?;
    guards::community(&mut tx, community).await?;
    // Output must succeed before any mutation in this transaction.
    line(writer, batch)?;
    for item in items(batch)? {
        let (table, predicate) = selection(item)?;
        let hash =
            guards::inspect(&mut tx, community, table, predicate, &item["key"], false).await?;
        if fingerprints.get(&identity(item)) != Some(&hash) {
            return Err(invalid(format!(
                "{table} row changed after planning; current batch rolled back"
            )));
        }
        guards::children(&mut tx, community, table, &item["key"]).await?;
        let verb = if item["action"] == "would_clear_workflow_run_id"
            && table == "scheduled_workflow_fires"
        {
            "UPDATE scheduled_workflow_fires SET workflow_run_id=NULL".to_owned()
        } else if item["action"] == "would_delete" {
            format!("DELETE FROM {table}")
        } else {
            return Err(invalid("unsupported internally generated action"));
        };
        let sql = format!("{verb} WHERE community_id=$1 AND {predicate}");
        let count = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community)
            .bind(&item["key"])
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if count != 1 {
            return Err(invalid("planned row missing; current batch rolled back"));
        }
    }
    repair::batch(&mut tx, community, batch).await?;
    tx.commit().await?;
    line(
        writer,
        &json!({"record":"batch_committed", "number":batch["number"], "items":items(batch)?.len()}),
    )?;
    Ok(())
}
