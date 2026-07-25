#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 5 wires the frozen source collector into the run orchestrator"
    )
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::DateTime;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::personas::specialist_definitions;
use super::provenance::ValidatedSource;
use super::types::{
    AdviserId, BriefSection, SourceKind, SourceLedgerEntry, MAX_ARRAY_ITEMS,
    MAX_SOURCE_LEDGER_ITEMS, MAX_TEXT_BYTES,
};
use crate::command_services::apple_inputs::{
    read_apple_inputs_blocking, AppleBriefSelection, AppleInputPermission, AppleInputRequest,
    AppleInputResponse,
};
use crate::command_services::memory::{
    extract_verified_memory_evidence, get_memory_source_binding,
};
use crate::command_services::policy::{AdmissionError, AuthenticatedSourceService};
use crate::command_services::rag::{
    extract_verified_rag_evidence, get_rag_source_binding, RagSnapshotError, VerifiedRagSnapshot,
};

const MAX_CO_REQUEST_BYTES: usize = 1024;
const MAX_RETRIEVAL_QUERY_BYTES: usize = 2048;
const RAG_TOOL: &str = "search_knowledge_base";
const MEMORY_TOOL: &str = "command_memory_context";
const COLLECTION_SCOPE: &str = "verified_catalogue";

const RAG_MEMORY_SECTIONS: [BriefSection; 5] = [
    BriefSection::Operations,
    BriefSection::Navigation,
    BriefSection::DailyRoutine,
    BriefSection::Reports,
    BriefSection::Planning306090,
];

/// Stable, redacted failures which the orchestrator may expose as run status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceCollectionError {
    Cancelled,
    InvalidRequest,
    InvalidTime,
    RagUnavailable,
    RagStale,
    RagInvalid,
    SnapshotChanged,
    ConflictingSourceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceReadError {
    code: &'static str,
}

impl SourceReadError {
    pub(crate) const fn new(code: &'static str) -> Self {
        Self { code }
    }

    fn code(self) -> &'static str {
        self.code
    }
}

/// One native-owned, bounded retrieval request for a fixed specialist.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FixedRetrievalIntent {
    adviser: AdviserId,
    rag_tool: &'static str,
    memory_tool: &'static str,
    collection_scope: &'static str,
    query: String,
}

impl FixedRetrievalIntent {
    pub(crate) const fn adviser(&self) -> AdviserId {
        self.adviser
    }

    pub(crate) const fn rag_tool(&self) -> &'static str {
        self.rag_tool
    }

    pub(crate) const fn memory_tool(&self) -> &'static str {
        self.memory_tool
    }

    pub(crate) const fn collection_scope(&self) -> &'static str {
        self.collection_scope
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }
}

/// Backend seam implemented by the admitted local RAG, Memory, and signed Apple services.
pub(crate) trait SourceBackend: Send + Sync {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError>;

    fn memory_conflict_count(&self) -> u64;

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
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

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
        cancellation: &CancellationToken,
    ) -> Result<Value, SourceReadError> {
        (**self).collect_rag(snapshot, intent, cancellation)
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
}

pub(crate) trait SourceToolCaller: Send + Sync {
    fn call(
        &self,
        service: &AuthenticatedSourceService,
        tool_name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, AdmissionError>;
}

#[allow(
    dead_code,
    reason = "Task 8 installs the production orchestrator into AppState"
)]
struct AuthenticatedMcpSourceCaller;

impl SourceToolCaller for AuthenticatedMcpSourceCaller {
    fn call(
        &self,
        service: &AuthenticatedSourceService,
        tool_name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, AdmissionError> {
        service.call(tool_name, arguments, cancellation)
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
            caller: Arc::new(AuthenticatedMcpSourceCaller),
        })
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

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
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
                    "query": intent.query(),
                    "collections": snapshot.logical_collections(),
                    "top_k": 8,
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
                json!({"query": intent.query(), "limit": 10}),
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
}

/// Trusted source collector which freezes one verified local knowledge view.
pub(crate) struct SourceCollector<B> {
    backend: B,
    run_id: String,
    co_request: String,
    observed_at: String,
    apple_selection: AppleBriefSelection,
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
        })
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
        let intents = fixed_retrieval_intents(&self.co_request);
        let mut candidates = vec![snapshot_catalogue_source(&snapshot)?];
        let mut degraded = BTreeSet::new();
        let mut limitations = BTreeSet::new();
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
        for intent in &intents {
            ensure_collection_active(cancellation)?;
            if rag_available {
                let result = self.backend.collect_rag(&snapshot, intent, cancellation);
                ensure_collection_active(cancellation)?;
                match result {
                    Ok(value) => {
                        match extract_verified_rag_evidence(&snapshot, intent.query(), &value) {
                            Ok(records) => {
                                candidates.extend(records.into_iter().map(|record| {
                                    CandidateSource {
                                        source_id: record.source_id,
                                        source_kind: SourceKind::Rag,
                                        collection: record.collection,
                                        document_id: record.document_id,
                                        chunk_id: record.chunk_id,
                                        timestamp: record.retrieved_at.clone(),
                                        location: record.location,
                                        retrieved_at: record.retrieved_at,
                                        observed_at: self.observed_at.clone(),
                                        quote: record.quote,
                                    }
                                }));
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
                    Ok(value) => match extract_verified_memory_evidence(&value) {
                        Ok(records) => {
                            candidates.extend(records.into_iter().map(|record| CandidateSource {
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
        let validated_sources = canonical
            .ledger
            .iter()
            .cloned()
            .map(ValidatedSource::from)
            .collect();
        let limitations = bounded_limitations(limitations);
        let context = FrozenSourceContext {
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

fn fixed_retrieval_intents(co_request: &str) -> Vec<FixedRetrievalIntent> {
    specialist_definitions()
        .iter()
        .map(|persona| {
            let prefix = format!(
                "{} Use only the verified local catalogue and conflict-safe command memory. CO request: ",
                persona.purpose
            );
            let remaining = MAX_RETRIEVAL_QUERY_BYTES.saturating_sub(prefix.len());
            let (request, _) = truncate_utf8(co_request, remaining);
            FixedRetrievalIntent {
                adviser: persona.adviser,
                rag_tool: RAG_TOOL,
                memory_tool: MEMORY_TOOL,
                collection_scope: COLLECTION_SCOPE,
                query: format!("{prefix}{request}"),
            }
        })
        .collect()
}

fn degrade_all(
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
    limitation: &str,
) {
    degraded.extend(RAG_MEMORY_SECTIONS);
    limitations.insert(limitation.to_string());
}

fn bounded_limitations(limitations: BTreeSet<String>) -> Vec<String> {
    if limitations.len() <= MAX_ARRAY_ITEMS {
        return limitations.into_iter().collect();
    }
    let omitted = limitations.len() - (MAX_ARRAY_ITEMS - 1);
    let mut bounded = limitations
        .into_iter()
        .take(MAX_ARRAY_ITEMS - 1)
        .collect::<Vec<_>>();
    bounded.push(format!(
        "{omitted} additional source limitations omitted after the canonical limit."
    ));
    bounded
}

mod canonical;
use canonical::*;
