//! User-deletion routing. The store owns the lock and atomic writes; this module
//! interprets the saved user inventory, history, and handler results.
use super::workflow::{decision, invalid, Progress, Step, INITIAL_STAGE};
use buzz_db::entity_deletion::{
    EntityDeletionStore, EntityLease, EntityRequest, EntityStage, NewEntityStage, NextStage,
    StageTransition,
};
use buzz_db::user_deletion::UserDeletionInventory as Inventory;
use buzz_db::Result;
use uuid::Uuid;

pub(super) struct UserDeletionTransition<'a> {
    store: &'a EntityDeletionStore,
}

impl<'a> UserDeletionTransition<'a> {
    pub fn new(store: &'a EntityDeletionStore) -> Self {
        Self { store }
    }

    /// On restart, None recovers the current position. After a handler returns,
    /// pass its ID; all decisions still use freshly loaded, locked database rows.
    pub async fn get_next_stage(
        &self,
        lease: &EntityLease,
        previous: Option<Uuid>,
    ) -> Result<NextStage> {
        self.store
            .transition_stage(lease, previous, decide_next)
            .await
    }
}

fn decide_next(request: &EntityRequest, history: &[EntityStage]) -> Result<StageTransition> {
    let inventory = Inventory::read(request)?;
    let mut progress = Progress::rebuild(request, &inventory, history)?;
    if request.status == "completed" {
        if !progress.terminal || progress.current.is_some() {
            return Err(invalid("completed request has inconsistent stage history"));
        }
        return Ok(StageTransition::Finished);
    }
    let Some(current) = progress.current.take() else {
        if request.status != "approved" || !history.is_empty() {
            return Err(invalid(
                "started request has no current stage or terminal outcome",
            ));
        }
        return Ok(StageTransition::Initialize(saved_stage(
            request,
            &inventory,
            Step {
                name: INITIAL_STAGE,
                community: None,
            },
        )?));
    };
    match current.status.as_str() {
        "pending" | "running" | "failed" => Ok(StageTransition::Resume(current.id)),
        "completed" => {
            let outcome = decision(&current)?;
            let successor = progress.apply(&inventory, &current, outcome)?;
            if successor.is_none()
                && progress.completed_communities.len() != inventory.communities.len()
            {
                return Err(invalid("terminal transition leaves uncovered communities"));
            }
            Ok(StageTransition::Advance {
                previous: current.id,
                decision: serde_json::to_value(outcome)?,
                successor: successor
                    .map(|step| saved_stage(request, &inventory, step))
                    .transpose()?,
            })
        }
        _ => Err(invalid("unknown current stage status")),
    }
}

fn saved_stage(
    request: &EntityRequest,
    inventory: &Inventory,
    step: Step,
) -> Result<NewEntityStage> {
    Ok(NewEntityStage {
        stage: step.name.to_owned(),
        community: step.community,
        inputs: inventory.inputs(&request.target, step.community)?,
    })
}

#[cfg(test)]
#[path = "transition_postgres_tests.rs"]
mod postgres_tests;
