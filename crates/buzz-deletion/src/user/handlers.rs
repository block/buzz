//! Dispatch real account cleanup or demo handlers; transitions own routing.

use super::worker::RunOptions;
use crate::RETRY_DELAY;
use anyhow::{bail, ensure, Context, Result};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityStage};
use buzz_db::user_deletion::{account_batches::AccountBatchOptions, ExecutionMode};
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
        if row.stage == "purge_account" {
            ensure!(options.delete, "real workflow execution requires --delete");
            let db = options.db.as_ref().context("missing account database")?;
            let row = store.start_stage(lease, row.id).await?;
            let result = async {
                let target = row.metadata_json["inputs"]["target"]
                    .as_str()
                    .context("missing target")?;
                let community = row
                    .target_community_id
                    .context("missing account community")?;
                let summary = super::account_batches::run(
                    db,
                    &AccountBatchOptions {
                        community,
                        pubkey: target.to_owned(),
                        batch_size: options.batch_size,
                        max_items: options.max_items,
                    },
                    ExecutionMode::Delete,
                    Some((lease, row.id)),
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
        bail!(
            "real handler {} is not implemented in this account-cleanup PR",
            row.stage
        );
    }
    match row.stage.as_str() {
        "gate_user" => gate_user(store, lease, row, options).await,
        "purge_db" => purge_db(store, lease, row, options).await,
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

async fn purge_db(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<()> {
    dummy_action(store, lease, row, options, "would purge user database rows").await
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
    if row.stage == "purge_db" {
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
