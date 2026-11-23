//! Real read-only adapters; only the journal and owned-community handoff can write.
use super::worker::RunOptions;
use anyhow::{bail, ensure, Context, Result};
use buzz_db::{
    entity_deletion::{EntityDeletionStore, EntityLease, EntityRequest, EntityStage},
    user_deletion::{
        account_batches::AccountBatchOptions, content_batches::ContentBatchOptions,
        profile_batches::ProfileBatchOptions, ExecutionMode, UserDeletionInventory as Inventory,
    },
    Db,
};
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) async fn dispatch(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<()> {
    let request = store.get(lease.request_id).await?;
    super::worker::validate_mode(&request, options.delete, options.read_only)?;
    let inventory = Inventory::read(&request)?;
    ensure!(
        row.request_id == request.id
            && row.metadata_json["inputs"]
                == inventory.inputs(&request.target, row.target_community_id)?,
        "rehearsal stage differs from approved inventory"
    );
    let db = options
        .db
        .as_ref()
        .context("missing read-only source database")?;
    if row.stage == "delete_owned_community" {
        // This explicit exception writes only child intent/inventory. The source
        // pool remains read-only; the existing journal pool owns the handoff.
        let configured;
        let media = match options.media.as_deref() {
            Some(media) => media,
            None => {
                configured = super::media::configured_storage()?;
                &configured
            }
        };
        let communities = options
            .community_store
            .as_ref()
            .context("missing child journal")?;
        super::owned_community::handoff(store, lease, row, communities, media).await?;
        return Ok(());
    }
    store.start_stage(lease, row.id).await?;
    let result = if row.stage == "remaining_cleanup" {
        report(store, &request).await
    } else {
        preview_handler(db, &request, &row.stage, row.target_community_id, options).await
    };
    match result {
        Ok(mut value) => {
            value["execution_mode"] = "rehearsal".into();
            value["account_erasure_complete"] = false.into();
            store.save_stage_success(lease, row.id, value).await?;
            if row.stage == "purge_media" && !options.next_step {
                use std::io::Write;
                // This is a progress hint after commit. A closed stderr cannot
                // invalidate the saved stage or its transition.
                let _ = writeln!(std::io::stderr(), "Media preview saved. View file decisions now with: buzz-admin user-deletions inspect {}", request.id);
            }
        }
        Err(error) => {
            if let Err(save) = store
                .fail_stage(lease, row.id, &format!("{error:#}"), crate::RETRY_DELAY)
                .await
            {
                return Err(error.context(format!("also could not record retry: {save}")));
            }
            return Err(error);
        }
    }
    Ok(())
}

pub(super) async fn preview_handler(
    db: &Db,
    request: &EntityRequest,
    stage: &str,
    community: Option<Uuid>,
    options: &RunOptions,
) -> Result<Value> {
    let subject = Inventory::subject(request)?;
    let users = db.user_deletion_store();
    let mut output = std::io::stderr();
    match stage {
        "gate_user" | "global_cleanup" => Ok(json!({
            "simulated":true,"stage":stage,"source_data_unchanged":true,
            "detail":"dummy rehearsal stage; no gate or global cleanup performed"
        })),
        "purge_media" => {
            let configured;
            let media = match options.media.as_deref() {
                Some(media) => media,
                None => {
                    configured = super::media::configured_storage()?;
                    &configured
                }
            };
            if options.read_only {
                super::media::stage_preview(db, media, subject).await
            } else {
                super::media::handle_with_storage(db, media, subject, false, &mut output).await
            }
        }
        "purge_content" => {
            let summary = users
                .run_message_batches(
                    &ContentBatchOptions {
                        community: community.context("missing content community")?,
                        pubkey: subject.into(),
                        relay_pubkey: options.relay_pubkey.clone(),
                        batch_size: options.batch_size,
                        max_items: options.max_items,
                    },
                    ExecutionMode::ReadOnly,
                    None,
                    &mut output,
                )
                .await?;
            Ok(json!({"content":summary,"account_erasure_complete":false}))
        }
        "purge_account" => {
            let summary = users
                .run_account_batches(
                    &AccountBatchOptions {
                        community: community.context("missing account community")?,
                        pubkey: subject.into(),
                        batch_size: options.batch_size,
                        max_items: options.max_items,
                    },
                    ExecutionMode::ReadOnly,
                    &mut output,
                    None,
                )
                .await?;
            ensure!(
                summary.supported_scope_exhausted,
                "account preview limit reached; increase --max-items and retry"
            );
            Ok(serde_json::to_value(summary)?)
        }
        "purge_profile" => {
            let profile = ProfileBatchOptions {
                community: community.context("missing profile community")?,
                pubkey: subject.into(),
                relay_pubkeys: options.relay_pubkey.iter().cloned().collect(),
                batch_size: options.batch_size,
                max_items: options.max_items,
            };
            let summary = if options.read_only {
                users
                    .preview_profile_plan(&profile, &mut std::io::sink())
                    .await?
            } else {
                users.preview_profile_plan(&profile, &mut output).await?
            };
            Ok(serde_json::to_value(summary)?)
        }
        "delete_owned_community" => Ok(
            json!({"community":community,"would_submit_or_adopt_child":true,
            "child_submitted":false,"detail":"stateless preview; persisted rehearsal records actual handoff"}),
        ),
        _ => bail!("unsupported rehearsal preview stage {stage}"),
    }
}

async fn report(store: &EntityDeletionStore, request: &EntityRequest) -> Result<Value> {
    let inspection = store.inspect(request.id).await?;
    let mut stages = Vec::new();
    for mut stage in inspection
        .stage_history
        .into_iter()
        .filter(|s| s.stage != "remaining_cleanup")
    {
        ensure!(
            stage.status == "completed",
            "cannot report completion with unfinished stages"
        );
        let mut result = stage.metadata_json["result"].take();
        // The final report links to the full media preview by stage ID.
        if stage.stage == "purge_media" {
            if let Some(media) = result.as_object_mut() {
                media.remove("files");
                media.remove("target_pubkeys");
            }
        }
        stages.push(json!({
            "id": stage.id, "stage": stage.stage, "community": stage.target_community_id,
            "status": stage.status, "result": result,
        }));
    }
    let report = json!({"record":"rehearsal_report","request_id":request.id,
        "run_id":request.inventory_manifest.as_ref().map(|m| &m["run_id"]),
        "subject":Inventory::subject(request)?,"source_data_unchanged":true,
        "account_erasure_complete":false,"stages":stages,
        "unsupported":["online gate and writer drain","global cleanup","cache reconciliation","final erasure verification"],
        "meaning":"inspection finished; no user data deleted; community children need separate approval and execution"});
    // Output failure is a retryable handler error, not a println! panic that
    // bypasses the worker's lease release and failure journal.
    {
        use std::io::Write;
        let mut output = std::io::stdout().lock();
        serde_json::to_writer_pretty(&mut output, &report)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(report)
}
