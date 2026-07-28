use super::*;
use crate::command_services::trusted_lan::{ModelRoutingPreference, TrustedLanConfig};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderAttempt {
    Local,
    Cloud(CloudProvider),
}

pub(crate) const fn provider_attempts(preference: ModelRoutingPreference) -> [ProviderAttempt; 3] {
    match preference {
        ModelRoutingPreference::CloudFirst => [
            ProviderAttempt::Cloud(CloudProvider::LiteLlm),
            ProviderAttempt::Cloud(CloudProvider::OpenAi),
            ProviderAttempt::Local,
        ],
        ModelRoutingPreference::LocalFirst => [
            ProviderAttempt::Local,
            ProviderAttempt::Cloud(CloudProvider::LiteLlm),
            ProviderAttempt::Cloud(CloudProvider::OpenAi),
        ],
    }
}

pub(super) struct ImmediateFinalizationGate;

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

pub(super) struct FallbackAdviserProvider {
    pub(super) local: AdviserExecutor,
    pub(super) cloud: CloudAdviserClient,
    pub(super) preference: ModelRoutingPreference,
}

impl BriefAdviserProvider for FallbackAdviserProvider {
    fn run_specialist<'a>(
        &'a self,
        run_id: &'a str,
        adviser: AdviserId,
        sources: Vec<ValidatedSource>,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<AdviserContribution, BriefAdviserError>> {
        Box::pin(async move {
            let identity = format!("{run_id}:{}", adviser_label(adviser));
            for attempt in provider_attempts(self.preference) {
                let result = match attempt {
                    ProviderAttempt::Local => AdviserExecutor::run_specialist(
                        &self.local,
                        SpecialistAdviserRequest::new(&identity, adviser, sources.clone()),
                        cancellation.clone(),
                    )
                    .await
                    .map(|result| result.contribution),
                    ProviderAttempt::Cloud(provider) => {
                        if !self.cloud.available(provider) {
                            eprintln!(
                                "buzz-desktop: {adviser:?} adviser provider {provider:?} is not configured"
                            );
                            continue;
                        }
                        self.cloud
                            .run_specialist(
                                provider,
                                &SpecialistAdviserRequest::new(&identity, adviser, sources.clone()),
                                cancellation.clone(),
                            )
                            .await
                    }
                };
                match result {
                    Ok(contribution) => return Ok(contribution),
                    Err(error) if !cloud_fallback_eligible(error.code()) => {
                        eprintln!(
                            "buzz-desktop: {adviser:?} adviser provider {attempt:?} failed without retry: {:?}",
                            error.code()
                        );
                        return Err(BriefAdviserError::from(error));
                    }
                    Err(error) => eprintln!(
                        "buzz-desktop: {adviser:?} adviser provider {attempt:?} failed: {:?}",
                        error.code()
                    ),
                }
            }
            eprintln!("buzz-desktop: {adviser:?} adviser providers exhausted");
            Err(BriefAdviserError::Failed)
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
            let identity = format!("{run_id}:chief_of_staff");
            for attempt in provider_attempts(self.preference) {
                let result = match attempt {
                    ProviderAttempt::Local => AdviserExecutor::run_chief_of_staff(
                        &self.local,
                        ChiefOfStaffRequest::new(
                            &identity,
                            contributions.clone(),
                            source_ledger.clone(),
                        ),
                        cancellation.clone(),
                    )
                    .await
                    .map(|result| result.contribution),
                    ProviderAttempt::Cloud(provider) => {
                        if !self.cloud.available(provider) {
                            eprintln!(
                                "buzz-desktop: ChiefOfStaff adviser provider {provider:?} is not configured"
                            );
                            continue;
                        }
                        self.cloud
                            .run_chief_of_staff(
                                provider,
                                &ChiefOfStaffRequest::new(
                                    &identity,
                                    contributions.clone(),
                                    source_ledger.clone(),
                                ),
                                cancellation.clone(),
                            )
                            .await
                    }
                };
                match result {
                    Ok(consolidation) => {
                        return serde_json::to_value(consolidation)
                            .map_err(|_| BriefAdviserError::Failed);
                    }
                    Err(error) if !cloud_fallback_eligible(error.code()) => {
                        eprintln!(
                            "buzz-desktop: ChiefOfStaff adviser provider {attempt:?} failed without retry: {:?}",
                            error.code()
                        );
                        return Err(BriefAdviserError::from(error));
                    }
                    Err(error) => eprintln!(
                        "buzz-desktop: ChiefOfStaff adviser provider {attempt:?} failed: {:?}",
                        error.code()
                    ),
                }
            }
            eprintln!("buzz-desktop: ChiefOfStaff adviser providers exhausted");
            Err(BriefAdviserError::Failed)
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
pub(super) struct ProductionSourceBackendLoader {
    pub(super) app: tauri::AppHandle,
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
            let command_team_discussions = load_command_team_discussions(&app, Utc::now())
                .await
                .unwrap_or_else(|_| {
                    CommandTeamDiscussionBatch::unavailable(
                        "Command-team discussion memory was unavailable.",
                    )
                });
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let backend = ProductionSourceBackend::from_app(app)
                .await?
                .with_command_team_discussions(command_team_discussions);
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            Ok(Arc::new(backend) as Arc<dyn SourceBackend + Send + Sync>)
        })
    }
}

#[derive(Clone)]
pub(super) struct TrustedLanSourceBackendLoader {
    pub(super) config: TrustedLanConfig,
    pub(super) app: tauri::AppHandle,
}

impl SourceBackendLoader for TrustedLanSourceBackendLoader {
    fn load<'a>(
        &'a self,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<Arc<dyn SourceBackend + Send + Sync>, SourceCollectionError>> {
        let config = self.config.clone();
        let app = self.app.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let observed_at_value = Utc::now();
            let command_team_discussions = load_command_team_discussions(&app, observed_at_value)
                .await
                .unwrap_or_else(|_| {
                    CommandTeamDiscussionBatch::unavailable(
                        "Command-team discussion memory was unavailable.",
                    )
                });
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let observed_at = observed_at_value.to_rfc3339_opts(SecondsFormat::Millis, true);
            let backend = tokio::task::spawn_blocking(move || {
                TrustedLanSourceBackend::from_config(&config, &observed_at)
                    .map(|backend| backend.with_command_team_discussions(command_team_discussions))
            })
            .await
            .map_err(|_| SourceCollectionError::RagInvalid)??;
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
    world_monitor: Option<(tauri::AppHandle, String)>,
}

impl ReloadingSourceProvider {
    #[cfg(test)]
    pub(crate) const fn new(
        loader: Arc<dyn SourceBackendLoader>,
        apple_selection: AppleBriefSelection,
    ) -> Self {
        Self {
            loader,
            apple_selection,
            world_monitor: None,
        }
    }

    pub(crate) fn new_with_world_monitor(
        loader: Arc<dyn SourceBackendLoader>,
        apple_selection: AppleBriefSelection,
        app: tauri::AppHandle,
        endpoint: String,
    ) -> Self {
        Self {
            loader,
            apple_selection,
            world_monitor: Some((app, endpoint)),
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
        let world_monitor = self.world_monitor.clone();
        Box::pin(async move {
            let backend = loader.load(cancellation.clone()).await?;
            if cancellation.is_cancelled() {
                return Err(SourceCollectionError::Cancelled);
            }
            let collection_cancellation = cancellation.clone();
            let result = tokio::task::spawn_blocking(move || {
                SourceCollector::new(backend, &run_id, &co_request, &observed_at, apple_selection)
                    .map(|collector| match world_monitor {
                        Some((app, endpoint)) => collector.with_world_monitor(
                            WorldMonitorBriefCollector::from_app(&app, &endpoint),
                        ),
                        None => collector,
                    })
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
        if context.rag_snapshot_assurance() == RagSnapshotAssurance::TrustedLanObserved {
            return Box::pin(async move {
                if cancellation.is_cancelled() {
                    Err(SourceCollectionError::Cancelled)
                } else {
                    Ok(())
                }
            });
        }
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
