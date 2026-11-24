//! Dispatch real account cleanup or demo handlers; transitions own routing.

use super::worker::RunOptions;
use crate::RETRY_DELAY;
use anyhow::{bail, ensure, Context, Result};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityStage};
use buzz_db::user_deletion::{
    account_batches::AccountBatchOptions, content_batches::ContentBatchOptions,
    profile_batches::ProfileBatchOptions, UserDeletionInventory,
};
use uuid::Uuid;

pub(super) enum HandlerProgress {
    Advance,
    Stop,
}

pub(super) async fn dispatch(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<HandlerProgress> {
    let row = store.start_stage(lease, row.id).await?;
    let outcome = execute_and_save(store, lease, &row, options).await;
    if let Err(error) = outcome {
        let reason = format!("{error:#}");
        store
            .fail_stage(lease, row.id, &reason, RETRY_DELAY)
            .await
            .with_context(|| format!("{reason}; could not record stage failure"))?;
        if row.metadata_json["inputs"]["target"]
            .as_str()
            .is_some_and(|t| t.starts_with("demo:"))
        {
            eprintln!("dummy stage {} failed: {reason}", row.stage);
        } else {
            return Err(error);
        }
        return Ok(HandlerProgress::Advance);
    }
    outcome
}

async fn execute_and_save(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<HandlerProgress> {
    let mut result = execute(store, lease, row, options).await?;
    if row.stage == "global_cleanup" {
        let summary = super::summary::final_review(store, lease, row, options, &result).await?;
        let waiting = !summary.remaining.is_empty();
        result["summary"] = serde_json::to_value(summary)?;
        if waiting {
            store.save_stage_waiting(lease, row.id, result).await?;
            return Ok(HandlerProgress::Stop);
        }
    }
    if row.stage == "delete_owned_community" && options.delete {
        let complete = super::owned_community::observed_complete(&result);
        store
            .save_owned_observation(lease, row.id, row.id, result, complete)
            .await?;
    } else {
        store.save_stage_success(lease, row.id, result).await?;
    }
    Ok(HandlerProgress::Advance)
}

async fn execute(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<serde_json::Value> {
    let target = row.metadata_json["inputs"]["target"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing target input"))?;
    if !target.starts_with("demo:") {
        let subject = target.strip_prefix("read-only:").unwrap_or(target);
        ensure!(
            options.delete == (target == subject),
            "execution mode differs from request"
        );
        if row.stage == "purge_media" {
            return purge_media(row, options, subject).await;
        }
        if row.stage == "purge_content" {
            return purge_content(store, lease, row, options, subject).await;
        }
        if row.stage == "purge_account" {
            let db = options.db.as_ref().context("missing account database")?;

            let mode = options.mode();
            let community = row
                .target_community_id
                .context("missing account community")?;
            let summary = super::account_batches::store(db, mode)?
                .run_account_batches(
                    &AccountBatchOptions {
                        community,
                        pubkey: subject.to_owned(),
                        batch_size: options.batch_size,
                        max_items: options.max_items,
                    },
                    mode,
                    &mut options.batch_output(row),
                    options.delete.then_some((lease, row.id)),
                )
                .await?;
            ensure!(
                summary.supported_scope_exhausted,
                "account batch limit reached; rerun to continue"
            );
            return Ok(serde_json::to_value(summary)?);
        }
        if matches!(
            row.stage.as_str(),
            "delete_owned_community" | "gate_user" | "global_cleanup" | "purge_profile"
        ) {
            return handle_remaining_stage(store, lease, row, options, subject).await;
        }
        bail!("real handler {} is not implemented", row.stage);
    }
    let message = match row.stage.as_str() {
        "purge_media" => "would purge user media",
        "gate_user" => "would gate user",
        "purge_content" => "would purge user content",
        "purge_agents" => "would purge user agents",
        name => bail!("unknown workflow stage {name}"),
    };
    perform(row, options, message).await
}

async fn purge_media(
    row: &EntityStage,
    options: &RunOptions,
    subject: &str,
) -> Result<serde_json::Value> {
    let db = options
        .db
        .as_ref()
        .context("media handler requires database")?;
    let configured;
    let storage = match options.media.as_deref() {
        Some(storage) => storage,
        None => {
            configured = super::media::configured_storage()?;
            &configured
        }
    };
    // Keep workflow stdout a single inspection JSON document.
    super::media::handle_with_storage(
        db,
        storage,
        subject,
        options.delete,
        &mut std::io::stderr(),
        options.ids(row).as_ref(),
    )
    .await
}

async fn purge_content(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
    subject: &str,
) -> Result<serde_json::Value> {
    let db = options.db.as_ref().context("missing content database")?;
    let request = store.get(lease.request_id).await?;
    let inventory = UserDeletionInventory::read(&request)?;
    let community = row
        .target_community_id
        .context("missing content community")?;
    ensure!(
        row.request_id == request.id
            && row.metadata_json["inputs"] == inventory.inputs(&request.target, Some(community))?,
        "content stage differs from approved inventory"
    );
    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &ContentBatchOptions {
                community,
                pubkey: subject.into(),
                relay_pubkey: options.relay_pubkey.clone(),
                batch_size: options.batch_size,
                max_items: options.max_items,
            },
            options.mode(),
            options.delete.then_some((lease, row.id)),
            &mut options.batch_output(row),
        )
        .await?;
    // Agent cleanup is a separate handler; do not claim it is unnecessary.
    Ok(serde_json::json!({"content": summary, "account_erasure_complete": false}))
}

async fn handle_remaining_stage(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
    subject: &str,
) -> Result<serde_json::Value> {
    match row.stage.as_str() {
        "gate_user" | "global_cleanup" => {
            let mode = if options.delete {
                "delete"
            } else {
                "read-only"
            };
            Ok(
                serde_json::json!({"execution_mode":mode,"implemented":false,"account_erasure_complete":false}),
            )
        }
        "delete_owned_community" => {
            let request = store.get(lease.request_id).await?;
            if !options.delete {
                let community = super::owned_community::approved_community(&request, row)?;
                let proposal = serde_json::json!({"community_id": community.as_uuid(), "action": "would_submit"});
                if let Some(ids) = options.ids(row) {
                    ids.record(proposal.clone())?;
                }
                return Ok(proposal);
            }
            let configured;
            let storage = match options.media.as_deref() {
                Some(storage) => storage,
                None => {
                    configured = super::media::configured_storage()?;
                    &configured
                }
            };
            let db = options.db.as_ref().context("missing source database")?;
            let metadata =
                super::owned_community::run_stage(&request, row, &db.deletion_store(), storage)
                    .await?;
            if let Some(ids) = options.ids(row) {
                ids.record(serde_json::json!({"child_request_id":metadata["child_request_id"],"action":"delegate"}))?;
            }
            Ok(metadata)
        }
        "purge_profile" => {
            let db = options.db.as_ref().context("missing source database")?;
            let profiles = db.user_deletion_store();
            let batch = ProfileBatchOptions {
                community: row
                    .target_community_id
                    .context("missing profile community")?,
                pubkey: subject.into(),
                relay_pubkeys: options.relay_pubkey.iter().cloned().collect(),
                batch_size: options.batch_size,
                max_items: options.max_items,
            };
            let mut output = options.batch_output(row);
            let summary = if options.delete {
                let secret = std::env::var("BUZZ_RELAY_PRIVATE_KEY").ok();
                profiles
                    .delete_profile_stage(&batch, secret.as_deref(), (lease, row.id), &mut output)
                    .await?
            } else {
                profiles.preview_profile_plan(&batch, &mut output).await?
            };
            Ok(serde_json::to_value(summary)?)
        }
        name => bail!("real handler {name} is not implemented for deletion"),
    }
}

async fn perform(
    row: &EntityStage,
    options: &RunOptions,
    message: &str,
) -> Result<serde_json::Value> {
    let inputs = &row.metadata_json["inputs"];
    let target = inputs["target"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing target input"))?;
    eprintln!(
        "DUMMY request={} stage={} community={:?} target={target}: {message}",
        row.request_id, row.stage, row.target_community_id
    );
    tokio::time::sleep(options.stage_delay).await;
    if options.fail_stage.as_deref() == Some(&row.stage) {
        bail!("injected dummy failure for {}", row.stage);
    }
    let mut result = serde_json::json!({"dummy": true, "message": message});
    if row.stage == "purge_content" {
        let community = inputs["community"]["community_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing community input"))?
            .parse::<Uuid>()?;
        if Some(community) != row.target_community_id {
            bail!("community input does not match stage");
        }
        // A simulated observation, persisted before routing. Restart uses this
        // result even if the next invocation supplies different demo options.
        result["has_agents"] = (!options.no_agents_in.contains(&community)).into();
    }
    Ok(result)
}
