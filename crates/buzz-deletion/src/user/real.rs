//! Real, offline workflow adapters. Preview never acquires execution ownership.
use super::media;
use anyhow::{bail, ensure, Context, Result};
use buzz_db::{
    entity_deletion::{EntityDeletionStore, EntityLease, EntityRequest, EntityStage},
    user_deletion::{
        account_batches::AccountBatchOptions, preview::PreviewOptions, UserDeletionInventory,
    },
    Db,
};
use serde_json::json;
use uuid::Uuid;

/// Controls shared by workflow preview and explicit offline execution.
#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// Print proposed actions without writing source or workflow state (default).
    #[arg(long, alias = "read-only", conflicts_with = "delete")]
    pub print_only: bool,
    /// Execute the approved request's real handlers.
    #[arg(long, conflicts_with_all=["stage", "community"])]
    pub delete: bool,
    /// Confirm relay and external workers are stopped for this database.
    #[arg(long, requires = "delete")]
    pub offline: bool,
    /// Preview only this handler, without changing the saved workflow position.
    #[arg(long, value_parser=["purge_media", "purge_content", "purge_account"])]
    pub stage: Option<String>,
    /// Preview a single approved community's content/account operations.
    #[arg(long, requires = "stage")]
    pub community: Option<Uuid>,
    /// Canonical media base URLs used for reference validation.
    #[arg(long)]
    pub media_base: Vec<String>,
    /// Trusted relay key for historical content attribution.
    #[arg(long, value_parser=buzz_db::user_deletion::normalize_user_deletion_pubkey)]
    pub relay_pubkey: Option<String>,
    /// Maximum explicit row operations in each content/account batch.
    #[arg(long, default_value_t=100, value_parser=clap::value_parser!(u16).range(1..=100))]
    pub batch_size: u16,
    /// Maximum row operations in a content/account invocation.
    #[arg(long, default_value_t=1_000_000, value_parser=clap::value_parser!(u32).range(1..=1_000_000))]
    pub max_items: u32,
}

pub(super) struct Execution {
    pub db: Db,
    pub args: Args,
}

fn content_options(args: &Args, target: &str, community: Uuid) -> PreviewOptions {
    PreviewOptions {
        community,
        pubkey: target.into(),
        relay_pubkey: args.relay_pubkey.clone(),
        batch_size: args.batch_size,
        max_items: args.max_items,
    }
}
fn account_options(args: &Args, target: &str, community: Uuid) -> AccountBatchOptions {
    AccountBatchOptions {
        community,
        pubkey: target.into(),
        batch_size: args.batch_size,
        max_items: args.max_items,
    }
}
fn media_options(
    args: &Args,
    target: &str,
    namespace: String,
    mode: media::Mode,
) -> media::StageOptions {
    media::StageOptions {
        human: target.into(),
        include_owned_agents: false,
        storage_namespace: namespace,
        media_bases: args.media_base.clone(),
        scan_budget: Default::default(),
        mode,
    }
}

pub(super) async fn preview(
    db: &Db,
    request: &EntityRequest,
    args: &Args,
    next_step: bool,
) -> Result<i32> {
    ensure!(!args.delete, "preview cannot execute deletion");
    let inventory = UserDeletionInventory::read(request)?;
    if request.target.starts_with("demo:") {
        crate::print_json(
            &json!({"read_only":true,"demo":true,"inventory":request.inventory_manifest}),
        )?;
        return Ok(0);
    }
    if let Some(community) = args.community {
        ensure!(
            inventory.communities.iter().any(|(id, _)| *id == community),
            "preview community is outside the saved inventory"
        );
        ensure!(
            args.stage.as_deref() != Some("purge_media"),
            "media is request-wide; omit --community"
        );
    }
    let current = db.entity_deletion_store().inspect(request.id).await?;
    let selected = if next_step && args.stage.is_none() {
        Some(
            current
                .stage_history
                .iter()
                .find(|s| s.is_current)
                .map(|s| s.stage.as_str())
                .unwrap_or("purge_media"),
        )
    } else {
        args.stage.as_deref()
    };
    if let Some(stage) = selected {
        ensure!(
            matches!(
                stage,
                "purge_media" | "purge_content" | "purge_account" | "remaining_cleanup"
            ),
            "saved workflow stage is not supported by the real preview"
        );
    }
    if selected == Some("remaining_cleanup") {
        crate::print_json(&remaining())?;
        return Ok(2);
    }
    let mut complete = true;
    if selected.is_none() || selected == Some("purge_media") {
        let (storage, namespace) = media::configured_storage()?;
        let result = media::preview(
            db,
            &storage,
            &media_options(args, &request.target, namespace, media::Mode::PrintOnly),
        )
        .await?;
        complete &= result.get("decision_complete").and_then(|v| v.as_bool()) == Some(true);
        crate::print_json(&result)?;
    }
    for (community, _) in &inventory.communities {
        if args.community.is_some_and(|id| id != *community) {
            continue;
        }
        if next_step
            && args.stage.is_none()
            && current
                .stage_history
                .iter()
                .find(|s| s.is_current)
                .and_then(|s| s.target_community_id)
                .is_some_and(|id| id != *community)
        {
            continue;
        }
        let mut output = std::io::stdout();
        if selected.is_none() || selected == Some("purge_content") {
            db.user_deletion_store()
                .preview_message_batches(
                    &content_options(args, &request.target, *community),
                    &mut output,
                )
                .await?;
        }
        if selected.is_none() || selected == Some("purge_account") {
            let summary = db
                .user_deletion_store()
                .preview_account_batches(
                    &account_options(args, &request.target, *community),
                    &mut output,
                )
                .await?;
            complete &= summary.supported_scope_exhausted;
        }
    }
    crate::print_json(
        &json!({"record":if complete { "workflow_preview_complete" } else { "workflow_preview_incomplete" },"read_only":true,"account_erasure_complete":false}),
    )?;
    Ok(if complete { 0 } else { 2 })
}

fn remaining() -> serde_json::Value {
    json!({"record":"supported_cleanup_complete","account_erasure_complete":false,
        "remaining":["archive/replay", "required actor references", "audit/moderation", "devices", "global records", "cache/session invalidation", "profile/final identity cleanup", "owned agents"]})
}

pub(super) async fn dispatch(
    execution: &Execution,
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
) -> Result<()> {
    let db = &execution.db;
    let args = &execution.args;
    ensure!(
        args.delete && args.offline,
        "real handlers require explicit offline delete mode"
    );
    let row = store.start_stage(lease, row.id).await?;
    if row.stage == "remaining_cleanup" {
        let result = remaining();
        crate::print_json(&result)?;
        store.save_stage_waiting(lease, row.id, result).await?;
        return Ok(());
    }
    let result = execute(db, args, lease, &row).await;
    match result {
        Ok(value) => store.save_stage_success(lease, row.id, value).await?,
        Err(error) => {
            store
                .fail_stage(lease, row.id, &format!("{error:#}"), crate::RETRY_DELAY)
                .await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn execute(
    db: &Db,
    args: &Args,
    lease: &EntityLease,
    row: &EntityStage,
) -> Result<serde_json::Value> {
    let target = row.metadata_json["inputs"]["target"]
        .as_str()
        .context("missing stage target")?;
    buzz_db::user_deletion::normalize_user_deletion_pubkey(target)?;
    match row.stage.as_str() {
        "purge_media" => {
            ensure!(
                row.target_community_id.is_none(),
                "media must be request-wide"
            );
            // No online serving path is supported by this local/offline workflow.
            db.user_deletion_store()
                .verify_offline_work_stopped()
                .await?;
            let (storage, namespace) = media::configured_storage()?;
            media::run_stage(
                db,
                &storage,
                lease,
                row.id,
                media_options(args, target, namespace, media::Mode::Delete),
            )
            .await?;
            Ok(json!({"supported_scope_exhausted":true,"account_erasure_complete":false}))
        }
        "purge_content" => {
            let community = row
                .target_community_id
                .context("missing content community")?;
            let result = db
                .user_deletion_store()
                .delete_message_stage(
                    lease,
                    row.id,
                    &content_options(args, target, community),
                    &mut std::io::stdout(),
                )
                .await?;
            Ok(serde_json::to_value(result)?)
        }
        "purge_account" => {
            let community = row
                .target_community_id
                .context("missing account community")?;
            let result = db
                .user_deletion_store()
                .delete_account_stage(
                    lease,
                    row.id,
                    &account_options(args, target, community),
                    &mut std::io::stdout(),
                )
                .await?;
            ensure!(
                result.supported_scope_exhausted,
                "account invocation limit reached; rerun to continue"
            );
            Ok(serde_json::to_value(result)?)
        }
        _ => bail!("unsupported real workflow stage {}", row.stage),
    }
}
