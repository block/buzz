use super::*;

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
