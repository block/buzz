//! Account cleanup shares its selection engine across read-only and delete.
use super::{account_options, HandlerContext};
use anyhow::{ensure, Context, Result};

pub(super) async fn run(context: &HandlerContext<'_>) -> Result<serde_json::Value> {
    let community = context
        .row
        .target_community_id
        .context("missing account community")?;
    let summary = context
        .db
        .user_deletion_store()
        .run_account_stage(
            context.lease,
            context.row.id,
            &account_options(context.args, context.subject, community),
            context.mode,
            &mut std::io::stdout(),
        )
        .await?;
    ensure!(
        summary.supported_scope_exhausted,
        "account invocation limit reached; stage has not completed"
    );
    Ok(serde_json::to_value(summary)?)
}
