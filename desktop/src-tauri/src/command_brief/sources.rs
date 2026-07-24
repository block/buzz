#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Phase 5 wires the frozen source collector into the run orchestrator"
    )
)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::DateTime;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::personas::specialist_definitions;
use super::provenance::ValidatedSource;
use super::types::{
    AdviserId, BriefSection, SourceKind, SourceLedgerEntry, MAX_ARRAY_ITEMS,
    MAX_SOURCE_LEDGER_ITEMS, MAX_TEXT_BYTES,
};
use crate::command_services::apple_inputs::{
    AppleBriefSelection, AppleInputPermission, AppleInputRequest, AppleInputResponse,
};
use crate::command_services::memory::sanitize_command_memory_context;
use crate::command_services::rag::{
    extract_verified_rag_evidence, RagSnapshotError, VerifiedRagSnapshot,
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
pub(crate) trait SourceBackend {
    fn verify_active_rag_snapshot(&self) -> Result<VerifiedRagSnapshot, SourceCollectionError>;

    fn memory_conflict_count(&self) -> u64;

    fn collect_rag(
        &self,
        snapshot: &VerifiedRagSnapshot,
        intent: &FixedRetrievalIntent,
    ) -> Result<Value, SourceReadError>;

    fn collect_memory(&self, intent: &FixedRetrievalIntent) -> Result<Vec<Value>, SourceReadError>;

    fn collect_apple(&self, request: &AppleInputRequest) -> AppleInputResponse;

    fn recheck_rag_snapshot(
        &self,
        expected: &VerifiedRagSnapshot,
    ) -> Result<(), SourceCollectionError>;
}

/// All source evidence frozen before any adviser runs.
pub(crate) struct FrozenSourceContext {
    snapshot: VerifiedRagSnapshot,
    observed_at: String,
    ledger: Vec<SourceLedgerEntry>,
    validated_sources: Vec<ValidatedSource>,
    degraded_sections: Vec<BriefSection>,
    limitations: Vec<String>,
    retrieval_intents: Vec<FixedRetrievalIntent>,
}

impl FrozenSourceContext {
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
        self.snapshot.collections()
    }
}

/// Trusted source collector which freezes one verified local knowledge view.
pub(crate) struct SourceCollector<B> {
    backend: B,
    co_request: String,
    observed_at: String,
    apple_selection: AppleBriefSelection,
}

impl<B: SourceBackend> SourceCollector<B> {
    pub(crate) fn new(
        backend: B,
        co_request: &str,
        observed_at: &str,
        apple_selection: AppleBriefSelection,
    ) -> Result<Self, SourceCollectionError> {
        if co_request.is_empty()
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
            co_request: co_request.to_string(),
            observed_at: observed_at.to_string(),
            apple_selection,
        })
    }

    pub(crate) fn backend(&self) -> &B {
        &self.backend
    }

    pub(crate) fn freeze(&self) -> Result<FrozenSourceContext, SourceCollectionError> {
        let snapshot = self.backend.verify_active_rag_snapshot()?;
        if snapshot.collections().is_empty() {
            return Err(SourceCollectionError::RagInvalid);
        }
        let intents = fixed_retrieval_intents(&self.co_request);
        let mut candidates = vec![snapshot_catalogue_source(&snapshot)?];
        let mut degraded = BTreeSet::new();
        let mut limitations = BTreeSet::new();
        let memory_conflict_count = self.backend.memory_conflict_count();
        if memory_conflict_count > 0 {
            degraded.insert(BriefSection::ConflictsAndGaps);
            limitations.insert(format!(
                "{memory_conflict_count} unresolved Memory conflicts were excluded from unattended evidence."
            ));
        }

        let mut rag_available = true;
        let mut memory_available = true;
        for intent in &intents {
            if rag_available {
                match self.backend.collect_rag(&snapshot, intent) {
                    Ok(value) => match extract_verified_rag_evidence(&snapshot, &value) {
                        Ok(records) => {
                            candidates.extend(records.into_iter().map(|record| CandidateSource {
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
                    },
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

            if memory_available {
                match self.backend.collect_memory(intent) {
                    Ok(records) => {
                        for value in records {
                            match sanitize_command_memory_context(&value) {
                                Ok(record) => {
                                    if !record.conflicted_fields().is_empty() {
                                        degraded.insert(BriefSection::ConflictsAndGaps);
                                        for field in record.conflicted_fields() {
                                            limitations.insert(format!(
                                                "Memory conflict excluded field {field} from entity {}.",
                                                record.entity_id()
                                            ));
                                        }
                                    }
                                    let quote = serde_jcs::to_vec(record.content())
                                        .ok()
                                        .and_then(|bytes| String::from_utf8(bytes).ok());
                                    if let Some(quote) = quote.filter(|quote| quote != "{}") {
                                        candidates.push(CandidateSource {
                                            source_id: record.event_id().to_string(),
                                            source_kind: SourceKind::Memory,
                                            collection: "command_memory".to_string(),
                                            document_id: record.entity_id().to_string(),
                                            chunk_id: record.event_id().to_string(),
                                            timestamp: record.timestamp().to_string(),
                                            location: format!(
                                                "entity {} conflict-safe revision",
                                                record.entity_id()
                                            ),
                                            retrieved_at: self.observed_at.clone(),
                                            observed_at: self.observed_at.clone(),
                                            quote,
                                        });
                                    }
                                }
                                Err(_) => {
                                    memory_available = false;
                                    degrade_all(
                                        &mut degraded,
                                        &mut limitations,
                                        "Memory command context was invalid or unsafe.",
                                    );
                                    break;
                                }
                            }
                        }
                    }
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
            collect_apple_response(
                &self.apple_selection,
                &request,
                self.backend.collect_apple(&request),
                &mut candidates,
                &mut degraded,
                &mut limitations,
            );
        }

        let (ledger, mut canonical_limitations) =
            canonical_ledger(snapshot.snapshot_id(), candidates)?;
        limitations.append(&mut canonical_limitations);
        let validated_sources = ledger.iter().cloned().map(ValidatedSource::from).collect();
        let limitations = bounded_limitations(limitations);
        let context = FrozenSourceContext {
            snapshot,
            observed_at: self.observed_at.clone(),
            ledger,
            validated_sources,
            degraded_sections: degraded.into_iter().collect(),
            limitations,
            retrieval_intents: intents,
        };
        self.recheck_snapshot(&context)?;
        Ok(context)
    }

    /// Rechecks the signed active snapshot before consolidation and persistence.
    pub(crate) fn recheck_snapshot(
        &self,
        context: &FrozenSourceContext,
    ) -> Result<(), SourceCollectionError> {
        self.backend.recheck_rag_snapshot(&context.snapshot)
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
        "collections": snapshot.collections(),
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
    if response.source_name() != expected_source
        || response.permission() != AppleInputPermission::Authorized
        || response.error().is_some()
    {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input is {}.",
            response.permission().name()
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
        if let Some(candidate) = apple_candidate(expected_source, response.observed_at(), fields) {
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
    source: &str,
    observed_at: &str,
    fields: &BTreeMap<String, String>,
) -> Option<CandidateSource> {
    let (kind, identity, collection, location) = match source {
        "calendar" => {
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
            let identifier = fields.get("identifier")?;
            (
                SourceKind::Notes,
                format!("note:{identifier}"),
                fields.get("folder_identifier")?.clone(),
                format!("note {identifier}"),
            )
        }
        "files" => {
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

fn canonical_ledger(
    snapshot_id: &str,
    candidates: Vec<CandidateSource>,
) -> Result<(Vec<SourceLedgerEntry>, BTreeSet<String>), SourceCollectionError> {
    let mut by_source = BTreeMap::<String, CandidateSource>::new();
    let mut limitations = BTreeSet::new();
    for mut candidate in candidates {
        let (quote, truncated) = truncate_utf8(candidate.quote.trim(), MAX_TEXT_BYTES);
        if quote.is_empty() {
            continue;
        }
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
    if candidates.len() > MAX_SOURCE_LEDGER_ITEMS {
        candidates.truncate(MAX_SOURCE_LEDGER_ITEMS);
        limitations.insert(format!(
            "Source ledger was truncated to {MAX_SOURCE_LEDGER_ITEMS} entries."
        ));
    }
    let ledger = candidates
        .into_iter()
        .map(|candidate| {
            let ledger_id = format!(
                "source-{}",
                &digest_text(&format!(
                    "{}:{}:{snapshot_id}",
                    source_priority(candidate.source_kind),
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
            SourceLedgerEntry::parse_for_snapshot(value, snapshot_id)
                .map_err(|_| SourceCollectionError::RagInvalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((ledger, limitations))
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
