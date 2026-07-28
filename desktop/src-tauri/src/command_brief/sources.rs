#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 5 wires the frozen source collector into the run orchestrator"
    )
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::DateTime;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::provenance::ValidatedSource;
use super::types::{AdviserId, BriefSection, SourceKind, SourceLedgerEntry, MAX_ARRAY_ITEMS};
use crate::command_services::apple_inputs::{
    read_apple_inputs_blocking, AppleBriefSelection, AppleInputPermission, AppleInputRequest,
    AppleInputResponse,
};
use crate::command_services::memory::{
    extract_verified_memory_evidence, get_memory_source_binding,
};
use crate::command_services::policy::{AdmissionError, AuthenticatedSourceService};
use crate::command_services::rag::{
    extract_verified_rag_evidence, get_rag_source_binding, RagSnapshotAssurance, RagSnapshotError,
    VerifiedRagSnapshot,
};
use crate::command_services::trusted_lan::{
    catalogue_fingerprint, TrustedLanConfig, TrustedLanSourceClient,
};

const MAX_CO_REQUEST_BYTES: usize = 1024;
const RAG_TOOL: &str = "search_knowledge_base";
const MEMORY_TOOL: &str = "command_memory_context";
const RETRIEVAL_RESULT_LIMIT: u32 = 3;

const RAG_MEMORY_SECTIONS: [BriefSection; 5] = [
    BriefSection::Operations,
    BriefSection::Navigation,
    BriefSection::DailyRoutine,
    BriefSection::Reports,
    BriefSection::Planning306090,
];

/// Backend seam implemented by the admitted local RAG, Memory, and signed Apple services.
pub(crate) trait SourceBackend: Send + Sync {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError>;

    fn memory_conflict_count(&self) -> u64;

    fn command_team_discussions(&self) -> CommandTeamDiscussionBatch {
        CommandTeamDiscussionBatch::default()
    }

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
        query: &str,
        collections: &[String],
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError>;

    fn collect_memory(
        &self,
        intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError>;

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError>;

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError>;

    fn post_recheck_limitations(&self) -> Vec<String> {
        Vec::new()
    }
}

impl<T> SourceBackend for Arc<T>
where
    T: SourceBackend + ?Sized,
{
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        (**self).verify_active_rag_snapshot()
    }

    fn memory_conflict_count(&self) -> u64 {
        (**self).memory_conflict_count()
    }

    fn command_team_discussions(&self) -> CommandTeamDiscussionBatch {
        (**self).command_team_discussions()
    }

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
        query: &str,
        collections: &[String],
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        (**self).collect_rag(snapshot, intent, query, collections, cancellation)
    }

    fn collect_memory(
        &self,
        intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        (**self).collect_memory(intent, cancellation)
    }

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        (**self).collect_apple(request, cancellation)
    }

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        (**self).recheck_rag_snapshot(expected, cancellation)
    }

    fn post_recheck_limitations(&self) -> Vec<String> {
        (**self).post_recheck_limitations()
    }
}

/// Concrete production backend for the verified local RAG, Memory, and signed
/// Apple helper sources.
#[derive(Clone)]
pub(crate) struct ProductionSourceBackend {
    snapshot: VerifiedRagSnapshot,
    rag: AuthenticatedSourceService,
    memory: Option<AuthenticatedSourceService>,
    memory_conflict_count: u64,
    command_team_discussions: CommandTeamDiscussionBatch,
    caller: Arc<dyn SourceToolCaller>,
}

impl std::fmt::Debug for ProductionSourceBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionSourceBackend")
            .field("snapshot_id", &self.snapshot.snapshot_id())
            .field("rag", &self.rag)
            .field("memory", &self.memory)
            .field("memory_conflict_count", &self.memory_conflict_count)
            .finish_non_exhaustive()
    }
}

impl ProductionSourceBackend {
    /// Re-attests and binds the exact configured local services.
    #[allow(
        dead_code,
        reason = "Task 8 installs the production orchestrator into AppState"
    )]
    pub(crate) async fn from_app(app: tauri::AppHandle) -> Result<Self, SourceCollectionError> {
        let (rag, memory) = tokio::join!(
            get_rag_source_binding(app.clone()),
            get_memory_source_binding(app),
        );
        let rag = rag.map_err(|_| SourceCollectionError::RagUnavailable)?;
        let (memory, memory_conflict_count) = memory
            .map(|binding| (Some(binding.service), binding.conflict_count))
            .unwrap_or((None, 0));
        Ok(Self {
            snapshot: rag.snapshot,
            rag: rag.service,
            memory,
            memory_conflict_count,
            command_team_discussions: CommandTeamDiscussionBatch::default(),
            caller: Arc::new(AuthenticatedMcpSourceCaller),
        })
    }

    pub(super) fn with_command_team_discussions(
        mut self,
        command_team_discussions: CommandTeamDiscussionBatch,
    ) -> Self {
        self.command_team_discussions = command_team_discussions;
        self
    }

    #[cfg(test)]
    pub(crate) fn from_bindings_for_test(
        snapshot: VerifiedRagSnapshot,
        rag: AuthenticatedSourceService,
        memory: AuthenticatedSourceService,
        memory_conflict_count: u64,
        caller: Arc<dyn SourceToolCaller>,
    ) -> Self {
        Self {
            snapshot,
            rag,
            memory: Some(memory),
            memory_conflict_count,
            command_team_discussions: CommandTeamDiscussionBatch::default(),
            caller,
        }
    }
}

impl SourceBackend for ProductionSourceBackend {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        if self.rag.active_identity() != self.snapshot.snapshot_id() {
            return Err(SourceCollectionError::RagInvalid);
        }
        Ok(self.snapshot.clone())
    }

    fn memory_conflict_count(&self) -> u64 {
        self.memory_conflict_count
    }

    fn command_team_discussions(&self) -> CommandTeamDiscussionBatch {
        self.command_team_discussions.clone()
    }

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        _intent: &FixedRetrievalIntent,
        query: &str,
        collections: &[String],
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        if cancellation.is_cancelled() {
            return Err(SourceReadError::new("cancelled"));
        }
        if snapshot != &self.snapshot || self.rag.active_identity() != snapshot.snapshot_id() {
            return Err(SourceReadError::new("rag_snapshot_mismatch"));
        }
        let result = self
            .caller
            .call(
                &self.rag,
                RAG_TOOL,
                json!({
                    "query": query,
                    "collections": collections,
                    "top_k": RETRIEVAL_RESULT_LIMIT,
                }),
                cancellation,
            )
            .map_err(|_| SourceReadError::new("rag_read_unavailable"))?;
        if cancellation.is_cancelled() {
            Err(SourceReadError::new("cancelled"))
        } else {
            Ok(result)
        }
    }

    fn collect_memory(
        &self,
        intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        if cancellation.is_cancelled() {
            return Err(SourceReadError::new("cancelled"));
        }
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| SourceReadError::new("memory_read_unavailable"))?;
        let result = self
            .caller
            .call(
                memory,
                MEMORY_TOOL,
                json!({"query": intent.query(), "limit": RETRIEVAL_RESULT_LIMIT}),
                cancellation,
            )
            .map_err(|_| SourceReadError::new("memory_read_unavailable"))?;
        if cancellation.is_cancelled() {
            Err(SourceReadError::new("cancelled"))
        } else {
            Ok(result)
        }
    }

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        if cancellation.is_cancelled() {
            return Err(SourceCollectionError::Cancelled);
        }
        let response = read_apple_inputs_blocking(request.clone());
        if cancellation.is_cancelled() {
            Err(SourceCollectionError::Cancelled)
        } else {
            Ok(response)
        }
    }

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        if cancellation.is_cancelled() {
            return Err(SourceCollectionError::Cancelled);
        }
        if expected != &self.snapshot {
            return Err(SourceCollectionError::SnapshotChanged);
        }
        let status = self
            .caller
            .call(&self.rag, "get_snapshot_status", json!({}), cancellation)
            .map_err(|_| SourceCollectionError::RagUnavailable)?;
        if cancellation.is_cancelled() {
            return Err(SourceCollectionError::Cancelled);
        }
        let observed = status
            .get("active_snapshot_id")
            .and_then(Value::as_str)
            .ok_or(SourceCollectionError::RagInvalid)?;
        expected.verify_unchanged(observed).map_err(Into::into)
    }
}

/// Direct read-only adapter for the owner-approved trusted-LAN Memory and RAG
/// services. Evidence is explicitly marked as observed rather than signed.
#[derive(Clone, Debug)]
pub(crate) struct TrustedLanSourceBackend {
    snapshot: VerifiedRagSnapshot,
    client: TrustedLanSourceClient,
    command_team_discussions: CommandTeamDiscussionBatch,
    post_recheck_warning: Arc<Mutex<Option<String>>>,
}

impl TrustedLanSourceBackend {
    pub(crate) fn from_config(
        config: &TrustedLanConfig,
        observed_at: &str,
    ) -> Result<Self, SourceCollectionError> {
        let client = config
            .source_client()
            .map_err(|_| SourceCollectionError::RagUnavailable)?;
        let catalogue = client
            .catalogue()
            .map_err(|_| SourceCollectionError::RagUnavailable)?;
        let fingerprint =
            catalogue_fingerprint(&catalogue).map_err(|_| SourceCollectionError::RagInvalid)?;
        let collections = observed_collection_names(&catalogue)?;
        let snapshot = VerifiedRagSnapshot::from_trusted_lan_observation(
            &fingerprint,
            observed_at,
            collections,
        )
        .map_err(|_| SourceCollectionError::RagInvalid)?;
        Ok(Self {
            snapshot,
            client,
            command_team_discussions: CommandTeamDiscussionBatch::default(),
            post_recheck_warning: Arc::new(Mutex::new(None)),
        })
    }

    pub(super) fn with_command_team_discussions(
        mut self,
        command_team_discussions: CommandTeamDiscussionBatch,
    ) -> Self {
        self.command_team_discussions = command_team_discussions;
        self
    }
}

impl SourceBackend for TrustedLanSourceBackend {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError> {
        Ok(self.snapshot.clone())
    }

    fn memory_conflict_count(&self) -> u64 {
        0
    }

    fn command_team_discussions(&self) -> CommandTeamDiscussionBatch {
        self.command_team_discussions.clone()
    }

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        _intent: &FixedRetrievalIntent,
        query: &str,
        collections: &[String],
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        if cancellation.is_cancelled() {
            return Err(SourceReadError::new("cancelled"));
        }
        if snapshot != &self.snapshot {
            return Err(SourceReadError::new("rag_catalogue_mismatch"));
        }
        let result = self
            .client
            .search_rag(query, collections)
            .map_err(|error| {
                eprintln!("buzz-desktop: trusted LAN RAG read failed: {error:?}");
                SourceReadError::new("rag_read_unavailable")
            })?;
        if cancellation.is_cancelled() {
            Err(SourceReadError::new("cancelled"))
        } else {
            Ok(result)
        }
    }

    fn collect_memory(
        &self,
        intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        if cancellation.is_cancelled() {
            return Err(SourceReadError::new("cancelled"));
        }
        let result = self
            .client
            .search_memory(intent.query(), RETRIEVAL_RESULT_LIMIT)
            .map_err(|error| {
                eprintln!("buzz-desktop: trusted LAN Memory read failed: {error:?}");
                SourceReadError::new("memory_read_unavailable")
            })?;
        if cancellation.is_cancelled() {
            Err(SourceReadError::new("cancelled"))
        } else {
            Ok(result)
        }
    }

    fn collect_apple(
        &self,
        request: &AppleInputRequest,
        cancellation: &CancellationToken,
    ) -> Result<AppleInputResponse, SourceCollectionError> {
        if cancellation.is_cancelled() {
            return Err(SourceCollectionError::Cancelled);
        }
        let response = read_apple_inputs_blocking(request.clone());
        if cancellation.is_cancelled() {
            Err(SourceCollectionError::Cancelled)
        } else {
            Ok(response)
        }
    }

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        if cancellation.is_cancelled() {
            return Err(SourceCollectionError::Cancelled);
        }
        // This is deliberately audit-only. A changed catalogue must never
        // restart, invalidate, or fail an in-flight trusted-LAN brief.
        let finish_fingerprint = self
            .client
            .catalogue()
            .ok()
            .and_then(|catalogue| catalogue_fingerprint(&catalogue).ok());
        if finish_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint != expected.snapshot_id())
        {
            if let Ok(mut warning) = self.post_recheck_warning.lock() {
                *warning = Some(
                    "Trusted-LAN catalogue changed during generation; the recorded fingerprint is audit-only and the brief uses its cited passages."
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    fn post_recheck_limitations(&self) -> Vec<String> {
        self.post_recheck_warning
            .lock()
            .ok()
            .and_then(|warning| warning.clone())
            .into_iter()
            .collect()
    }
}

/// All source evidence frozen before any adviser runs.
pub(crate) struct FrozenSourceContext {
    run_id: String,
    snapshot: VerifiedRagSnapshot,
    observed_at: String,
    ledger: Vec<SourceLedgerEntry>,
    validated_sources: Vec<ValidatedSource>,
    degraded_sections: Vec<BriefSection>,
    limitations: Vec<String>,
    retrieval_intents: Vec<FixedRetrievalIntent>,
}

impl FrozenSourceContext {
    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn snapshot_id(&self) -> &str {
        self.snapshot.snapshot_id()
    }

    pub(crate) fn observed_at(&self) -> &str {
        &self.observed_at
    }

    pub(crate) fn ledger(&self) -> &[SourceLedgerEntry] {
        &self.ledger
    }

    pub(crate) fn validated_sources(&self) -> &[ValidatedSource] {
        &self.validated_sources
    }

    pub(crate) fn degraded_sections(&self) -> &[BriefSection] {
        &self.degraded_sections
    }

    pub(crate) fn limitations(&self) -> &[String] {
        &self.limitations
    }

    pub(crate) fn retrieval_intents(&self) -> &[FixedRetrievalIntent] {
        &self.retrieval_intents
    }

    pub(crate) fn rag_catalogue(&self) -> &[String] {
        self.snapshot.logical_collections()
    }

    pub(crate) const fn rag_snapshot_assurance(&self) -> RagSnapshotAssurance {
        self.snapshot.assurance()
    }

    #[allow(
        dead_code,
        reason = "Task 8 constructs the Task 5 production source adapter"
    )]
    pub(crate) fn snapshot_binding(&self) -> &VerifiedRagSnapshot {
        &self.snapshot
    }

    #[cfg(test)]
    pub(crate) fn for_orchestrator_test(
        run_id: &str,
        snapshot_id: &str,
        observed_at: &str,
        ledger: Vec<SourceLedgerEntry>,
        degraded_sections: Vec<BriefSection>,
        limitations: Vec<String>,
    ) -> Self {
        let validated_sources = ledger.iter().cloned().map(ValidatedSource::from).collect();
        Self {
            run_id: run_id.to_string(),
            snapshot: VerifiedRagSnapshot::for_test(snapshot_id, observed_at, observed_at),
            observed_at: observed_at.to_string(),
            ledger,
            validated_sources,
            degraded_sections,
            limitations,
            retrieval_intents: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_orchestrator_trusted_test(
        run_id: &str,
        snapshot_id: &str,
        observed_at: &str,
        ledger: Vec<SourceLedgerEntry>,
        degraded_sections: Vec<BriefSection>,
        limitations: Vec<String>,
    ) -> Self {
        let validated_sources = ledger.iter().cloned().map(ValidatedSource::from).collect();
        Self {
            run_id: run_id.to_string(),
            snapshot: VerifiedRagSnapshot::from_trusted_lan_observation(
                snapshot_id,
                observed_at,
                vec!["navy-publications".to_string()],
            )
            .expect("valid trusted test snapshot"),
            observed_at: observed_at.to_string(),
            ledger,
            validated_sources,
            degraded_sections,
            limitations,
            retrieval_intents: Vec::new(),
        }
    }
}

/// Trusted source collector which freezes one verified local knowledge view.
pub(crate) struct SourceCollector<B> {
    backend: B,
    run_id: String,
    co_request: String,
    observed_at: String,
    apple_selection: AppleBriefSelection,
    world_monitor: Option<WorldMonitorBriefCollector>,
}

impl<B: SourceBackend> SourceCollector<B> {
    pub(crate) fn new(
        backend: B,
        run_id: &str,
        co_request: &str,
        observed_at: &str,
        apple_selection: AppleBriefSelection,
    ) -> Result<Self, SourceCollectionError> {
        if run_id.is_empty()
            || run_id.trim() != run_id
            || run_id.len() > 256
            || run_id.chars().any(char::is_control)
            || co_request.is_empty()
            || co_request.trim() != co_request
            || co_request.len() > MAX_CO_REQUEST_BYTES
            || co_request.chars().any(char::is_control)
        {
            return Err(SourceCollectionError::InvalidRequest);
        }
        if DateTime::parse_from_rfc3339(observed_at).is_err() {
            return Err(SourceCollectionError::InvalidTime);
        }
        Ok(Self {
            backend,
            run_id: run_id.to_string(),
            co_request: co_request.to_string(),
            observed_at: observed_at.to_string(),
            apple_selection,
            world_monitor: None,
        })
    }

    pub(crate) fn with_world_monitor(mut self, collector: WorldMonitorBriefCollector) -> Self {
        self.world_monitor = Some(collector);
        self
    }

    pub(crate) fn backend(&self) -> &B {
        &self.backend
    }

    pub(crate) fn freeze(&self) -> Result<FrozenSourceContext, SourceCollectionError> {
        self.freeze_with_cancellation(&CancellationToken::new())
    }

    pub(crate) fn freeze_with_cancellation(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<FrozenSourceContext, SourceCollectionError> {
        ensure_collection_active(cancellation)?;
        let snapshot = self.backend.verify_active_rag_snapshot()?;
        ensure_collection_active(cancellation)?;
        if snapshot.physical_collections().is_empty() || snapshot.logical_collections().is_empty() {
            return Err(SourceCollectionError::RagInvalid);
        }
        let intents = fixed_retrieval_intents(&self.co_request, snapshot.assurance());
        let mut candidates = vec![snapshot_catalogue_source(&snapshot)?];
        let mut degraded = BTreeSet::new();
        let mut limitations = BTreeSet::new();
        let command_team_discussions = self.backend.command_team_discussions();
        let mut world_monitor_focus = self.co_request.clone();
        for candidate in &command_team_discussions.candidates {
            if world_monitor_focus.len() >= 16 * 1024 {
                break;
            }
            world_monitor_focus.push('\n');
            let remaining = (16_usize * 1024).saturating_sub(world_monitor_focus.len());
            let (quote, _) = truncate_utf8(&candidate.quote, remaining);
            world_monitor_focus.push_str(&quote);
        }
        ensure_collection_active(cancellation)?;
        candidates.extend(command_team_discussions.candidates);
        limitations.extend(command_team_discussions.limitations);
        if snapshot.assurance() == RagSnapshotAssurance::TrustedLanObserved {
            limitations.insert(
                "Unsigned trusted-LAN evidence was observed directly from the approved LAN services; it is not a signed RAG snapshot or replicated Memory revision."
                    .to_string(),
            );
        }
        ensure_collection_active(cancellation)?;
        let memory_conflict_count = self.backend.memory_conflict_count();
        ensure_collection_active(cancellation)?;
        if memory_conflict_count > 0 {
            degraded.insert(BriefSection::ConflictsAndGaps);
            limitations.insert(format!(
                "{memory_conflict_count} unresolved Memory conflicts were excluded from unattended evidence."
            ));
        }

        let mut rag_available = true;
        let mut memory_available = true;
        let doctrine_collections = snapshot
            .logical_collections()
            .iter()
            .filter(|collection| {
                intents.first().is_some_and(|intent| {
                    intent.doctrine_collections().contains(&collection.as_str())
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        for intent in &intents {
            ensure_collection_active(cancellation)?;
            if rag_available {
                if !doctrine_collections.is_empty() {
                    let doctrine_result = self.backend.collect_rag(
                        &snapshot,
                        intent,
                        intent.doctrine_query(),
                        &doctrine_collections,
                        cancellation,
                    );
                    ensure_collection_active(cancellation)?;
                    match doctrine_result {
                        Ok(value) => {
                            let records = extract_rag_records(
                                &snapshot,
                                intent.doctrine_query(),
                                &value,
                                &self.observed_at,
                                &doctrine_collections,
                            );
                            match records {
                                Ok(records) => candidates.extend(records),
                                Err(_) => {
                                    limitations.insert(format!(
                                        "{:?} doctrine evidence was malformed; broader knowledge retrieval continued.",
                                        intent.adviser()
                                    ));
                                }
                            }
                        }
                        Err(error) => {
                            limitations.insert(format!(
                                "{:?} doctrine lookup was unavailable: {}; broader knowledge retrieval continued.",
                                intent.adviser(),
                                error.code()
                            ));
                        }
                    }
                }
                let result = self.backend.collect_rag(
                    &snapshot,
                    intent,
                    intent.context_query(),
                    snapshot.logical_collections(),
                    cancellation,
                );
                ensure_collection_active(cancellation)?;
                match result {
                    Ok(value) => {
                        let records = extract_rag_records(
                            &snapshot,
                            intent.context_query(),
                            &value,
                            &self.observed_at,
                            snapshot.logical_collections(),
                        );
                        match records {
                            Ok(records) => {
                                candidates.extend(records);
                            }
                            Err(_) => {
                                rag_available = false;
                                degrade_all(
                                    &mut degraded,
                                    &mut limitations,
                                    "RAG evidence was malformed or bound to a different snapshot.",
                                );
                            }
                        }
                    }
                    Err(error) => {
                        rag_available = false;
                        degrade_all(
                            &mut degraded,
                            &mut limitations,
                            &format!("RAG source unavailable: {}.", error.code()),
                        );
                    }
                }
            }

            ensure_collection_active(cancellation)?;
            if memory_available {
                let result = self.backend.collect_memory(intent, cancellation);
                ensure_collection_active(cancellation)?;
                match result {
                    Ok(value) => match snapshot.assurance() {
                        RagSnapshotAssurance::TrustedLanObserved => {
                            match extract_trusted_lan_memory_evidence(&value, &self.observed_at) {
                                Ok(records) => candidates.extend(records),
                                Err(_) => {
                                    memory_available = false;
                                    degrade_all(
                                        &mut degraded,
                                        &mut limitations,
                                        "Memory evidence from the trusted LAN was malformed.",
                                    );
                                }
                            }
                        }
                        RagSnapshotAssurance::SignedSnapshot => {
                            match extract_verified_memory_evidence(&value) {
                                Ok(records) => {
                                    candidates.extend(records.into_iter().map(|record| {
                                        CandidateSource {
                                            source_id: record.revision_hash().to_string(),
                                            source_kind: SourceKind::Memory,
                                            collection: "command_memory".to_string(),
                                            document_id: record.entity_id().to_string(),
                                            chunk_id: record.envelope_hash().to_string(),
                                            timestamp: record.origin_timestamp().to_string(),
                                            location: format!(
                                                "origin {} cursor {} revision {} served by {}",
                                                record.origin_node_id(),
                                                record.cursor(),
                                                record.revision_hash(),
                                                record.serving_node_id()
                                            ),
                                            retrieved_at: record.retrieved_at().to_string(),
                                            observed_at: self.observed_at.clone(),
                                            quote: record.quoted_text().to_string(),
                                        }
                                    }));
                                }
                                Err(_) => {
                                    memory_available = false;
                                    degrade_all(
                                &mut degraded,
                                &mut limitations,
                                "Memory evidence failed revision, envelope, or citation verification.",
                            );
                                }
                            }
                        }
                    },
                    Err(error) => {
                        memory_available = false;
                        degrade_all(
                            &mut degraded,
                            &mut limitations,
                            &format!("Memory source unavailable: {}.", error.code()),
                        );
                    }
                }
            }
        }

        if let Some(world_monitor) = &self.world_monitor {
            ensure_collection_active(cancellation)?;
            let batch =
                world_monitor.collect(&world_monitor_focus, &self.observed_at, cancellation);
            ensure_collection_active(cancellation)?;
            candidates.extend(batch.candidates);
            if batch.quota_limited || !batch.limitations.is_empty() {
                degraded.insert(BriefSection::Intelligence);
            }
            limitations.extend(batch.limitations);
        }

        for request in self
            .apple_selection
            .brief_requests(&self.observed_at)
            .map_err(|_| SourceCollectionError::InvalidTime)?
        {
            ensure_collection_active(cancellation)?;
            let response = self.backend.collect_apple(&request, cancellation)?;
            ensure_collection_active(cancellation)?;
            collect_apple_response(
                &self.apple_selection,
                &request,
                response,
                &mut candidates,
                &mut degraded,
                &mut limitations,
            );
        }

        ensure_collection_active(cancellation)?;
        let mut canonical = canonical_ledger(&self.run_id, snapshot.snapshot_id(), candidates)?;
        limitations.append(&mut canonical.limitations);
        apply_ledger_omissions(&canonical.omitted_by_kind, &mut degraded, &mut limitations);
        apply_ledger_rejections(&canonical.rejected_by_kind, &mut degraded, &mut limitations);
        apply_command_team_ledger_losses(
            canonical.omitted_command_team_discussions,
            canonical.rejected_command_team_discussions,
            &mut limitations,
        );
        let validated_sources = canonical
            .ledger
            .iter()
            .cloned()
            .map(ValidatedSource::from)
            .collect();
        let limitations = bounded_limitations(limitations);
        let mut context = FrozenSourceContext {
            run_id: self.run_id.clone(),
            snapshot,
            observed_at: self.observed_at.clone(),
            ledger: canonical.ledger,
            validated_sources,
            degraded_sections: degraded.into_iter().collect(),
            limitations,
            retrieval_intents: intents,
        };
        self.recheck_snapshot_with_cancellation(&context, cancellation)?;
        let post_recheck_limitations = self.backend.post_recheck_limitations();
        if !post_recheck_limitations.is_empty() {
            let mut rechecked_limitations =
                context.limitations.iter().cloned().collect::<BTreeSet<_>>();
            rechecked_limitations.extend(post_recheck_limitations);
            context.limitations = bounded_limitations(rechecked_limitations);
        }
        Ok(context)
    }

    /// Rechecks the signed active snapshot before consolidation and persistence.
    pub(crate) fn recheck_snapshot(
        &self,
        context: &FrozenSourceContext,
    ) -> Result<(), SourceCollectionError> {
        self.recheck_snapshot_with_cancellation(context, &CancellationToken::new())
    }

    pub(crate) fn recheck_snapshot_with_cancellation(
        &self,
        context: &FrozenSourceContext,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceCollectionError> {
        ensure_collection_active(cancellation)?;
        let result = self
            .backend
            .recheck_rag_snapshot(&context.snapshot, cancellation);
        ensure_collection_active(cancellation)?;
        result
    }
}

fn ensure_collection_active(cancellation: &CancellationToken) -> Result<(), SourceCollectionError> {
    if cancellation.is_cancelled() {
        Err(SourceCollectionError::Cancelled)
    } else {
        Ok(())
    }
}

mod backend_tools;
use backend_tools::AuthenticatedMcpSourceCaller;
pub(crate) use backend_tools::SourceToolCaller;
mod canonical;
use canonical::*;
mod command_team_discussions;
pub(crate) use command_team_discussions::{
    load_command_team_discussions, CommandTeamDiscussionBatch,
};
mod errors;
pub(crate) use errors::{SourceCollectionError, SourceReadError};
mod limitations;
use limitations::*;
mod rag_collection;
use rag_collection::extract_rag_records;
mod retrieval_intents;
use retrieval_intents::fixed_retrieval_intents;
pub(crate) use retrieval_intents::FixedRetrievalIntent;
mod trusted_lan_evidence;
use trusted_lan_evidence::*;
mod world_monitor;
pub(crate) use world_monitor::WorldMonitorBriefCollector;
