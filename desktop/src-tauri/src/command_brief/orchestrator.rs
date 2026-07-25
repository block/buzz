use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_core_pkg::command_brief::{CommandBriefFailureCode, CommandBriefLifecycleState};
use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

use super::audit::{PersistedTerminal, TerminalAuditInput};
use super::lmstudio::{
    AdviserExecutionError, AdviserExecutionErrorCode, AdviserExecutor, ChiefOfStaffRequest,
    SpecialistAdviserRequest,
};
use super::provenance::ValidatedSource;
use super::scheduler::{LocalModelScheduler, SchedulerError, SchedulerJobKey};
use super::sources::{
    FrozenSourceContext, ProductionSourceBackend, SourceBackend, SourceCollectionError,
    SourceCollector,
};
use super::types::{
    AdviserContribution, AdviserId, BriefRunState, BriefRunStatus, BriefSection, CitedFinding,
    Classification, CommandBrief, PublishedCommandBrief, MAX_AGGREGATE_DISSENT_ITEMS,
    MAX_ARRAY_ITEMS, MAX_TEXT_BYTES, SPECIALIST_ADVISERS,
};
use crate::command_services::apple_inputs::AppleBriefSelection;

mod lifecycle;
mod runtime;
pub(crate) use runtime::OrchestratorAdmissionState;
pub use runtime::OrchestratorStartError;
const MAX_CO_REQUEST_BYTES: usize = 1024;
const MAX_SCHEDULE_ID_BYTES: usize = 256;
const MAX_STATUS_HISTORY: usize = 32;
const MAX_TRACKED_RUNS: usize = 64;

/// A boxed, cancellation-aware future used by the orchestration seams.
pub(crate) type BriefFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Stable adviser failure classes that never carry prompts, evidence, or provider bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BriefAdviserError {
    Cancelled,
    Failed,
}

impl From<AdviserExecutionError> for BriefAdviserError {
    fn from(error: AdviserExecutionError) -> Self {
        if error.code() == AdviserExecutionErrorCode::Cancelled {
            Self::Cancelled
        } else {
            Self::Failed
        }
    }
}

/// Stable persistence failures used until Task 6 installs the encrypted store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BriefPersistenceError {
    #[allow(
        dead_code,
        reason = "Task 6's encrypted persistence implementation returns this stable failure"
    )]
    Cancelled,
    #[allow(
        dead_code,
        reason = "Task 6's encrypted persistence implementation returns this stable failure"
    )]
    Failed,
}

/// Native source collection seam. Implementations must return one frozen local context.
pub(crate) trait BriefSourceProvider: Send + Sync {
    fn freeze<'a>(
        &'a self,
        run_id: &'a str,
        co_request: &'a str,
        observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>>;

    fn recheck<'a>(
        &'a self,
        context: &'a FrozenSourceContext,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>>;
}

/// Native adviser seam. Its Chief method accepts no integration/tool parameter.
pub(crate) trait BriefAdviserProvider: Send + Sync {
    fn run_specialist<'a>(
        &'a self,
        run_id: &'a str,
        adviser: AdviserId,
        sources: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<AdviserContribution, BriefAdviserError>>;

    fn run_chief_of_staff<'a>(
        &'a self,
        run_id: &'a str,
        contributions: Vec<AdviserContribution>,
        source_ledger: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Value, BriefAdviserError>>;
}

/// Task 6 persistence seam. Completion is impossible until this future settles.
pub(crate) trait BriefPersistence: Send + Sync {
    fn persist_terminal<'a>(
        &'a self,
        input: TerminalAuditInput,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<PersistedTerminal, BriefPersistenceError>>;

    fn request_cancel(&self, run_id: &str, cancellation: &CancellationToken) -> bool {
        let _ = run_id;
        cancellation.cancel();
        true
    }
}

/// Boundary between durable persistence and the one atomic terminal decision.
pub(crate) trait BriefFinalizationGate: Send + Sync {
    fn wait<'a>(&'a self) -> BriefFuture<'a, ()>;
}

mod providers;
pub(crate) use providers::ReloadingSourceProvider;
#[cfg(test)]
pub(crate) use providers::{CollectedSourceProvider, SourceBackendLoader};
use providers::{ImmediateFinalizationGate, ProductionSourceBackendLoader};

/// Bounded, trusted input for one OFFICIAL command-brief run.
#[derive(Clone)]
pub struct CommandBriefRequest {
    schedule_id: String,
    co_request: String,
    observed_at: String,
}

impl fmt::Debug for CommandBriefRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandBriefRequest")
            .field("schedule_id", &self.schedule_id)
            .field("co_request", &"[REDACTED]")
            .field("observed_at", &self.observed_at)
            .finish()
    }
}

impl CommandBriefRequest {
    /// Creates a request without accepting a classification, prompt, provider, or tools.
    pub fn new(
        schedule_id: &str,
        co_request: &str,
        observed_at: &str,
    ) -> Result<Self, OrchestratorStartError> {
        if !valid_bounded_text(schedule_id, MAX_SCHEDULE_ID_BYTES)
            || !valid_bounded_text(co_request, MAX_CO_REQUEST_BYTES)
            || DateTime::parse_from_rfc3339(observed_at).is_err()
        {
            return Err(OrchestratorStartError::Rejected);
        }
        Ok(Self {
            schedule_id: schedule_id.to_string(),
            co_request: co_request.to_string(),
            observed_at: observed_at.to_string(),
        })
    }
}

struct RunRecord {
    cancellation: CancellationToken,
    state: BriefRunState,
    history: VecDeque<BriefRunStatus>,
    result: Option<PublishedCommandBrief>,
}

struct OrchestratorInner {
    scheduler: LocalModelScheduler,
    sources: Arc<dyn BriefSourceProvider>,
    advisers: Arc<dyn BriefAdviserProvider>,
    persistence: Arc<dyn BriefPersistence>,
    finalization_gate: Arc<dyn BriefFinalizationGate>,
    runs: Mutex<BTreeMap<String, RunRecord>>,
}

/// Trusted state machine for one or more bounded local Daily Command Brief runs.
#[derive(Clone)]
pub struct CommandBriefOrchestrator {
    inner: Arc<OrchestratorInner>,
}

impl fmt::Debug for CommandBriefOrchestrator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandBriefOrchestrator")
            .finish_non_exhaustive()
    }
}

impl CommandBriefOrchestrator {
    /// Installs the app-owned scheduler and native-only orchestration seams.
    pub(crate) fn new(
        scheduler: LocalModelScheduler,
        sources: Arc<dyn BriefSourceProvider>,
        advisers: Arc<dyn BriefAdviserProvider>,
        persistence: Arc<dyn BriefPersistence>,
    ) -> Self {
        Self::new_with_finalization_gate(
            scheduler,
            sources,
            advisers,
            persistence,
            Arc::new(ImmediateFinalizationGate),
        )
    }

    /// Installs an explicit post-persistence boundary used by lifecycle
    /// integrations and deterministic terminal-race verification.
    pub(crate) fn new_with_finalization_gate(
        scheduler: LocalModelScheduler,
        sources: Arc<dyn BriefSourceProvider>,
        advisers: Arc<dyn BriefAdviserProvider>,
        persistence: Arc<dyn BriefPersistence>,
        finalization_gate: Arc<dyn BriefFinalizationGate>,
    ) -> Self {
        Self {
            inner: Arc::new(OrchestratorInner {
                scheduler,
                sources,
                advisers,
                persistence,
                finalization_gate,
                runs: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Constructs the real local-only runtime from protected command-service
    /// configuration, Keychain credentials, the signed Apple allowlist, and
    /// the app-owned scheduler.
    #[allow(
        dead_code,
        reason = "Task 8 installs the production orchestrator into AppState"
    )]
    pub(crate) async fn production(
        app: tauri::AppHandle,
        scheduler: LocalModelScheduler,
        model: &str,
        timeout: Duration,
        persistence: Arc<dyn BriefPersistence>,
    ) -> Result<Self, ProductionOrchestratorError> {
        let config_path = app
            .path()
            .app_config_dir()
            .map_err(|_| ProductionOrchestratorError)?
            .join("command-apple-inputs.json");
        let apple_selection = AppleBriefSelection::load_protected(&config_path)
            .map_err(|_| ProductionOrchestratorError)?;
        let executor = AdviserExecutor::from_catalog(model.to_string(), timeout)
            .map_err(|_| ProductionOrchestratorError)?;
        Ok(Self::new(
            scheduler,
            Arc::new(ReloadingSourceProvider::new(
                Arc::new(ProductionSourceBackendLoader { app }),
                apple_selection,
            )),
            Arc::new(executor),
            persistence,
        ))
    }

    /// Returns the latest bounded status for a run.
    pub fn status(&self, run_id: &str) -> Option<BriefRunStatus> {
        self.inner
            .runs
            .lock()
            .ok()?
            .get(run_id)?
            .history
            .back()
            .cloned()
    }

    /// Returns at most 32 metadata-only lifecycle states.
    pub fn history(&self, run_id: &str) -> Vec<BriefRunStatus> {
        self.inner
            .runs
            .lock()
            .ok()
            .and_then(|runs| {
                runs.get(run_id)
                    .map(|record| record.history.iter().cloned().collect())
            })
            .unwrap_or_default()
    }

    /// Returns the immutable validated brief only after persistence succeeds.
    pub fn result(&self, run_id: &str) -> Option<PublishedCommandBrief> {
        self.inner.runs.lock().ok()?.get(run_id)?.result.clone()
    }

    /// Atomically observes the latest status and its installed validated result.
    pub fn status_and_result(
        &self,
        run_id: &str,
    ) -> Option<(BriefRunStatus, Option<PublishedCommandBrief>)> {
        let runs = self.inner.runs.lock().ok()?;
        let record = runs.get(run_id)?;
        Some((record.history.back()?.clone(), record.result.clone()))
    }

    /// Return whether this runtime still owns any queued or running run.
    pub fn has_nonterminal_runs(&self) -> bool {
        self.inner
            .runs
            .lock()
            .map(|runs| runs.values().any(|record| !is_terminal(record.state)))
            .unwrap_or(true)
    }

    /// Cancels collection, queued/running model work, or persistence.
    pub fn cancel(&self, run_id: &str) -> bool {
        let Ok(runs) = self.inner.runs.lock() else {
            return false;
        };
        let Some(record) = runs.get(run_id) else {
            return false;
        };
        if is_terminal(record.state) {
            return false;
        }
        self.inner
            .persistence
            .request_cancel(run_id, &record.cancellation)
    }

    async fn run(
        &self,
        run_id: String,
        request: CommandBriefRequest,
        cancellation: CancellationToken,
    ) {
        let mut restarted = false;
        loop {
            if cancellation.is_cancelled() {
                self.persist_closed_and_install(
                    &run_id,
                    &request.schedule_id,
                    "unavailable",
                    CommandBriefFailureCode::CancellationRequested,
                    &[],
                    cancellation.clone(),
                )
                .await;
                return;
            }
            self.transition(
                &run_id,
                &request.schedule_id,
                BriefRunState::CollectingSources,
                &[],
                None,
            );
            let context = match self
                .inner
                .sources
                .freeze(
                    &run_id,
                    &request.co_request,
                    &request.observed_at,
                    cancellation.clone(),
                )
                .await
            {
                Ok(context) => context,
                Err(SourceCollectionError::SnapshotChanged) if !restarted => {
                    restarted = true;
                    continue;
                }
                Err(SourceCollectionError::Cancelled) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        "unavailable",
                        CommandBriefFailureCode::CancellationRequested,
                        &[],
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    let code = source_error_code(&error);
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        "unavailable",
                        code,
                        &[],
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            };

            self.transition(
                &run_id,
                &request.schedule_id,
                BriefRunState::RunningSpecialists,
                context.degraded_sections(),
                None,
            );
            let jobs = SPECIALIST_ADVISERS.into_iter().map(|adviser| {
                let scheduler = self.inner.scheduler.clone();
                let advisers = Arc::clone(&self.inner.advisers);
                let sources = context.validated_sources().to_vec();
                let token = cancellation.clone();
                let key = SchedulerJobKey::new(&run_id, adviser);
                let specialist_run_id = run_id.clone();
                async move {
                    let key = key.map_err(|_| SchedulerError::Unavailable)?;
                    scheduler
                        .schedule(key, token.clone(), move |job_token| async move {
                            advisers
                                .run_specialist(&specialist_run_id, adviser, sources, job_token)
                                .await
                        })
                        .await
                }
            });
            let specialist_results = join_all(jobs).await;
            if cancellation.is_cancelled()
                || specialist_results
                    .iter()
                    .any(|result| matches!(result, Err(SchedulerError::Cancelled)))
            {
                self.persist_closed_and_install(
                    &run_id,
                    &request.schedule_id,
                    context.snapshot_id(),
                    CommandBriefFailureCode::CancellationRequested,
                    context.degraded_sections(),
                    cancellation.clone(),
                )
                .await;
                return;
            }
            let ledger_ids = context
                .ledger()
                .iter()
                .map(|source| source.ledger_id().to_string())
                .collect::<BTreeSet<_>>();
            let mut contributions = Vec::with_capacity(SPECIALIST_ADVISERS.len());
            let mut failed_advisers = Vec::new();
            for (adviser, result) in SPECIALIST_ADVISERS.into_iter().zip(specialist_results) {
                match result {
                    Ok(contribution) => contributions.push(contribution),
                    Err(_) => {
                        failed_advisers.push(adviser);
                        let limitation = adviser_unavailable(adviser);
                        let placeholder =
                            limitation_only_contribution(adviser, &limitation, &ledger_ids);
                        let Ok(placeholder) = placeholder else {
                            self.persist_closed_and_install(
                                &run_id,
                                &request.schedule_id,
                                context.snapshot_id(),
                                CommandBriefFailureCode::BriefAssemblyRejected,
                                context.degraded_sections(),
                                cancellation.clone(),
                            )
                            .await;
                            return;
                        };
                        contributions.push(placeholder);
                    }
                }
            }

            match self
                .inner
                .sources
                .recheck(&context, cancellation.clone())
                .await
            {
                Ok(()) => {}
                Err(SourceCollectionError::SnapshotChanged) if !restarted => {
                    restarted = true;
                    continue;
                }
                Err(SourceCollectionError::SnapshotChanged) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::SnapshotChanged,
                        context.degraded_sections(),
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
                Err(SourceCollectionError::Cancelled) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::CancellationRequested,
                        context.degraded_sections(),
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        source_error_code(&error),
                        context.degraded_sections(),
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            }

            let mut degraded = context.degraded_sections().to_vec();
            degraded.extend(failed_advisers.iter().copied().map(section_for_adviser));
            degraded.sort();
            degraded.dedup();
            self.transition(
                &run_id,
                &request.schedule_id,
                BriefRunState::Consolidating,
                &degraded,
                None,
            );
            let chief_key = match SchedulerJobKey::new(&run_id, AdviserId::ChiefOfStaff) {
                Ok(key) => key,
                Err(_) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::ChiefOfStaffOutputRejected,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            };
            let advisers = Arc::clone(&self.inner.advisers);
            let chief_contributions = contributions.clone();
            let chief_sources = context.validated_sources().to_vec();
            let chief_run_id = run_id.clone();
            let chief = self
                .inner
                .scheduler
                .schedule(
                    chief_key,
                    cancellation.clone(),
                    move |job_token| async move {
                        advisers
                            .run_chief_of_staff(
                                &chief_run_id,
                                chief_contributions,
                                chief_sources,
                                job_token,
                            )
                            .await
                    },
                )
                .await;
            let chief_value = match chief {
                Ok(value) => value,
                Err(SchedulerError::Cancelled)
                | Err(SchedulerError::Task(BriefAdviserError::Cancelled)) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::CancellationRequested,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
                Err(_) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::ChiefOfStaffFailed,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            };
            let chief = match validate_chief_output(
                chief_value,
                &contributions,
                context.limitations(),
                &ledger_ids,
            ) {
                Ok(chief) => chief,
                Err(()) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::ChiefOfStaffOutputRejected,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            };

            match self
                .inner
                .sources
                .recheck(&context, cancellation.clone())
                .await
            {
                Ok(()) => {}
                Err(SourceCollectionError::SnapshotChanged) if !restarted => {
                    restarted = true;
                    continue;
                }
                Err(SourceCollectionError::SnapshotChanged) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::SnapshotChanged,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
                Err(SourceCollectionError::Cancelled) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::CancellationRequested,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        source_error_code(&error),
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            }

            let brief = match assemble_brief(
                &run_id,
                &request,
                &context,
                contributions,
                chief,
                &degraded,
                &failed_advisers,
            ) {
                Ok(brief) => brief,
                Err(()) => {
                    self.persist_closed_and_install(
                        &run_id,
                        &request.schedule_id,
                        context.snapshot_id(),
                        CommandBriefFailureCode::BriefAssemblyRejected,
                        &degraded,
                        cancellation.clone(),
                    )
                    .await;
                    return;
                }
            };
            self.transition(
                &run_id,
                &request.schedule_id,
                BriefRunState::Persisting,
                &degraded,
                None,
            );
            self.persist_and_install(
                &run_id,
                &request.schedule_id,
                &degraded,
                TerminalAuditInput::completed(brief),
                cancellation.clone(),
            )
            .await;
            return;
        }
    }

    fn transition(
        &self,
        run_id: &str,
        schedule_id: &str,
        state: BriefRunState,
        degraded: &[BriefSection],
        error: Option<&str>,
    ) {
        if let Ok(mut runs) = self.inner.runs.lock() {
            if let Some(record) = runs.get_mut(run_id) {
                let Some(sequence) = record
                    .history
                    .back()
                    .and_then(|status| status.sequence().checked_add(1))
                else {
                    return;
                };
                let Ok(status) =
                    status_value(run_id, schedule_id, sequence, state, degraded, error)
                else {
                    return;
                };
                if record.history.len() == MAX_STATUS_HISTORY {
                    record.history.pop_front();
                }
                record.state = state;
                record.history.push_back(status);
            }
        }
    }

    fn terminal(
        &self,
        run_id: &str,
        schedule_id: &str,
        state: BriefRunState,
        degraded: &[BriefSection],
        error: Option<&str>,
        result: Option<PublishedCommandBrief>,
    ) {
        if let Ok(mut runs) = self.inner.runs.lock() {
            if let Some(record) = runs.get_mut(run_id) {
                if is_terminal(record.state) {
                    return;
                }
                let Some(sequence) = record
                    .history
                    .back()
                    .and_then(|status| status.sequence().checked_add(1))
                else {
                    return;
                };
                let Ok(status) =
                    status_value(run_id, schedule_id, sequence, state, degraded, error)
                else {
                    return;
                };
                if record.history.len() == MAX_STATUS_HISTORY {
                    record.history.pop_front();
                }
                record.state = state;
                record.history.push_back(status);
                record.result = result;
            }
        }
    }
}

/// Redacted failure to construct the protected production runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "Task 8 installs the production orchestrator into AppState"
)]
pub(crate) struct ProductionOrchestratorError;

mod assembly;
use assembly::*;
