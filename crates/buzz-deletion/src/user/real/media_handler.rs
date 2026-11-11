//! Media owns its mode policy; the dispatcher only selects this handler.
use super::{media, media_options, ExecutionMode, HandlerContext};
use anyhow::{ensure, Result};
use serde_json::Value;

pub(super) async fn run(context: &HandlerContext<'_>) -> Result<Value> {
    ensure!(
        context.row.target_community_id.is_none(),
        "media must be request-wide"
    );
    let mode = match context.mode {
        ExecutionMode::ReadOnly => media::Mode::PrintOnly,
        ExecutionMode::Delete => media::Mode::Delete,
    };
    let (storage, namespace) = media::configured_storage()?;
    media::run_stage(
        context.db,
        &storage,
        context.lease,
        context.row.id,
        media_options(context.subject, namespace, mode),
    )
    .await
}
