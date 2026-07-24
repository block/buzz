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

struct ImmediateFinalizationGate;

impl BriefFinalizationGate for ImmediateFinalizationGate {
    fn wait<'a>(&'a self) -> BriefFuture<'a, ()> {
        Box::pin(async {})
    }
}

impl BriefAdviserProvider for AdviserExecutor {
    fn run_specialist<'a>(
        &'a self,
        run_id: &'a str,
        adviser: AdviserId,
        sources: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<AdviserContribution, BriefAdviserError>> {
        Box::pin(async move {
            AdviserExecutor::run_specialist(
                self,
                SpecialistAdviserRequest::new(
                    format!("{run_id}:{}", adviser_label(adviser)),
                    adviser,
                    sources,
                ),
                cancellation,
            )
            .await
            .map(|result| result.contribution)
            .map_err(BriefAdviserError::from)
        })
    }

    fn run_chief_of_staff<'a>(
        &'a self,
        run_id: &'a str,
        contributions: Vec<AdviserContribution>,
        source_ledger: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Value, BriefAdviserError>> {
        Box::pin(async move {
            AdviserExecutor::run_chief_of_staff(
                self,
                ChiefOfStaffRequest::new(
                    format!("{run_id}:chief_of_staff"),
                    contributions,
                    source_ledger,
                ),
                cancellation,
            )
            .await
            .map_err(BriefAdviserError::from)
            .and_then(|result| {
                serde_json::to_value(result.contribution).map_err(|_| BriefAdviserError::Failed)
            })
        })
    }
}

/// Production adapter over Task 4's trusted `SourceCollector` and exact backend.
#[allow(
    dead_code,
    reason = "Task 8 constructs this adapter when Tauri commands expose orchestration"
)]
pub(crate) struct CollectedSourceProvider<B> {
    backend: B,
    apple_selection: AppleBriefSelection,
}

#[allow(
    dead_code,
    reason = "Task 8 constructs this adapter when Tauri commands expose orchestration"
)]
impl<B> CollectedSourceProvider<B> {
    pub(crate) const fn new(backend: B, apple_selection: AppleBriefSelection) -> Self {
        Self {
            backend,
            apple_selection,
        }
    }
}

impl<B> BriefSourceProvider for CollectedSourceProvider<B>
where
    B: SourceBackend + Clone + Send + Sync + 'static,
{
    fn freeze<'a>(
        &'a self,
        run_id: &'a str,
        co_request: &'a str,
        observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        let backend = self.backend.clone();
        let apple_selection = self.apple_selection.clone();
        let run_id = run_id.to_string();
        let co_request = co_request.to_string();
        let observed_at = observed_at.to_string();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let collection_cancellation = cancellation.clone();
            let result = tokio::task::spawn_blocking(move || {
                SourceCollector::new(backend, &run_id, &co_request, &observed_at, apple_selection)
                    .and_then(|collector| {
                        collector.freeze_with_cancellation(&collection_cancellation)
                    })
            })
            .await
            .map_err(|_| SourceCollectionError::RagInvalid)?;
            if cancellation.is_cancelled() {
                Err(SourceCollectionError::Cancelled)
            } else {
                result
            }
        })
    }

    fn recheck<'a>(
        &'a self,
        context: &'a FrozenSourceContext,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>> {
        let backend = self.backend.clone();
        let snapshot = context.snapshot_binding().clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let recheck_cancellation = cancellation.clone();
            let result = tokio::task::spawn_blocking(move || {
                backend.recheck_rag_snapshot(&snapshot, &recheck_cancellation)
            })
            .await
            .map_err(|_| SourceCollectionError::RagInvalid)?;
            if cancellation.is_cancelled() {
                Err(SourceCollectionError::Cancelled)
            } else {
                result
            }
        })
    }
}

/// Loads freshly re-attested local source bindings for each collection or
/// snapshot-consistency boundary.
pub(crate) trait SourceBackendLoader: Send + Sync {
    fn load<'a>(
        &'a self,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Arc<dyn SourceBackend + Send + Sync>, SourceCollectionError>>;
}

#[derive(Clone)]
struct ProductionSourceBackendLoader {
    app: tauri::AppHandle,
}

impl SourceBackendLoader for ProductionSourceBackendLoader {
    fn load<'a>(
        &'a self,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Arc<dyn SourceBackend + Send + Sync>, SourceCollectionError>> {
        let app = self.app.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let backend = ProductionSourceBackend::from_app(app).await?;
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            Ok(Arc::new(backend) as Arc<dyn SourceBackend + Send + Sync>)
        })
    }
}

/// Production source provider that never retains an admitted source binding
/// across collection attempts or snapshot-consistency boundaries.
pub(crate) struct ReloadingSourceProvider {
    loader: Arc<dyn SourceBackendLoader>,
    apple_selection: AppleBriefSelection,
}

impl ReloadingSourceProvider {
    pub(crate) const fn new(
        loader: Arc<dyn SourceBackendLoader>,
        apple_selection: AppleBriefSelection,
    ) -> Self {
        Self {
            loader,
            apple_selection,
        }
    }
}

impl BriefSourceProvider for ReloadingSourceProvider {
    fn freeze<'a>(
        &'a self,
        run_id: &'a str,
        co_request: &'a str,
        observed_at: &'a str,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<FrozenSourceContext, SourceCollectionError>> {
        let loader = Arc::clone(&self.loader);
        let apple_selection = self.apple_selection.clone();
        let run_id = run_id.to_string();
        let co_request = co_request.to_string();
        let observed_at = observed_at.to_string();
        Box::pin(async move {
            let backend = loader.load(cancellation.clone()).await?;
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let collection_cancellation = cancellation.clone();
            let result = tokio::task::spawn_blocking(move || {
                SourceCollector::new(backend, &run_id, &co_request, &observed_at, apple_selection)
                    .and_then(|collector| {
                        collector.freeze_with_cancellation(&collection_cancellation)
                    })
            })
            .await
            .map_err(|_| SourceCollectionError::RagInvalid)?;
            if cancellation.is_cancelled() {
                Err(SourceCollectionError::Cancelled)
            } else {
                result
            }
        })
    }

    fn recheck<'a>(
        &'a self,
        context: &'a FrozenSourceContext,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<(), SourceCollectionError>> {
        let loader = Arc::clone(&self.loader);
        let snapshot = context.snapshot_binding().clone();
        Box::pin(async move {
            let backend = loader.load(cancellation.clone()).await?;
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let recheck_cancellation = cancellation.clone();
            let result = tokio::task::spawn_blocking(move || {
                backend.recheck_rag_snapshot(&snapshot, &recheck_cancellation)
            })
            .await
            .map_err(|_| SourceCollectionError::RagInvalid)?;
            if cancellation.is_cancelled() {
                Err(SourceCollectionError::Cancelled)
            } else {
                result
            }
        })
    }
}

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
            return Err(OrchestratorStartError);
        }
        Ok(Self {
            schedule_id: schedule_id.to_string(),
            co_request: co_request.to_string(),
            observed_at: observed_at.to_string(),
        })
    }
}

/// A redacted start error for invalid or capacity-exhausted run requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrchestratorStartError;

impl fmt::Display for OrchestratorStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("command brief run rejected")
    }
}

impl std::error::Error for OrchestratorStartError {}

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

    /// Starts a unique trusted run and returns immediately with its UUID.
    pub fn start(&self, request: CommandBriefRequest) -> Result<String, OrchestratorStartError> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let cancellation = CancellationToken::new();
        let queued = status_value(
            &run_id,
            &request.schedule_id,
            BriefRunState::Queued,
            &[],
            None,
        )
        .map_err(|_| OrchestratorStartError)?;
        {
            let mut runs = self.inner.runs.lock().map_err(|_| OrchestratorStartError)?;
            if runs.len() >= MAX_TRACKED_RUNS {
                let removable = runs
                    .iter()
                    .find(|(_, record)| is_terminal(record.state))
                    .map(|(id, _)| id.clone());
                if let Some(removable) = removable {
                    runs.remove(&removable);
                } else {
                    return Err(OrchestratorStartError);
                }
            }
            runs.insert(
                run_id.clone(),
                RunRecord {
                    cancellation: cancellation.clone(),
                    state: BriefRunState::Queued,
                    history: VecDeque::from([queued]),
                    result: None,
                },
            );
        }
        let orchestrator = self.clone();
        let spawned_run_id = run_id.clone();
        tokio::spawn(async move {
            orchestrator
                .run(spawned_run_id, request, cancellation)
                .await;
        });
        Ok(run_id)
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
                None,
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
        let Ok(status) = status_value(run_id, schedule_id, state, degraded, error) else {
            return;
        };
        if let Ok(mut runs) = self.inner.runs.lock() {
            if let Some(record) = runs.get_mut(run_id) {
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
                let Ok(status) = status_value(run_id, schedule_id, state, degraded, error) else {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawChiefOutput {
    classification: Classification,
    adviser: AdviserId,
    findings: Vec<Value>,
    limitations: Vec<String>,
    dissent: Vec<String>,
}

struct ValidatedChief {
    findings: Vec<CitedFinding>,
}

fn validate_chief_output(
    value: Value,
    contributions: &[AdviserContribution],
    source_limitations: &[String],
    ledger_ids: &BTreeSet<String>,
) -> Result<ValidatedChief, ()> {
    let raw: RawChiefOutput = serde_json::from_value(value).map_err(|_| ())?;
    if raw.classification != Classification::Official
        || raw.adviser != AdviserId::ChiefOfStaff
        || raw.findings.len() > MAX_ARRAY_ITEMS
        || !valid_text_array(&raw.limitations, MAX_ARRAY_ITEMS)
        || !valid_text_array(&raw.dissent, MAX_AGGREGATE_DISSENT_ITEMS)
    {
        return Err(());
    }
    let expected_dissent = contributions
        .iter()
        .flat_map(|contribution| contribution.dissent().iter().cloned())
        .collect::<Vec<_>>();
    if raw.dissent != expected_dissent {
        return Err(());
    }
    let allowed_limitations = source_limitations
        .iter()
        .chain(
            contributions
                .iter()
                .flat_map(|contribution| contribution.limitations()),
        )
        .collect::<BTreeSet<_>>();
    let mut seen_limitations = BTreeSet::new();
    if raw.limitations.iter().any(|limitation| {
        !allowed_limitations.contains(limitation) || !seen_limitations.insert(limitation)
    }) {
        return Err(());
    }
    let allowed = contributions
        .iter()
        .flat_map(|contribution| contribution.findings())
        .map(|finding| (finding.text().to_string(), finding.source_ids().to_vec()))
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut findings = Vec::with_capacity(raw.findings.len());
    for value in raw.findings {
        let finding = CitedFinding::parse_for_ledger(value, ledger_ids).map_err(|_| ())?;
        let identity = (finding.text().to_string(), finding.source_ids().to_vec());
        if !allowed.contains(&identity) || !seen.insert(identity) {
            return Err(());
        }
        findings.push(finding);
    }
    Ok(ValidatedChief { findings })
}

fn assemble_brief(
    run_id: &str,
    request: &CommandBriefRequest,
    context: &FrozenSourceContext,
    contributions: Vec<AdviserContribution>,
    chief: ValidatedChief,
    degraded: &[BriefSection],
    failed_advisers: &[AdviserId],
) -> Result<CommandBrief, ()> {
    let mut sections = BTreeMap::<BriefSection, Vec<CitedFinding>>::from([
        (BriefSection::Today, chief.findings),
        (BriefSection::Operations, Vec::new()),
        (BriefSection::Navigation, Vec::new()),
        (BriefSection::DailyRoutine, Vec::new()),
        (BriefSection::Reports, Vec::new()),
        (BriefSection::Planning306090, Vec::new()),
        (BriefSection::Decisions, Vec::new()),
        (BriefSection::ConflictsAndGaps, Vec::new()),
        (BriefSection::Sources, Vec::new()),
    ]);
    for contribution in &contributions {
        sections.insert(contribution.section(), contribution.findings().to_vec());
    }
    let dissent = contributions
        .iter()
        .flat_map(|contribution| contribution.dissent().iter().cloned())
        .collect::<Vec<_>>();
    let missing_information =
        bounded_missing_information(failed_advisers, &contributions, context.limitations());
    let value = json!({
        "version": 1,
        "classification": "OFFICIAL",
        "generatedAt": timestamp(),
        "runId": run_id,
        "scheduleId": request.schedule_id,
        "snapshotId": context.snapshot_id(),
        "sections": sections,
        "degradedSections": degraded,
        "missingInformation": missing_information,
        "dissent": dissent,
        "sourceLedger": context.ledger(),
        "sourceFreshness": {
            "classification": "OFFICIAL",
            "asOf": context.observed_at(),
            "staleSourceIds": []
        },
        "contributions": contributions,
        "advisoryLimitation": super::types::ADVISORY_LIMITATION
    });
    CommandBrief::try_from(value).map_err(|_| ())
}

fn limitation_only_contribution(
    adviser: AdviserId,
    limitation: &str,
    ledger_ids: &BTreeSet<String>,
) -> Result<AdviserContribution, ()> {
    AdviserContribution::parse_for_adviser(
        json!({
            "classification": "OFFICIAL",
            "adviser": adviser,
            "section": section_for_adviser(adviser),
            "findings": [],
            "confidence": 0.0,
            "limitations": [limitation],
            "dissent": [],
            "proposedActions": []
        }),
        adviser,
        ledger_ids,
    )
    .map_err(|_| ())
}

fn section_for_adviser(adviser: AdviserId) -> BriefSection {
    match adviser {
        AdviserId::Operations => BriefSection::Operations,
        AdviserId::Navigation => BriefSection::Navigation,
        AdviserId::DailyRoutine => BriefSection::DailyRoutine,
        AdviserId::Reporting => BriefSection::Reports,
        AdviserId::Plans => BriefSection::Planning306090,
        AdviserId::ChiefOfStaff => BriefSection::ConflictsAndGaps,
    }
}

fn adviser_unavailable(adviser: AdviserId) -> String {
    format!(
        "{} adviser output was unavailable.",
        adviser_display(adviser)
    )
}

fn adviser_display(adviser: AdviserId) -> &'static str {
    match adviser {
        AdviserId::ChiefOfStaff => "Chief of Staff",
        AdviserId::Operations => "Operations",
        AdviserId::Navigation => "Navigation",
        AdviserId::DailyRoutine => "Daily Routine",
        AdviserId::Reporting => "Reporting",
        AdviserId::Plans => "Plans",
    }
}

fn adviser_label(adviser: AdviserId) -> &'static str {
    match adviser {
        AdviserId::ChiefOfStaff => "chief_of_staff",
        AdviserId::Operations => "operations",
        AdviserId::Navigation => "navigation",
        AdviserId::DailyRoutine => "daily_routine",
        AdviserId::Reporting => "reporting",
        AdviserId::Plans => "plans",
    }
}

fn source_error_code(error: &SourceCollectionError) -> CommandBriefFailureCode {
    match error {
        SourceCollectionError::Cancelled => CommandBriefFailureCode::CancellationRequested,
        SourceCollectionError::SnapshotChanged => CommandBriefFailureCode::SnapshotChanged,
        SourceCollectionError::RagUnavailable => CommandBriefFailureCode::RagUnavailable,
        SourceCollectionError::RagStale => CommandBriefFailureCode::RagStale,
        SourceCollectionError::RagInvalid => CommandBriefFailureCode::RagInvalid,
        SourceCollectionError::InvalidRequest => CommandBriefFailureCode::SourceRequestRejected,
        SourceCollectionError::InvalidTime => CommandBriefFailureCode::SourceTimeRejected,
        SourceCollectionError::ConflictingSourceIdentity => {
            CommandBriefFailureCode::SourceIdentityConflict
        }
    }
}

fn status_value(
    run_id: &str,
    schedule_id: &str,
    state: BriefRunState,
    degraded: &[BriefSection],
    error: Option<&str>,
) -> Result<BriefRunStatus, ()> {
    BriefRunStatus::try_from(json!({
        "classification": "OFFICIAL",
        "runId": run_id,
        "scheduleId": schedule_id,
        "state": state,
        "updatedAt": timestamp(),
        "degradedSections": degraded,
        "error": error
    }))
    .map_err(|_| ())
}

fn valid_bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}

fn valid_text_array(values: &[String], maximum_items: usize) -> bool {
    values.len() <= maximum_items
        && values
            .iter()
            .all(|value| valid_bounded_text(value, MAX_TEXT_BYTES))
}

fn bounded_missing_information(
    failed_advisers: &[AdviserId],
    contributions: &[AdviserContribution],
    source_limitations: &[String],
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut required = Vec::new();
    for adviser in SPECIALIST_ADVISERS {
        if failed_advisers.contains(&adviser) {
            let limitation = adviser_unavailable(adviser);
            if valid_bounded_text(&limitation, MAX_TEXT_BYTES) && seen.insert(limitation.clone()) {
                required.push(limitation);
            }
        }
    }

    let mut specialist_limitations = contributions
        .iter()
        .flat_map(|contribution| contribution.limitations().iter().cloned())
        .collect::<Vec<_>>();
    specialist_limitations.sort();
    let mut source_limitations = source_limitations.to_vec();
    source_limitations.sort();

    let mut optional = Vec::new();
    for value in specialist_limitations.into_iter().chain(source_limitations) {
        if valid_bounded_text(&value, MAX_TEXT_BYTES) && seen.insert(value.clone()) {
            optional.push(value);
        }
    }

    if required.len() + optional.len() <= MAX_ARRAY_ITEMS {
        required.extend(optional);
        return required;
    }

    let optional_capacity = MAX_ARRAY_ITEMS
        .saturating_sub(required.len())
        .saturating_sub(1);
    let omitted = optional.len().saturating_sub(optional_capacity);
    required.extend(optional.into_iter().take(optional_capacity));
    required.push(format!(
        "{omitted} additional trusted limitations omitted after the canonical limit."
    ));
    required
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn is_terminal(state: BriefRunState) -> bool {
    matches!(
        state,
        BriefRunState::Completed
            | BriefRunState::Degraded
            | BriefRunState::Cancelled
            | BriefRunState::Failed
    )
}
