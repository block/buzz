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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CandidateSource {
    source_id: String,
    source_kind: SourceKind,
    collection: String,
    document_id: String,
    chunk_id: String,
    timestamp: String,
    location: String,
    retrieved_at: String,
    observed_at: String,
    quote: String,
}

fn snapshot_catalogue_source(
    snapshot: &VerifiedRagSnapshot,
) -> Result<CandidateSource, SourceCollectionError> {
    let quote = serde_jcs::to_vec(&json!({
        "active_snapshot_id": snapshot.snapshot_id(),
        "logical_collections": snapshot.logical_collections(),
        "physical_collections": snapshot.physical_collections(),
        "snapshot_time": snapshot.snapshot_time(),
        "verified_at": snapshot.verified_at(),
    }))
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())
    .ok_or(SourceCollectionError::RagInvalid)?;
    Ok(CandidateSource {
        source_id: format!("rag:snapshot:{}", snapshot.snapshot_id()),
        source_kind: SourceKind::Rag,
        collection: "verified_catalogue".to_string(),
        document_id: snapshot.snapshot_id().to_string(),
        chunk_id: "active_snapshot".to_string(),
        timestamp: snapshot.snapshot_time().to_string(),
        location: "cryptographically verified active snapshot catalogue".to_string(),
        retrieved_at: snapshot.verified_at().to_string(),
        observed_at: snapshot.verified_at().to_string(),
        quote,
    })
}

fn collect_apple_response(
    selection: &AppleBriefSelection,
    request: &AppleInputRequest,
    response: AppleInputResponse,
    candidates: &mut Vec<CandidateSource>,
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
) {
    let expected_source = request.source_name();
    if response.source_name() != expected_source {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input failed signed-helper source binding."
        ));
        return;
    }
    if response.permission() != AppleInputPermission::Authorized {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input permission is {}.",
            response.permission().name()
        ));
        return;
    }
    if response.error().is_some() {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input failed in the signed helper."
        ));
        return;
    }
    if DateTime::parse_from_rfc3339(response.observed_at()).is_err() {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input had an invalid observation time."
        ));
        return;
    }
    if response.truncated() {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input was truncated by the signed helper."
        ));
    }
    for record in response.records() {
        let fields = record.fields();
        if !selection.permits_record(expected_source, fields) {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "Apple {expected_source} returned a record outside the protected allowlist."
            ));
            continue;
        }
        let deleted = bool_field(fields, "is_deleted");
        let stale = bool_field(fields, "is_stale");
        if deleted == Some(true) {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "A deleted Apple {expected_source} record was excluded."
            ));
            continue;
        }
        if stale == Some(true) {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "A stale Apple {expected_source} record was excluded."
            ));
            continue;
        }
        if deleted.is_none() && fields.contains_key("is_deleted")
            || stale.is_none() && fields.contains_key("is_stale")
        {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "An Apple {expected_source} record had invalid freshness metadata."
            ));
            continue;
        }
        if let Some(candidate) = apple_candidate(request, response.observed_at(), fields) {
            candidates.push(candidate);
        } else {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!("An Apple {expected_source} record was malformed."));
        }
    }
}

fn bool_field(fields: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|value| match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

fn apple_candidate(
    request: &AppleInputRequest,
    observed_at: &str,
    fields: &BTreeMap<String, String>,
) -> Option<CandidateSource> {
    let source = request.source_name();
    let (kind, identity, collection, location) = match source {
        "calendar" => {
            if !exact_apple_fields(
                fields,
                &[
                    "identifier",
                    "calendar_identifier",
                    "title",
                    "start",
                    "end",
                    "is_recurring",
                    "recurrence_identifier",
                    "is_deleted",
                    "is_stale",
                ],
            ) || bool_field(fields, "is_recurring").is_none()
                || bool_field(fields, "is_deleted").is_none()
                || bool_field(fields, "is_stale").is_none()
                || !calendar_record_in_window(request, fields)
            {
                return None;
            }
            let identifier = fields.get("identifier")?;
            let recurrence = fields.get("recurrence_identifier")?;
            let location = if recurrence.is_empty() {
                format!("calendar event {identifier}")
            } else {
                format!("calendar event {identifier} recurrence {recurrence}")
            };
            (
                SourceKind::Calendar,
                format!("calendar:{identifier}:{recurrence}"),
                fields.get("calendar_identifier")?.clone(),
                location,
            )
        }
        "reminders" => {
            if !exact_apple_fields(
                fields,
                &[
                    "identifier",
                    "list_identifier",
                    "title",
                    "is_completed",
                    "recurrence_identifier",
                    "due_date",
                    "completion_date",
                    "is_deleted",
                    "is_stale",
                ],
            ) || bool_field(fields, "is_completed").is_none()
                || bool_field(fields, "is_deleted").is_none()
                || bool_field(fields, "is_stale").is_none()
                || !reminder_record_in_window(request, fields)
            {
                return None;
            }
            let identifier = fields.get("identifier")?;
            let recurrence = fields.get("recurrence_identifier")?;
            let location = if recurrence.is_empty() {
                format!("reminder {identifier}")
            } else {
                format!("reminder {identifier} recurrence {recurrence}")
            };
            (
                SourceKind::Reminders,
                format!("reminder:{identifier}:{recurrence}"),
                fields.get("list_identifier")?.clone(),
                location,
            )
        }
        "notes" => {
            if !exact_apple_fields(
                fields,
                &["identifier", "folder_identifier", "title", "body"],
            ) {
                return None;
            }
            let identifier = fields.get("identifier")?;
            (
                SourceKind::Notes,
                format!("note:{identifier}"),
                fields.get("folder_identifier")?.clone(),
                format!("note {identifier}"),
            )
        }
        "files" => {
            if !exact_apple_fields(fields, &["path", "contents", "device", "inode"])
                || fields
                    .get("device")
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_none()
                || fields
                    .get("inode")
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_none()
            {
                return None;
            }
            let path = fields.get("path")?;
            let identity = format!(
                "file:{}",
                digest_text(&format!(
                    "{path}:{}:{}",
                    fields.get("device")?,
                    fields.get("inode")?
                ))
            );
            (
                SourceKind::File,
                identity,
                "approved_files".to_string(),
                path.clone(),
            )
        }
        _ => return None,
    };
    let quote = serde_jcs::to_vec(fields)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())?;
    Some(CandidateSource {
        source_id: identity.clone(),
        source_kind: kind,
        collection,
        document_id: identity.clone(),
        chunk_id: identity,
        timestamp: observed_at.to_string(),
        location,
        retrieved_at: observed_at.to_string(),
        observed_at: observed_at.to_string(),
        quote,
    })
}

fn exact_apple_fields(fields: &BTreeMap<String, String>, expected: &[&str]) -> bool {
    fields.len() == expected.len()
        && expected.iter().all(|key| fields.contains_key(*key))
        && fields.iter().all(|(key, value)| {
            value.len() <= 1024 * 1024
                && (matches!(
                    key.as_str(),
                    "title"
                        | "body"
                        | "contents"
                        | "recurrence_identifier"
                        | "due_date"
                        | "completion_date"
                ) || (!value.is_empty()
                    && value.trim() == value
                    && !value.chars().any(char::is_control)))
        })
}

fn calendar_record_in_window(
    request: &AppleInputRequest,
    fields: &BTreeMap<String, String>,
) -> bool {
    let Some((window_start, window_end)) = request.read_window() else {
        return false;
    };
    let (Ok(window_start), Ok(window_end), Some(start), Some(end)) = (
        DateTime::parse_from_rfc3339(window_start),
        DateTime::parse_from_rfc3339(window_end),
        fields
            .get("start")
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
        fields
            .get("end")
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
    ) else {
        return false;
    };
    start < end && start < window_end && end > window_start
}

fn reminder_record_in_window(
    request: &AppleInputRequest,
    fields: &BTreeMap<String, String>,
) -> bool {
    let Some((window_start, window_end)) = request.read_window() else {
        return false;
    };
    let (Ok(window_start), Ok(window_end), Some(completed)) = (
        DateTime::parse_from_rfc3339(window_start),
        DateTime::parse_from_rfc3339(window_end),
        bool_field(fields, "is_completed"),
    ) else {
        return false;
    };
    let parse_optional = |key: &str| {
        fields.get(key).and_then(|value| {
            if value.is_empty() {
                Some(None)
            } else {
                DateTime::parse_from_rfc3339(value).ok().map(Some)
            }
        })
    };
    let (Some(due), Some(completion)) = (
        parse_optional("due_date"),
        parse_optional("completion_date"),
    ) else {
        return false;
    };
    let inside = |value: &DateTime<_>| *value >= window_start && *value < window_end;
    if completed {
        completion.as_ref().is_some_and(inside)
    } else {
        due.as_ref().is_some_and(inside)
    }
}

fn canonical_ledger(
    run_id: &str,
    snapshot_id: &str,
    candidates: Vec<CandidateSource>,
) -> Result<CanonicalLedgerOutput, SourceCollectionError> {
    let mut by_source = BTreeMap::<String, CandidateSource>::new();
    let mut limitations = BTreeSet::new();
    let mut rejected_by_kind = [0_usize; 6];
    for mut candidate in candidates {
        let Some((quote, truncated)) = canonical_quote(&candidate.quote, MAX_TEXT_BYTES) else {
            rejected_by_kind[source_priority(candidate.source_kind) as usize] += 1;
            continue;
        };
        candidate.quote = quote;
        if truncated {
            limitations.insert(format!(
                "Source {} was truncated to the canonical source-size limit.",
                candidate.source_id
            ));
        }
        match by_source.get_mut(&candidate.source_id) {
            None => {
                by_source.insert(candidate.source_id.clone(), candidate);
            }
            Some(existing) if same_source_content(existing, &candidate) => {
                if candidate.retrieved_at < existing.retrieved_at {
                    existing.retrieved_at = candidate.retrieved_at;
                }
                if candidate.observed_at < existing.observed_at {
                    existing.observed_at = candidate.observed_at;
                }
            }
            Some(_) => return Err(SourceCollectionError::ConflictingSourceIdentity),
        }
    }
    let mut candidates = by_source.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        source_priority(left.source_kind)
            .cmp(&source_priority(right.source_kind))
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    let mut omitted_by_kind = [0_usize; 6];
    if candidates.len() > MAX_SOURCE_LEDGER_ITEMS {
        for candidate in &candidates[MAX_SOURCE_LEDGER_ITEMS..] {
            omitted_by_kind[source_priority(candidate.source_kind) as usize] += 1;
        }
        candidates.truncate(MAX_SOURCE_LEDGER_ITEMS);
    }
    let mut ledger = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let priority = source_priority(candidate.source_kind);
        let ledger_id = format!(
            "source-{}",
            &digest_text(&format!(
                "{run_id}:{priority}:{}:{snapshot_id}",
                candidate.source_id
            ))[..24]
        );
        let value = json!({
            "classification": "OFFICIAL",
            "ledgerId": ledger_id,
            "sourceId": candidate.source_id,
            "sourceKind": candidate.source_kind,
            "collection": candidate.collection,
            "documentId": candidate.document_id,
            "chunkId": candidate.chunk_id,
            "timestamp": candidate.timestamp,
            "snapshotId": snapshot_id,
            "quotedLocation": {
                "quote": candidate.quote,
                "location": candidate.location
            },
            "retrievedAt": candidate.retrieved_at,
            "observedAt": candidate.observed_at
        });
        match SourceLedgerEntry::parse_for_snapshot(value, snapshot_id) {
            Ok(entry) => ledger.push(entry),
            Err(_) => rejected_by_kind[priority as usize] += 1,
        }
    }
    Ok(CanonicalLedgerOutput {
        ledger,
        limitations,
        omitted_by_kind,
        rejected_by_kind,
    })
}

struct CanonicalLedgerOutput {
    ledger: Vec<SourceLedgerEntry>,
    limitations: BTreeSet<String>,
    omitted_by_kind: [usize; 6],
    rejected_by_kind: [usize; 6],
}

fn apply_ledger_omissions(
    omitted_by_kind: &[usize; 6],
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
) {
    for (kind, count) in [
        (SourceKind::Rag, omitted_by_kind[0]),
        (SourceKind::Memory, omitted_by_kind[1]),
        (SourceKind::Calendar, omitted_by_kind[2]),
        (SourceKind::Reminders, omitted_by_kind[3]),
        (SourceKind::Notes, omitted_by_kind[4]),
        (SourceKind::File, omitted_by_kind[5]),
    ] {
        if count == 0 {
            continue;
        }
        limitations.insert(format!(
            "{count} {} sources were omitted by the canonical ledger limit.",
            source_kind_name(kind)
        ));
        match kind {
            SourceKind::Rag | SourceKind::Memory => degraded.extend(RAG_MEMORY_SECTIONS),
            SourceKind::Calendar | SourceKind::Reminders | SourceKind::Notes | SourceKind::File => {
                degraded.insert(BriefSection::DailyRoutine);
            }
        }
    }
}

fn apply_ledger_rejections(
    rejected_by_kind: &[usize; 6],
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
) {
    for (kind, count) in [
        (SourceKind::Rag, rejected_by_kind[0]),
        (SourceKind::Memory, rejected_by_kind[1]),
        (SourceKind::Calendar, rejected_by_kind[2]),
        (SourceKind::Reminders, rejected_by_kind[3]),
        (SourceKind::Notes, rejected_by_kind[4]),
        (SourceKind::File, rejected_by_kind[5]),
    ] {
        if count == 0 {
            continue;
        }
        limitations.insert(format!(
            "{count} malformed {} sources were excluded from the canonical ledger.",
            source_kind_name(kind)
        ));
        match kind {
            SourceKind::Rag | SourceKind::Memory => degraded.extend(RAG_MEMORY_SECTIONS),
            SourceKind::Calendar | SourceKind::Reminders | SourceKind::Notes | SourceKind::File => {
                degraded.insert(BriefSection::DailyRoutine);
            }
        }
    }
}

const fn source_kind_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Rag => "RAG",
        SourceKind::Memory => "Memory",
        SourceKind::Calendar => "calendar",
        SourceKind::Reminders => "reminder",
        SourceKind::Notes => "note",
        SourceKind::File => "file",
    }
}

fn same_source_content(left: &CandidateSource, right: &CandidateSource) -> bool {
    left.source_kind == right.source_kind
        && left.collection == right.collection
        && left.document_id == right.document_id
        && left.chunk_id == right.chunk_id
        && left.timestamp == right.timestamp
        && left.location == right.location
        && left.quote == right.quote
}

const fn source_priority(kind: SourceKind) -> u8 {
    match kind {
        SourceKind::Rag => 0,
        SourceKind::Memory => 1,
        SourceKind::Calendar => 2,
        SourceKind::Reminders => 3,
        SourceKind::Notes => 4,
        SourceKind::File => 5,
    }
}

fn digest_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn canonical_quote(value: &str, maximum_bytes: usize) -> Option<(String, bool)> {
    if value.trim().is_empty() {
        return None;
    }
    let encoded = serde_json::to_string(value).ok()?;
    if encoded.len() <= maximum_bytes {
        return Some((encoded, false));
    }

    let boundaries = value
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .collect::<Vec<_>>();
    let mut low = 0;
    let mut high = boundaries.len();
    let mut best = None;
    while low < high {
        let middle = low + (high - low) / 2;
        let end = boundaries[middle];
        let candidate = serde_json::to_string(&value[..end]).ok()?;
        if candidate.len() <= maximum_bytes {
            best = Some(candidate);
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    best.map(|candidate| (candidate, true))
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> (String, bool) {
    if value.len() <= maximum_bytes {
        return (value.to_string(), false);
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

impl From<RagSnapshotError> for SourceCollectionError {
    fn from(value: RagSnapshotError) -> Self {
        match value {
            RagSnapshotError::Invalid => Self::RagInvalid,
            RagSnapshotError::Changed => Self::SnapshotChanged,
        }
    }
}
