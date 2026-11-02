//! Standalone owned-community handler; deliberately absent from dispatch and
//! transition rules. The caller owns claim, heartbeat, and release lifecycle.

use crate::community::submit_and_prepare;
use crate::RETRY_DELAY;
use anyhow::{bail, Context, Result};
use buzz_core::CommunityId;
use buzz_db::deletion::{DeletionRequest, DeletionRequestOrigin, DeletionStage, DeletionStore};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityStage};
use buzz_media::MediaStorage;
use serde_json::{json, Value};

/// Durable outcome of one observation; waiting is not successful completion.
#[derive(Debug, PartialEq, Eq)]
pub enum OwnedCommunityOutcome {
    /// The child still needs preparation, approval, execution, or intervention.
    Waiting,
    /// Logical community deletion finished; physical retention can remain.
    Completed,
}

/// Check the child first, submit only if missing, and save the result on this
/// user stage. Errors propagate after recording a retry when the lease is valid.
/// This never approves or runs the child, advances stages, or releases the lease.
pub async fn handle(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    community_store: &DeletionStore,
    media: &MediaStorage,
) -> Result<OwnedCommunityOutcome> {
    if row.request_id != lease.request_id || row.stage != "delete_owned_community" {
        bail!("expected this request's delete_owned_community stage");
    }
    let row = store.start_stage(lease, row.id).await?;
    match observe(store, lease, &row, community_store, media).await {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            if let Err(save_error) = store
                .fail_stage(lease, row.id, &format!("{error:#}"), RETRY_DELAY)
                .await
            {
                return Err(error.context(format!("also could not save retry: {save_error}")));
            }
            Err(error)
        }
    }
}

async fn observe(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    community_store: &DeletionStore,
    media: &MediaStorage,
) -> Result<OwnedCommunityOutcome> {
    let request = store.get(lease.request_id).await?;
    let community_id = row
        .target_community_id
        .context("missing owned community ID")?;
    let inputs = &row.metadata_json["inputs"];
    let inventory = request
        .inventory_manifest
        .as_ref()
        .context("missing inventory")?;
    let approved_community = inventory["communities"]
        .as_array()
        .context("missing inventory communities")?
        .iter()
        .find(|community| community["community_id"] == community_id.to_string())
        .context("owned community is outside approved inventory")?;
    if request.entity_type != "user"
        || row.stage != "delete_owned_community"
        || inputs["target"] != request.target
        || &inputs["community"] != approved_community
        || approved_community["relationship"] != "owner"
    {
        bail!("stage inputs do not match the approved owned-community inventory");
    }
    let community = CommunityId::from_uuid(community_id);
    let saved_child = row.metadata_json["waiting"]["child_request_id"].as_str();
    let child = match community_store.find_for_community(community).await? {
        Some(child) => child,
        None if saved_child.is_some() => bail!("previously observed community request is missing"),
        None => {
            submit_and_prepare(
                community_store,
                media,
                None,
                &request.requested_by,
                Some(&format!(
                    "User deletion request {}; stage {}",
                    request.id, row.id
                )),
                Some(community),
            )
            .await?
        }
    };
    // The stage tracks the approved community, not one deletion attempt. A
    // replacement request returned by community-ID lookup becomes its current
    // observation without changing or approving the child.
    let completed = child.stage == DeletionStage::RetentionPending
        && child.completed_at.is_some()
        && child.blocked_reason.is_none();
    let observation = observation(&child, completed);
    if completed {
        store.save_stage_success(lease, row.id, observation).await?;
        Ok(OwnedCommunityOutcome::Completed)
    } else {
        store.save_stage_waiting(lease, row.id, observation).await?;
        Ok(OwnedCommunityOutcome::Waiting)
    }
}

fn observation(child: &DeletionRequest, completed: bool) -> Value {
    let id = child.id.to_string();
    let inspect = json!(["buzz-admin", "deletions", "inspect", id]);
    let (reason, action, commands) = if completed {
        (
            "logical_deletion_complete",
            "Community logical deletion is complete. Retained copies remain subject to community retention policy.",
            vec![inspect],
        )
    } else if child.stage == DeletionStage::Aborted {
        (
            "operator_intervention",
            "Inspect the aborted child. To continue deletion, submit a replacement for this community; the user stage will follow it on the next invocation.",
            vec![inspect, json!(["buzz-admin", "deletions", "submit", "--host", child.community_host, "--requested-by", "<operator>"])],
        )
    } else if child.blocked_reason.is_some() {
        (
            "operator_intervention",
            "Inspect and fix the recorded cause, then unblock the community deletion request.",
            vec![
                inspect,
                json!([
                    "buzz-admin",
                    "deletions",
                    "unblock",
                    id,
                    "--unblocked-by",
                    "<operator>",
                    "--reason",
                    "<remediation>"
                ]),
            ],
        )
    } else {
        match child.stage {
            DeletionStage::Submitted if child.request_origin == DeletionRequestOrigin::Owner => (
                "owner_preparation_pending",
                "Let the community deletion drain prepare and approve the existing owner-origin request after validating its recorded owner intent.",
                vec![inspect],
            ),
            DeletionStage::Submitted => (
                "preparation_required",
                "Finish community inventory preparation using the original submitter, then obtain separate community approval.",
                vec![inspect, json!(["buzz-admin", "deletions", "submit", "--host", child.community_host, "--requested-by", child.requested_by])],
            ),
            DeletionStage::Inventoried => (
                "approval_required",
                "Review the child's frozen inventory, then have an operator approve the community deletion separately.",
                vec![inspect, json!(["buzz-admin", "deletions", "approve", id, "--approved-by", "<operator>"])],
            ),
            _ => (
                "community_execution_pending",
                "Community deletion runs independently through its worker/drain or an operator run. Invoke this user-stage handler again after it progresses.",
                vec![inspect, json!(["buzz-admin", "deletions", "run", id])],
            ),
        }
    };
    json!({
        "kind": "community_deletion",
        "child_request_id": child.id,
        "community_id": child.community_id.as_uuid(),
        "community_host": child.community_host,
        "child_stage": child.stage,
        "child_updated_at": child.updated_at,
        "observed_at": chrono::Utc::now(),
        "inventory_digest": child.inventory_digest,
        "blocked_reason": child.blocked_reason,
        "last_error": child.last_error,
        "next_attempt_at": child.next_attempt_at,
        "completed_at": child.completed_at,
        "reason": reason,
        "operator_action": action,
        "commands": commands,
        "retention_pending": child.stage == DeletionStage::RetentionPending,
    })
}

#[cfg(test)]
mod postgres_tests;
