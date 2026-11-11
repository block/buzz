//! Workflow handlers validate saved scope and persist outcomes; they never route.

use super::worker::RunOptions;
use crate::RETRY_DELAY;
use anyhow::{bail, ensure, Context, Result};
use buzz_db::{
    entity_deletion::{EntityDeletionStore, EntityLease, EntityStage},
    user_deletion::{content_batches::ContentBatchOptions, ExecutionMode, UserDeletionInventory},
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
        .unwrap_or_default();
    if !target.starts_with("demo:") {
        return match row.stage.as_str() {
            "purge_content" => purge_content(store, lease, row, options).await,
            name => bail!("real handler {name} is not implemented"),
        };
    }
    match row.stage.as_str() {
        "gate_user" => gate_user(store, lease, row, options).await,
        "purge_content" => {
            dummy_action(store, lease, row, options, "would purge user content").await
        }
        "purge_agents" => purge_agents(store, lease, row, options).await,
        name => bail!("unknown dummy stage {name}"),
    }
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
) -> Result<()> {
    ensure!(
        options.delete,
        "real workflow execution requires --delete; use content-batches --read-only to inspect"
    );
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
                    pubkey: request.target.clone(),
                    relay_pubkey: options.relay_pubkey.clone(),
                    batch_size: options.batch_size,
                    max_items: options.max_items,
                },
                ExecutionMode::Delete,
                Some((lease, row.id)),
                &mut std::io::stdout(),
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
