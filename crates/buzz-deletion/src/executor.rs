//! Shared request selection, execution, heartbeats, cancellation and recovery.
//! Dispatch is explicit; each store owns its transaction and safety semantics.

use super::*;
use buzz_db::entity_deletion::{EntityClaim, EntityRequest, StageRecord};

#[derive(Clone)]
pub(super) enum WorkflowServices {
    Community(Services),
    User(user::Services),
}

impl From<Services> for WorkflowServices {
    fn from(services: Services) -> Self {
        Self::Community(services)
    }
}

impl From<user::Services> for WorkflowServices {
    fn from(services: user::Services) -> Self {
        Self::User(services)
    }
}

#[derive(Clone)]
enum Execution {
    Community {
        services: Services,
        claim: ClaimedDeletion,
    },
    User {
        services: user::Services,
        claim: EntityClaim,
        stage: StageRecord,
    },
}

enum Selection {
    Ready(Box<Execution>),
    Empty,
    Finished(i32),
}

#[cfg(test)]
pub(super) async fn execute_claim(
    services: &Services,
    mode: LoopMode,
    claim: ClaimedDeletion,
    shutdown: &CancellationToken,
) -> Result<RunOutput> {
    let mut execution = Execution::Community {
        services: services.clone(),
        claim,
    };
    match execution.run(mode, shutdown).await? {
        Output::Community(output) => Ok(output),
        Output::User { .. } => anyhow::bail!("community execution returned a user result"),
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum Output {
    Community(RunOutput),
    User {
        request: Box<EntityRequest>,
        demonstration: &'static str,
    },
}

impl Output {
    fn failed(&self) -> bool {
        match self {
            Self::Community(output) => {
                output.last_error.is_some() || output.blocked_reason.is_some()
            }
            Self::User { request, .. } => {
                request.last_error.is_some() || request.blocked_reason.is_some()
            }
        }
    }
}

pub(super) async fn run_loop(
    services: impl Into<WorkflowServices>,
    mode: LoopMode,
    request_id: Option<Uuid>,
    executor_id: String,
) -> Result<i32> {
    let services = services.into();
    let shutdown = shutdown_token();
    loop {
        if shutdown.is_cancelled() {
            services.stop(&executor_id).await?;
            return Ok(0);
        }
        let mut execution = match services
            .claim(mode, request_id, &executor_id, &shutdown)
            .await?
        {
            Selection::Ready(execution) => execution,
            Selection::Finished(code) => return Ok(code),
            Selection::Empty => {
                if mode == LoopMode::Run {
                    anyhow::bail!("deletion request is not runnable, is blocked, or is leased by another executor");
                }
                return Ok(0);
            }
        };
        let output = execution.run(mode, &shutdown).await?;
        print_json(&output)?;
        let failed = output.failed();
        let stopped = execution.stop_requested();
        if mode == LoopMode::Run || shutdown.is_cancelled() || failed || stopped {
            return Ok(i32::from(failed));
        }
    }
}

impl WorkflowServices {
    async fn stop(&self, owner: &str) -> Result<()> {
        match self {
            Self::Community(s) => s.store.stop_executor(None, owner).await?,
            Self::User(s) => s.store.stop_executor(None, owner).await?,
        }
        Ok(())
    }

    async fn claim(
        &self,
        mode: LoopMode,
        id: Option<Uuid>,
        owner: &str,
        shutdown: &CancellationToken,
    ) -> Result<Selection> {
        match self {
            Self::User(services) => {
                let Some(claim) = services
                    .store
                    .claim(id, owner, DEFAULT_LEASE_DURATION)
                    .await?
                else {
                    return Ok(Selection::Empty);
                };
                let stage = services.store.current_stage(claim.request.id).await?;
                Ok(Selection::Ready(Box::new(Execution::User {
                    services: services.clone(),
                    claim,
                    stage,
                })))
            }
            Self::Community(services) => {
                let claim = match id {
                    Some(id) => {
                        services
                            .store
                            .claim_specific(id, owner, DEFAULT_LEASE_DURATION)
                            .await?
                    }
                    None => {
                        services
                            .store
                            .claim_next(owner, DEFAULT_LEASE_DURATION)
                            .await?
                    }
                };
                let claim = if claim.is_none() && mode == LoopMode::Drain && id.is_none() {
                    services
                        .store
                        .claim_next_owner_submission(owner, DEFAULT_LEASE_DURATION)
                        .await?
                } else {
                    claim
                };
                let Some(mut claim) = claim else {
                    return Ok(Selection::Empty);
                };
                if claim.request.stage == DeletionStage::Submitted {
                    let request = claim.request.clone();
                    match prepare_owner_claim_with(
                        services, mode, claim, shutdown, HEARTBEAT_INTERVAL,
                        |lost| async move { build_inventory(services, &request, Some(&lost)).await },
                    ).await? {
                        OwnerPreparationOutcome::Prepared(prepared) => claim = *prepared,
                        OwnerPreparationOutcome::Stopped => return Ok(Selection::Finished(0)),
                        OwnerPreparationOutcome::Failed(output) => {
                            // Preserve the original drain's nonzero exit and JSON evidence.
                            print_json(&output)?;
                            return Ok(Selection::Finished(1));
                        }
                    }
                }
                Ok(Selection::Ready(Box::new(Execution::Community {
                    services: services.clone(),
                    claim,
                })))
            }
        }
    }
}

impl Execution {
    async fn run(&mut self, mode: LoopMode, shutdown: &CancellationToken) -> Result<Output> {
        let mut completed = 0u16;
        loop {
            if shutdown.is_cancelled() {
                self.stop(mode).await?;
                return self.output().await;
            }
            self.heartbeat(mode).await?;
            match self.run_stage(mode, shutdown).await {
                StageOutcome::Completed => {}
                StageOutcome::Shutdown => {
                    self.stop(mode).await?;
                    return self.output().await;
                }
                StageOutcome::Failed(error) => {
                    self.record_failure(&error).await?;
                    return self.output().await;
                }
            }
            completed += 1;
            if self.refresh().await? {
                if let Self::User {
                    services, claim, ..
                } = self
                {
                    services
                        .store
                        .stop_executor(Some(&claim.lease), &claim.lease.owner)
                        .await?;
                }
                return self.output().await;
            }
            if let Self::User { services, .. } = self {
                if services
                    .controls
                    .stop_after
                    .is_some_and(|limit| completed >= limit)
                {
                    self.stop(mode).await?;
                    return self.output().await;
                }
            }
        }
    }

    fn stop_requested(&self) -> bool {
        matches!(self, Self::User { services, .. } if services.controls.stop_after.is_some())
    }

    async fn heartbeat(&self, mode: LoopMode) -> Result<()> {
        match self {
            Self::Community { services, claim } => {
                services
                    .store
                    .heartbeat(&claim.lease, mode.as_str(), DEFAULT_LEASE_DURATION, false)
                    .await?;
            }
            Self::User {
                services, claim, ..
            } => {
                services
                    .store
                    .heartbeat(&claim.lease, mode.as_str(), DEFAULT_LEASE_DURATION)
                    .await?;
            }
        }
        Ok(())
    }

    async fn stop(&self, mode: LoopMode) -> Result<()> {
        match self {
            Self::Community { services, claim } => {
                stop_claim_executor(services, mode, &claim.lease).await?
            }
            Self::User {
                services, claim, ..
            } => {
                services
                    .store
                    .stop_executor(Some(&claim.lease), &claim.lease.owner)
                    .await?;
            }
        }
        Ok(())
    }

    async fn output(&self) -> Result<Output> {
        match self {
            Self::Community { services, claim } => Ok(Output::Community(run_output(
                services.store.get(claim.request.id).await?,
            ))),
            Self::User {
                services, claim, ..
            } => Ok(Output::User {
                request: Box::new(services.store.get(claim.request.id).await?),
                demonstration: "Synthetic workflow only; no user data was deleted.",
            }),
        }
    }

    async fn refresh(&mut self) -> Result<bool> {
        match self {
            Self::Community { services, claim } => {
                let request = services.store.get(claim.request.id).await?;
                let terminal = request.stage == DeletionStage::RetentionPending
                    || request.blocked_reason.is_some();
                claim.lease.fence_generation = request.fence_generation;
                claim.request = request;
                Ok(terminal)
            }
            Self::User {
                services,
                claim,
                stage,
            } => {
                claim.request = services.store.get(claim.request.id).await?;
                let terminal =
                    claim.request.status == "demo_completed" || claim.request.blocked_at.is_some();
                if !terminal {
                    *stage = services.store.current_stage(claim.request.id).await?;
                }
                Ok(terminal)
            }
        }
    }

    async fn record_failure(&self, error: &anyhow::Error) -> Result<()> {
        match self {
            Self::Community { services, claim } => {
                let request = services.store.get(claim.request.id).await?;
                if request.lease_owner.as_deref() == Some(&claim.lease.owner)
                    && request.lease_generation == claim.lease.generation
                {
                    record_stage_failure(services, &claim.lease, claim.request.stage, error)
                        .await?;
                }
            }
            Self::User {
                services,
                claim,
                stage,
            } => {
                match services
                    .store
                    .record_failure(
                        &claim.lease,
                        stage.id,
                        &format!("{error:#}"),
                        is_permanent_error(error),
                        RETRY_DELAY,
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(error) if buzz_db::entity_deletion::is_stale_lease(&error) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    async fn run_stage(&self, mode: LoopMode, shutdown: &CancellationToken) -> StageOutcome {
        let heartbeat_execution = self.clone();
        let cancel = CancellationToken::new();
        let heartbeat_cancel = cancel.clone();
        let lost = CancellationToken::new();
        let heartbeat_lost = lost.clone();
        let heartbeat = tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_interval());
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = heartbeat_cancel.cancelled() => return,
                    _ = interval.tick() => {
                        if heartbeat_execution.heartbeat(mode).await.is_err() {
                            heartbeat_lost.cancel();
                            return;
                        }
                    }
                }
            }
        });
        let operation = async {
            match self {
                Self::Community { services, claim } => execute_stage(services, claim, &lost).await,
                Self::User {
                    services,
                    claim,
                    stage,
                } => user::execute_stage(services, claim, stage, &lost).await,
            }
        };
        let result = await_stage(operation, shutdown, &lost).await;
        cancel.cancel();
        match heartbeat.await {
            Ok(()) => result,
            Err(error) => {
                StageOutcome::Failed(anyhow::anyhow!("deletion heartbeat task failed: {error}"))
            }
        }
    }
}
