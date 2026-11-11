//! Require the completed request-wide upload cleanup before deleting content.
//! Copied links do not establish upload ownership and need no per-source evidence.
use super::*;
use sqlx::{Postgres, Transaction};

pub(super) async fn verify_completed(
    tx: &mut Transaction<'_, Postgres>,
    context: &stage::Context<'_>,
) -> Result<()> {
    let stages: Vec<(Uuid, Value)> = sqlx::query_as("SELECT id,metadata_json FROM entity_deletion_stage_history WHERE request_id=$1 AND stage='purge_media' AND target_community_id IS NULL AND status='completed' AND NOT is_current LIMIT 2")
        .bind(context.lease.request_id).fetch_all(&mut **tx).await?;
    let [(stage, metadata)] = stages.as_slice() else {
        return Err(invalid(
            "content deletion requires one completed request-wide media stage",
        ));
    };
    let binding = &metadata["binding"];
    if binding["policy"] != "upload-ownership-v1"
        || binding["mode"] != "delete"
        || binding["human"] != normalize_user_deletion_pubkey(&context.options.pubkey)?
        || metadata["media_discovery"]["complete"] != true
    {
        return Err(invalid(
            "media stage has no completed destructive inventory for this user",
        ));
    }
    let namespace = binding["storage_namespace"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("media namespace missing"))?
        .to_owned();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_deletion_media_files WHERE stage_id=$1")
            .bind(stage)
            .fetch_one(&mut **tx)
            .await?;
    if metadata["media_discovery"]["candidate_count"] != json!(count) {
        return Err(invalid("completed media inventory row count changed"));
    }
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_deletion_media_files WHERE stage_id=$1 AND (storage_namespace<>$2 OR status NOT IN ('deleted','dont_delete') OR last_error IS NOT NULL))")
        .bind(stage).bind(&namespace).fetch_one(&mut **tx).await?;
    if pending {
        return Err(invalid(
            "media stage has unfinished or mismatched file evidence",
        ));
    }
    Ok(())
}
