//! Message cleanup uses the same planner in read-only and delete modes.
use super::*;

pub(super) async fn run(context: &HandlerContext<'_>) -> Result<serde_json::Value> {
    let community = context
        .row
        .target_community_id
        .context("missing content community")?;
    let summary = context
        .db
        .user_deletion_store()
        .run_message_stage(
            context.lease,
            context.row.id,
            &content_options(context.args, context.subject, community),
            context.mode,
            &mut std::io::stdout(),
        )
        .await?;
    Ok(serde_json::to_value(summary)?)
}
