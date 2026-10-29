//! Request summaries from a single user-table snapshot. No child API or
//! community deletion tables are consulted; observations can therefore be stale.

use buzz_db::entity_deletion::{EntityDeletionStore, EntityInspection, EntityStage};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// Saved workflow progress, suitable for an operator report or future handler.
#[derive(Debug, Serialize)]
pub struct RequestSummary {
    /// The authoritative request, lease/retry state, approvals and stage rows.
    pub saved: EntityInspection,
    /// True only if the parent is complete and every reached stage is complete.
    pub complete: bool,
    /// Completed reached stages; this is not a count of all future required work.
    pub completed_stages: usize,
    /// Reached stages still waiting, failed, pending, or running.
    pub unfinished_stages: Vec<UnfinishedStage>,
    /// Inventory communities that have no stage row yet. Empty does not prove
    /// all work is done: runtime routing can still create additional stages.
    pub communities_not_reached: Vec<Value>,
    /// Clarifies whether more execution or an external dependency is outstanding.
    pub explanation: &'static str,
}

/// One unfinished stage with enough saved context to act without a child lookup.
#[derive(Debug, Serialize)]
pub struct UnfinishedStage {
    /// Exact stage row, including community, inputs, timestamps, and errors.
    pub stage: EntityStage,
    /// Display state: failed, pending, running, or waiting_external.
    pub state: String,
    /// Last saved external observation, including reason, child ID/status,
    /// observation time, operator action and command argument arrays when known.
    pub external_observation: Option<Value>,
    /// Whether invoking the handler again is needed to refresh external status.
    pub needs_external_refresh: bool,
}

/// Standalone read-only summary handler. It never submits/checks children,
/// changes a stage, claims a request, or declares completion from child status.
pub async fn summarize_request(
    store: &EntityDeletionStore,
    request_id: Uuid,
) -> buzz_db::Result<RequestSummary> {
    let saved = store.inspect(request_id).await?;
    let completed_stages = saved
        .stage_history
        .iter()
        .filter(|row| row.status == "completed")
        .count();
    let unfinished_stages: Vec<_> = saved
        .stage_history
        .iter()
        .filter(|row| row.status != "completed")
        .map(|row| {
            let observation = row.metadata_json.get("waiting").cloned();
            UnfinishedStage {
                stage: row.clone(),
                state: if row.status == "running" && observation.is_some() {
                    "waiting_external".into()
                } else {
                    row.status.clone()
                },
                needs_external_refresh: observation.is_some(),
                external_observation: observation,
            }
        })
        .collect();
    let communities_not_reached = saved
        .request
        .inventory_manifest
        .as_ref()
        .and_then(|inventory| inventory["communities"].as_array())
        .into_iter()
        .flatten()
        .filter(|community| {
            !saved.stage_history.iter().any(|row| {
                row.target_community_id
                    .is_some_and(|id| community["community_id"] == id.to_string())
            })
        })
        .cloned()
        .collect();
    let complete = saved.request.status == "completed" && unfinished_stages.is_empty();
    let explanation = if complete {
        "The parent request is complete and every reached stage is complete. Any retained-copy obligations remain in the saved results."
    } else if unfinished_stages
        .iter()
        .any(|row| row.needs_external_refresh)
    {
        "The request is unfinished. External statuses are last-observed snapshots, not live checks. Revisit those stage handlers to refresh them; this summary performs no external work."
    } else {
        "The request is unfinished. Inspect its approval, retry and stage state. Completed reached stages do not prove that all workflow steps have been reached."
    };
    Ok(RequestSummary {
        saved,
        complete,
        completed_stages,
        unfinished_stages,
        communities_not_reached,
        explanation,
    })
}
