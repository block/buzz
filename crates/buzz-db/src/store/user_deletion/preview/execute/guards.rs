use super::*;

pub(super) async fn community(conn: &mut PgConnection, community: Uuid) -> Result<()> {
    let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE id=$1 AND deletion_state='active' AND deleted_at IS NULL)
        AND NOT EXISTS(SELECT 1 FROM community_serving_write_leases WHERE community_id=$1 AND lease_until>clock_timestamp())")
        .bind(community).fetch_one(conn).await?;
    if !ready {
        return Err(invalid(
            "community must be active with external serving work stopped before deletion",
        ));
    }
    Ok(())
}

pub(super) async fn inspect(
    conn: &mut PgConnection,
    community: Uuid,
    table: &str,
    predicate: &str,
    key: &Value,
    preflight: bool,
    evidence: Option<&media::Evidence>,
) -> Result<String> {
    if table == "workflow_approvals" {
        let inconsistent: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workflow_approvals a JOIN workflow_runs r ON r.community_id=a.community_id AND r.id=a.run_id WHERE a.community_id=$1 AND a.token=decode($2->>'token_hash','hex') AND a.workflow_id<>r.workflow_id)").bind(community).bind(key).fetch_one(&mut *conn).await?;
        if inconsistent {
            return Err(invalid(
                "approval and run workflow disagree; reconcile before deletion",
            ));
        }
    }
    let media = match table {
        "events" => "content || tags::text",
        "workflow_runs" => "COALESCE(trigger_context::text,'') || execution_trace::text || COALESCE(error_message,'')",
        "workflow_approvals" => "COALESCE(note,'') || approver_spec",
        "workflows" => "definition::text",
        "delivery_log" => "COALESCE(error_message,'')",
        _ => "''::text",
    };
    let active = match table {
        "workflow_runs" => "status NOT IN ('completed','failed','cancelled')",
        "push_match_queue" => "COALESCE(lease_until>clock_timestamp(),false)",
        "push_wake_outbox" | "operator_listener_outbox" => {
            "state='sending' OR COALESCE(lease_until>clock_timestamp(),false)"
        }
        _ => "false",
    };
    // These formats may hide media in ciphertext. An empty URL scan cannot
    // authorize losing their source before media evidence is integrated.
    let opaque = if table == "events" {
        "(kind IN (10000,10001,10003,30000,30003,30078,30300,30174,44200,30350) AND content<>'') OR tags @> '[\"encrypted\"]'::jsonb OR tags @> '[[\"encrypted\"]]'::jsonb"
    } else {
        "false"
    };
    let lock = if preflight { "" } else { " FOR UPDATE" };
    let query = format!("SELECT md5(to_jsonb(t)::text),({media}) ~* 'https?://|/media/',({active}),({opaque}) FROM {table} t WHERE community_id=$1 AND {predicate}{lock}");
    let (hash, media, active, opaque): (String, bool, bool, bool) =
        sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(community)
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| invalid(format!("selected {table} row is missing")))?;
    if opaque || (media && evidence.is_none()) {
        return Err(invalid(format!(
            "selected {table} payload requires media evidence; nothing further deleted"
        )));
    }
    if active {
        return Err(invalid(format!(
            "selected {table} has active work; nothing further deleted"
        )));
    }
    if let Some(evidence) = evidence {
        evidence
            .verify_source(conn, community, table, predicate, key)
            .await?;
    }
    Ok(hash)
}

// No unchecked FK cascade is allowed to do extra work outside the printed
// batch. New children make the current transaction fail rather than disappear.
pub(super) async fn children(
    conn: &mut PgConnection,
    community: Uuid,
    table: &str,
    key: &Value,
) -> Result<()> {
    let query = match table {
        "workflows" => "SELECT EXISTS(SELECT 1 FROM workflow_runs WHERE community_id=$1 AND workflow_id=($2->>'id')::uuid)
            OR EXISTS(SELECT 1 FROM workflow_approvals WHERE community_id=$1 AND workflow_id=($2->>'id')::uuid)
            OR EXISTS(SELECT 1 FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_id=($2->>'id')::uuid)",
        "workflow_runs" => "SELECT EXISTS(SELECT 1 FROM workflow_approvals WHERE community_id=$1 AND run_id=($2->>'id')::uuid)
            OR EXISTS(SELECT 1 FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_run_id=($2->>'id')::uuid)",
        "subscriptions" => "SELECT EXISTS(SELECT 1 FROM delivery_log WHERE community_id=$1 AND subscription_id=$2->>'id')",
        "push_leases" => "SELECT EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND author=decode($2->>'author','hex') AND installation_id=$2->>'installation_id')",
        "events" => include_str!("event_children.sql"),
        _ => return Ok(()),
    };
    let remains: bool = sqlx::query_scalar(query)
        .bind(community)
        .bind(key)
        .fetch_one(conn)
        .await?;
    if remains {
        return Err(invalid(format!(
            "{table} still has dependent rows; current batch rolled back"
        )));
    }
    Ok(())
}
