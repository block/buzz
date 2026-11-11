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

mod account;
mod content;
mod gate;
mod media_handler;

pub(super) use buzz_db::user_deletion::ExecutionMode;

/// Identical arguments for every real workflow handler in both modes.
pub(super) struct HandlerContext<'a> {
    pub db: &'a Db,
    pub args: &'a Args,
    pub request: &'a EntityRequest,
    pub lease: &'a EntityLease,
    pub row: &'a EntityStage,
    pub subject: &'a str,
    pub mode: ExecutionMode,
}

/// Controls shared by workflow preview and explicit offline execution.
#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// Print proposed actions without writing source or workflow state (default).
    #[arg(long, conflicts_with_all = ["delete", "rehearse"])]
    pub print_only: bool,
    /// Execute the approved request's real handlers.
    #[arg(long, conflicts_with_all=["stage", "community", "rehearse"])]
    pub delete: bool,
    /// Run handlers read-only while persisting progress on a separate print request.
    #[arg(long = "read-only", alias = "rehearse", conflicts_with_all=["delete", "print_only", "stage", "community", "offline"])]
    pub rehearse: bool,
    /// Confirm relay and external workers are stopped for this database.
    #[arg(long, requires = "delete")]
    pub offline: bool,
    /// Preview only this handler, without changing the saved workflow position.
    #[arg(long, value_parser=["gate_user", "purge_media", "purge_content", "purge_account"])]
    pub stage: Option<String>,
    /// Preview a single approved community's content/account operations.
    #[arg(long, requires = "stage")]
    pub community: Option<Uuid>,
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

impl Args {
    pub(super) fn validate_execution(&self, request: &EntityRequest) -> Result<()> {
        ensure!(
            self.delete != self.rehearse,
            "select exactly one worker mode: --delete or --read-only"
        );
        ensure!(
            !self.print_only && self.stage.is_none() && self.community.is_none(),
            "worker execution cannot select a preview stage"
        );
        let rehearsal = UserDeletionInventory::is_rehearsal(request)?;
        ensure!(
            self.rehearse == rehearsal,
            "worker mode does not match request; use a separate rehearsal/deletion request"
        );
        if self.rehearse {
            ensure!(!self.offline, "--offline is only an assertion for deletion");
        } else if !request.target.starts_with("demo:") {
            ensure!(
                self.offline,
                "stop relay and external workers, then supply --offline with --delete"
            );
        }
        Ok(())
    }
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
fn media_options(target: &str, namespace: String, mode: media::Mode) -> media::StageOptions {
    media::StageOptions {
        human: target.into(),
        include_owned_agents: false,
        storage_namespace: namespace,
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
    let subject = UserDeletionInventory::subject(request)?;
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
            !matches!(args.stage.as_deref(), Some("gate_user" | "purge_media")),
            "gate and media are request-wide; omit --community"
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
                .unwrap_or("gate_user"),
        )
    } else {
        args.stage.as_deref()
    };
    if let Some(stage) = selected {
        ensure!(
            matches!(
                stage,
                "gate_user"
                    | "purge_media"
                    | "purge_content"
                    | "purge_account"
                    | "remaining_cleanup"
            ),
            "saved workflow stage is not supported by the real preview"
        );
    }
    if selected == Some("remaining_cleanup") {
        crate::print_json(&remaining())?;
        return Ok(2);
    }
    let mut complete = true;
    if selected.is_none() || selected == Some("gate_user") {
        crate::print_json(&gate::proposal(subject))?;
    }
    if selected.is_none() || selected == Some("purge_media") {
        let (storage, namespace) = media::configured_storage()?;
        let result = media::preview(
            db,
            &storage,
            &media_options(subject, namespace, media::Mode::PrintOnly),
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
                .preview_message_batches(&content_options(args, subject, *community), &mut output)
                .await?;
        }
        if selected.is_none() || selected == Some("purge_account") {
            let summary = db
                .user_deletion_store()
                .preview_account_batches(&account_options(args, subject, *community), &mut output)
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
    let request = store.get(lease.request_id).await?;
    args.validate_execution(&request)?;
    ensure!(
        row.request_id == request.id && row.metadata_json["inputs"]["target"] == request.target,
        "stage does not match request target"
    );
    let row = store.start_stage(lease, row.id).await?;
    if row.stage == "remaining_cleanup" {
        let mut result = remaining();
        if args.rehearse {
            result["record"] = "supported_cleanup_rehearsed".into();
            result["execution_mode"] = "rehearsal".into();
        }
        crate::print_json(&result)?;
        store.save_stage_waiting(lease, row.id, result).await?;
        return Ok(());
    }
    let context = HandlerContext {
        db,
        args,
        request: &request,
        lease,
        row: &row,
        subject: UserDeletionInventory::subject(&request)?,
        mode: if args.rehearse {
            ExecutionMode::ReadOnly
        } else {
            ExecutionMode::Delete
        },
    };
    let result = execute(&context).await.map(|proposal| {
        if context.mode == ExecutionMode::ReadOnly {
            json!({"execution_mode":"rehearsal", "source_data_unchanged":true,
                "account_erasure_complete":false, "proposal":proposal})
        } else {
            proposal
        }
    });
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

async fn execute(context: &HandlerContext<'_>) -> Result<serde_json::Value> {
    let inventory = UserDeletionInventory::read(context.request)?;
    ensure!(
        context.row.metadata_json["inputs"]
            == inventory.inputs(&context.request.target, context.row.target_community_id)?,
        "handler inputs differ from approved inventory"
    );
    // Stage routing is independent of mode. Each handler owns its effects.
    match context.row.stage.as_str() {
        "gate_user" => gate::run(context).await,
        "purge_media" => media_handler::run(context).await,
        "purge_content" => content::run(context).await,
        "purge_account" => account::run(context).await,
        _ => bail!("unsupported real workflow stage {}", context.row.stage),
    }
}
