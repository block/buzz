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

pub(super) async fn dispatch(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<()> {
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
            return purge_media(store, lease, row, options, subject).await;
        }
        if row.stage == "purge_content" {
            return purge_content(store, lease, row, options, subject).await;
        }
        if row.stage == "purge_account" {
            let db = options.db.as_ref().context("missing account database")?;
            let row = store.start_stage(lease, row.id).await?;
            let result = async {
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
                        &mut options.batch_output(&row),
                        options.delete.then_some((lease, row.id)),
                    )
                    .await?;
                ensure!(
                    summary.supported_scope_exhausted,
                    "account batch limit reached; rerun to continue"
                );
                Ok::<_, anyhow::Error>(serde_json::to_value(summary)?)
            }
            .await;
            match result {
                Ok(value) => store.save_stage_success(lease, row.id, value).await?,
                Err(error) => {
                    store
                        .fail_stage(lease, row.id, &format!("{error:#}"), RETRY_DELAY)
                        .await?;
                    return Err(error);
                }
            }
            return Ok(());
        }
        if matches!(
            row.stage.as_str(),
            "delete_owned_community" | "gate_user" | "global_cleanup" | "purge_profile"
        ) {
            return handle_remaining_stage(store, lease, row, options, subject).await;
        }
        bail!("real handler {} is not implemented", row.stage);
    }
    match row.stage.as_str() {
        "purge_media" => dummy_action(store, lease, row, options, "would purge user media").await,
        "gate_user" => gate_user(store, lease, row, options).await,
        "purge_content" => {
            dummy_action(store, lease, row, options, "would purge user content").await
        }
        "purge_agents" => purge_agents(store, lease, row, options).await,
        name => bail!("unknown workflow stage {name}"),
    }
}

async fn purge_media(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
    subject: &str,
) -> Result<()> {
    store.start_stage(lease, row.id).await?;
    let result = async {
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
    .await;
    match result {
        Ok(result) => store.save_stage_success(lease, row.id, result).await?,
        Err(error) => {
            store
                .fail_stage(lease, row.id, &format!("{error:#}"), RETRY_DELAY)
                .await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn gate_user(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<()> {
    dummy_action(store, lease, row, options, "would gate user").await
}

async fn purge_content(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
    subject: &str,
) -> Result<()> {
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
    store.start_stage(lease, row.id).await?;
    let result = async {
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
        Ok::<_, anyhow::Error>(
            serde_json::json!({"content": summary, "account_erasure_complete": false}),
        )
    }
    .await;
    match result {
        Ok(result) => store.save_stage_success(lease, row.id, result).await?,
        Err(error) => {
            store
                .fail_stage(lease, row.id, &format!("{error:#}"), crate::RETRY_DELAY)
                .await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn handle_remaining_stage(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
    subject: &str,
) -> Result<()> {
    if options.delete && row.stage != "delete_owned_community" {
        bail!("real handler {} is not implemented for deletion", row.stage);
    }
    let db = options.db.as_ref().context("missing source database")?;
    let row = store.start_stage(lease, row.id).await?;
    let result: Result<_> = async {
        match row.stage.as_str() {
            "delete_owned_community" => {
                let request = store.get(lease.request_id).await?;
                let configured;
                let storage = match options.media.as_deref() {
                    Some(storage) => storage,
                    None => { configured = super::media::configured_storage()?; &configured }
                };
                let metadata = super::owned_community::run_stage(&request, &row, &db.deletion_store(), storage).await?;
                if let Some(ids) = options.ids(&row) {
                    ids.record(serde_json::json!({"child_request_id":metadata["child_request_id"],"action":"delegate"}))?;
                }
                Ok(metadata)
            }
            "purge_profile" => {
                let summary = db.user_deletion_store().preview_profile_plan(&ProfileBatchOptions {
                    community: row.target_community_id.context("missing profile community")?,
                    pubkey: subject.into(), relay_pubkeys: options.relay_pubkey.iter().cloned().collect(),
                    batch_size: options.batch_size, max_items: options.max_items,
                }, &mut options.batch_output(&row)).await?;
                Ok(serde_json::to_value(summary)?)
            }
            _ => Ok(serde_json::json!({"execution_mode":"read-only","implemented":false,"account_erasure_complete":false})),
        }
    }.await;
    match result {
        Ok(metadata) => store.save_stage_success(lease, row.id, metadata).await?,
        Err(error) => {
            if let Some(child) = error.downcast_ref::<super::owned_community::BlockedChild>() {
                store
                    .save_stage_waiting(lease, row.id, child.metadata.clone())
                    .await?;
            }
            store
                .fail_stage(lease, row.id, &format!("{error:#}"), RETRY_DELAY)
                .await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn purge_agents(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<()> {
    dummy_action(store, lease, row, options, "would purge user agents").await
}

async fn dummy_action(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
    message: &str,
) -> Result<()> {
    store.start_stage(lease, row.id).await?;
    let result = perform(row, options, message).await;
    match result {
        Ok(result) => store.save_stage_success(lease, row.id, result).await?,
        Err(error) => {
            eprintln!("dummy stage {} failed: {error:#}", row.stage);
            // Ok means the outcome is durable, not that the action succeeded.
            // The transition function will reload this row and return WaitUntil.
            store
                .fail_stage(lease, row.id, &format!("{error:#}"), RETRY_DELAY)
                .await?;
        }
    }
    Ok(())
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
