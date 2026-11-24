//! Request-wide identity revocation with a durable success receipt.

use anyhow::{ensure, Context, Result};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityRequest, EntityStage};
use buzz_db::user_deletion::UserDeletionInventory;
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::worker::RunOptions;

mod transport;
pub(super) use transport::IdentityGateConfig;

#[derive(Deserialize, serde::Serialize)]
struct Receipt {
    request_id: Uuid,
    pubkey: String,
    identity_revoked: bool,
    identity_blocked: bool,
    revoked_at: DateTime<FixedOffset>,
}

fn receipt(value: &Value, request_id: Uuid, pubkey: &str) -> Result<Receipt> {
    let receipt: Receipt = serde_json::from_value(value.clone())
        .map_err(|_| anyhow::anyhow!("invalid identity revocation receipt"))?;
    ensure!(
        receipt.request_id == request_id
            && receipt.pubkey == pubkey
            && receipt.identity_revoked
            && receipt.identity_blocked,
        "identity revocation receipt does not match this request"
    );
    // Parsing the timestamp validates the protocol; it is not cleanup readiness.
    Ok(receipt)
}

pub(super) async fn handle(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    request: &EntityRequest,
    options: &RunOptions,
) -> Result<()> {
    ensure!(options.delete, "real workflow execution requires --delete");
    let inventory = UserDeletionInventory::read(request)?;
    ensure!(
        row.request_id == request.id
            && row.stage == "gate_user"
            && row.target_community_id.is_none()
            && row.metadata_json["inputs"] == inventory.inputs(&request.target, None)?,
        "identity gate differs from approved request-wide inventory"
    );
    if let Some(saved) = row.metadata_json.get("waiting") {
        ensure!(
            saved["reason"] != "inspect_block_outcome",
            "inspect the previous block outcome before proceeding"
        );
        let saved_receipt = saved
            .get("identity_receipt")
            .context("missing saved identity receipt")?;
        receipt(saved_receipt, request.id, &request.target)?;
        return Ok(());
    }
    ensure!(
        row.status == "pending",
        "inspect the previous block outcome before proceeding"
    );
    let config = options
        .identity
        .as_ref()
        .context("identity gate requires configured identity URLs")?;
    let outbound =
        config.prepare_block(&json!({"request_id":request.id,"pubkey":request.target}))?;
    store.start_stage(lease, row.id).await?;
    store
        .save_stage_waiting(lease, row.id, json!({"reason":"inspect_block_outcome"}))
        .await?;
    let revoked = receipt(&config.send(outbound).await?, request.id, &request.target)?;
    store
        .save_stage_waiting(
            lease,
            row.id,
            json!({
                "identity_receipt": revoked,
                "reason": "awaiting_access_enforcement_and_writer_drain",
            }),
        )
        .await?;
    // Identity revocation alone cannot certify Buzz's writer drain or relay enforcement.
    // Keep this stage current; a future readiness integration must supply that evidence.
    Ok(())
}

#[cfg(test)]
#[path = "gate_postgres_tests.rs"]
mod postgres_tests;
