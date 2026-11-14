//! Fail closed before deletion if indexed reference lookup is unavailable.
use super::*;

const DEFINITION: &str = r#"CREATE INDEX idx_events_reference_ids ON ONLY public.events USING gin (((lower((jsonb_path_query_array(tags, 'strict $[*]?(@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]'::jsonpath, '{}'::jsonb, true))::text))::jsonb) jsonb_path_ops)"#;

pub(super) async fn ensure(conn: &mut PgConnection) -> Result<()> {
    // Match the migration runner's exclusive schema lock. Hold this through
    // each transaction so supported DDL cannot invalidate the checked index.
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(crate::deletion::SCHEMA_DESTRUCTION_LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    let ready: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_index i
         WHERE i.indexrelid=to_regclass('public.idx_events_reference_ids')
           AND i.indrelid='public.events'::regclass
           AND i.indisvalid AND i.indisready AND i.indislive
           AND pg_get_indexdef(i.indexrelid)=$1)",
    )
    .bind(DEFINITION)
    .fetch_one(conn)
    .await?;
    if !ready {
        return Err(invalid("event reference index missing, invalid or wrong definition; apply migration 0089; see docs/event-reference-index-deployment.md"));
    }
    Ok(())
}
