//! Final account deletion after an operator verifies cleanup for this request.
use super::{handlers::DispatchOutcome, worker::RunOptions};
use anyhow::{ensure, Context, Result};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityStage};
use buzz_db::user_deletion::UserDeletionInventory;
use serde_json::{json, Value};

pub(super) async fn handle(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<DispatchOutcome> {
    let request = store.get(lease.request_id).await?;
    ensure!(
        options.delete
            && !request.target.starts_with("demo:")
            && row.request_id == request.id
            && row.target_community_id.is_none()
            && row.metadata_json["inputs"]
                == UserDeletionInventory::read(&request)?.inputs(&request.target, None)?,
        "finalization differs from approved request"
    );
    // A prior process may have sent a destructive request before losing its
    // response or lease. The service does not deduplicate by request ID.
    if let Some(saved) = row.metadata_json.get("waiting") {
        ensure!(
            saved["reason"] == "awaiting_verified_buzz_cleanup",
            "inspect the external deletion outcome before proceeding: {}",
            saved["reason"]
        );
    } else {
        ensure!(
            row.status == "pending",
            "inspect the previous account deletion attempt before proceeding"
        );
    }
    if !options.cleanup_verified {
        store.start_stage(lease, row.id).await?;
        store
            .save_stage_waiting(
                lease,
                row.id,
                json!({"reason":"awaiting_verified_buzz_cleanup"}),
            )
            .await?;
        return Ok(DispatchOutcome::Waiting);
    }
    let identity = options
        .identity
        .as_ref()
        .context("finalization requires configured identity URLs and token file")?;
    let body = json!({"request_id":request.id,"pubkey":request.target,"cleanup_verified":true});
    let login_request = identity.prepare_delete_login(&body).await?;
    let account_request = identity.prepare_delete_account(&body).await?;
    store.start_stage(lease, row.id).await?;
    store
        .save_stage_waiting(
            lease,
            row.id,
            json!({"reason":"inspect_login_deletion_outcome"}),
        )
        .await?;
    let login = identity.send(login_request).await?;
    ensure!(
        matches_receipt(&login, request.id, &request.target, "auth0_deleted"),
        "login deletion unconfirmed; inspect the external login"
    );
    store
        .save_stage_waiting(
            lease,
            row.id,
            json!({"reason":"inspect_account_deletion_outcome","login_receipt":login}),
        )
        .await?;
    let account = identity.send(account_request).await?;
    ensure!(
        matches_receipt(&account, request.id, &request.target, "account_deleted"),
        "account deletion unconfirmed; inspect the local account"
    );
    store
        .save_stage_success(
            lease,
            row.id,
            json!({"login_receipt":login,"account_receipt":account}),
        )
        .await?;
    Ok(DispatchOutcome::Completed)
}

pub(super) fn confirmed(receipt: &Value, id: uuid::Uuid, target: &str) -> bool {
    matches_receipt(&receipt["login_receipt"], id, target, "auth0_deleted")
        && matches_receipt(&receipt["account_receipt"], id, target, "account_deleted")
}

fn matches_receipt(receipt: &Value, id: uuid::Uuid, target: &str, flag: &str) -> bool {
    receipt["request_id"] == json!(id) && receipt["pubkey"] == target && receipt[flag] == true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_receipt_cannot_complete_separate_deletions() {
        let id = uuid::Uuid::new_v4();
        let combined =
            json!({"request_id":id,"pubkey":"user","auth0_deleted":true,"account_deleted":true});
        assert!(!confirmed(&combined, id, "user"));
    }
}
