//! Personal workflow fields and unused definitions; shared execution history stays.
use super::*;
use crate::AdmittedTx;

const RUN_INPUT: &str = "cause_pubkey=ANY($2::bytea[]) AND jsonb_typeof(trigger_context)='object'
    AND ((cause_source='event' AND jsonb_typeof(trigger_context->'text')='string'
        AND jsonb_typeof(trigger_context->'emoji')='string'
        AND (trigger_context->>'text'<>'' OR trigger_context->>'emoji'<>''))
      OR (cause_source='manual' AND jsonb_typeof(trigger_context->'webhook_fields')='object'
        AND trigger_context->'webhook_fields'<>'{}'::jsonb))";
const APPROVAL_NOTE: &str = "approver_pubkey=ANY($2::bytea[]) AND note IS NOT NULL";

// A schedule claim can link to a run independently of the run's workflow_id.
// Preserve even malformed cross-workflow links rather than cascade any history.
fn unused(alias: &str) -> String {
    format!("NOT EXISTS(SELECT 1 FROM workflow_runs r WHERE r.community_id={alias}.community_id AND r.workflow_id={alias}.id)
        AND NOT EXISTS(SELECT 1 FROM workflow_approvals a WHERE a.community_id={alias}.community_id AND a.workflow_id={alias}.id)
        AND NOT EXISTS(SELECT 1 FROM scheduled_workflow_fires s WHERE s.community_id={alias}.community_id AND s.workflow_id={alias}.id AND s.workflow_run_id IS NOT NULL)")
}

fn selections() -> Vec<(&'static str, String, &'static str)> {
    let empty = unused("f");
    vec![
        ("workflow_runs", format!("SELECT jsonb_build_object('id',id) AS row_key, NULL::text AS warning
            FROM workflow_runs WHERE community_id=$1 AND {RUN_INPUT}"), "would_scrub"),
        ("workflow_approvals", format!("SELECT jsonb_build_object('token',encode(token,'hex')) AS row_key, NULL::text AS warning
            FROM workflow_approvals WHERE community_id=$1 AND {APPROVAL_NOTE}"), "would_scrub"),
        ("workflows", format!("SELECT jsonb_build_object('id',f.id) AS row_key, NULL::text AS warning
            FROM workflows f WHERE f.community_id=$1 AND f.owner_pubkey=ANY($2::bytea[]) AND {empty}"), "would_delete"),
    ]
}

pub(super) async fn plan(
    conn: &mut PgConnection,
    targets: &[Vec<u8>],
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    for (table, select, action) in selections()
        .into_iter()
        .filter(|(t, _, _)| *t != "workflows")
    {
        rows::rows(
            conn,
            output,
            table,
            &select,
            targets,
            &json!({"type":"personal_workflow_data"}),
            action,
        )
        .await?;
    }
    // A claim and its unused parent must commit together: a partial prefix
    // must never leave enabled automation without its at-most-once claim.
    let query = format!(
        "SELECT f.id FROM workflows f WHERE f.community_id=$1
        AND f.owner_pubkey=ANY($2::bytea[]) AND {} AND ($3::uuid IS NULL OR f.id>$3)
        ORDER BY f.id LIMIT 100",
        unused("f")
    );
    let mut cursor: Option<Uuid> = None;
    loop {
        let ids: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(query.clone()))
            .bind(output.community)
            .bind(targets)
            .bind(cursor)
            .fetch_all(&mut *conn)
            .await?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            cursor = Some(id);
            let claims: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('workflow_id',workflow_id,'scheduled_for',scheduled_for)
                FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_id=$2 ORDER BY scheduled_for LIMIT $3")
                .bind(output.community).bind(id).bind(output.limit as i64).fetch_all(&mut *conn).await?;
            if claims.len() >= output.limit {
                return Err(invalid("unused workflow and its schedule claims exceed --batch-size; increase it (maximum 100); nothing deleted"));
            }
            output.flush()?;
            let reason = json!({"type":"unused_workflow", "atomic_with_schedule_claims":true});
            for key in claims {
                output.add(
                    "scheduled_workflow_fires",
                    key,
                    "would_delete",
                    &reason,
                    None,
                )?;
            }
            output.add("workflows", json!({"id":id}), "would_delete", &reason, None)?;
            output.flush()?;
        }
    }
    Ok(())
}

pub(super) async fn remaining(
    conn: &mut PgConnection,
    community: Uuid,
    targets: &[Vec<u8>],
) -> Result<bool> {
    for (_, select, _) in selections() {
        let exists: bool =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT EXISTS({select})")))
                .bind(community)
                .bind(targets)
                .fetch_one(&mut *conn)
                .await?;
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

// Recheck after locking the planned parent, including unplanned children. Never
// use its ON DELETE CASCADE to erase runs, approvals or unplanned schedule rows.
pub(super) const DEFINITION_CHILDREN: &str = "EXISTS(SELECT 1 FROM workflow_runs WHERE community_id=$1 AND workflow_id=(w.row_key->>'id')::uuid)
    OR EXISTS(SELECT 1 FROM workflow_approvals WHERE community_id=$1 AND workflow_id=(w.row_key->>'id')::uuid)
    OR EXISTS(SELECT 1 FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_id=(w.row_key->>'id')::uuid)";

pub(super) const FIRE_HISTORY: &str = "EXISTS(SELECT 1 FROM workflow_runs WHERE community_id=$1 AND workflow_id=(w.row_key->>'workflow_id')::uuid)
    OR EXISTS(SELECT 1 FROM workflow_approvals WHERE community_id=$1 AND workflow_id=(w.row_key->>'workflow_id')::uuid)
    OR EXISTS(SELECT 1 FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_id=(w.row_key->>'workflow_id')::uuid AND workflow_run_id IS NOT NULL)
";

pub(super) async fn scrub(
    tx: &mut AdmittedTx,
    number: i64,
    table: &str,
    expected: usize,
) -> Result<()> {
    let query = match table {
        // Keep the required serialized TriggerContext fields, so retained runs
        // remain readable. Do not touch outputs, provenance or unrelated input.
        "workflow_runs" => "UPDATE workflow_runs t SET trigger_context=CASE t.cause_source
            WHEN 'event' THEN t.trigger_context || '{\"text\":\"\",\"emoji\":\"\"}'::jsonb
            WHEN 'manual' THEN t.trigger_context || '{\"webhook_fields\":{}}'::jsonb END
            FROM content_batch_work w WHERE w.table_name='workflow_runs' AND w.batch=$2
                AND w.action='would_scrub' AND t.community_id=$1 AND t.id=(w.row_key->>'id')::uuid",
        "workflow_approvals" => "UPDATE workflow_approvals t SET note=NULL
            FROM content_batch_work w WHERE w.table_name='workflow_approvals' AND w.batch=$2
                AND w.action='would_scrub' AND t.community_id=$1 AND t.token=decode(w.row_key->>'token','hex')",
        _ => return Err(invalid("unsupported workflow scrub")),
    };
    let community = *tx.community().as_uuid();
    let affected = sqlx::query(query)
        .bind(community)
        .bind(number)
        .execute(tx.conn())
        .await?
        .rows_affected();
    if affected as usize != expected {
        return Err(invalid(
            "workflow rows missing or unsupported action; current batch rolled back",
        ));
    }
    Ok(())
}
