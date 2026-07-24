use std::cmp::Ordering;

use serde::Serialize;

use super::personas::PersonaDefinition;
use super::types::{SourceKind, SourceLedgerEntry};

/// Maximum UTF-8 bytes retained from one source quote in a model-visible envelope.
pub const MAX_EVIDENCE_ENVELOPE_QUOTE_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes for one complete model-visible evidence envelope.
pub const MAX_EVIDENCE_ENVELOPE_BYTES: usize = 6 * 1024;
/// Maximum UTF-8 bytes for the complete model-visible evidence JSON payload.
pub const MAX_PROMPT_EVIDENCE_BYTES: usize = 16 * 1024;

/// Trusted source metadata admitted by native collection before prompt rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSource {
    ledger_id: String,
    source_kind: SourceKind,
    source_id: String,
    collection: String,
    document_id: String,
    chunk_id: String,
    snapshot_id: String,
    observed_at: String,
    retrieved_at: String,
    location: String,
    quote: String,
}

impl From<SourceLedgerEntry> for ValidatedSource {
    fn from(source: SourceLedgerEntry) -> Self {
        Self {
            ledger_id: source.ledger_id().to_string(),
            source_kind: source.source_kind(),
            source_id: source.source_id().to_string(),
            collection: source.collection().to_string(),
            document_id: source.document_id().to_string(),
            chunk_id: source.chunk_id().to_string(),
            snapshot_id: source.snapshot_id().to_string(),
            observed_at: source.observed_at().to_string(),
            retrieved_at: source.retrieved_at().to_string(),
            location: source.location().to_string(),
            quote: source.quote().to_string(),
        }
    }
}

impl ValidatedSource {
    pub(super) fn ledger_id(&self) -> &str {
        &self.ledger_id
    }

    pub(super) fn source_id(&self) -> &str {
        &self.source_id
    }

    pub(super) const fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    pub(super) fn collection(&self) -> &str {
        &self.collection
    }

    pub(super) fn document_id(&self) -> &str {
        &self.document_id
    }

    pub(super) fn chunk_id(&self) -> &str {
        &self.chunk_id
    }

    pub(super) fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    pub(super) fn observed_at(&self) -> &str {
        &self.observed_at
    }

    pub(super) fn retrieved_at(&self) -> &str {
        &self.retrieved_at
    }

    pub(super) fn location(&self) -> &str {
        &self.location
    }

    pub(super) fn quote(&self) -> &str {
        &self.quote
    }
}

/// A bounded, explicitly inert representation of one source for a model prompt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvidenceEnvelope {
    pub ledger_id: String,
    pub source_kind: SourceKind,
    pub source_id: String,
    pub collection: String,
    pub document_id: String,
    pub chunk_id: String,
    pub snapshot_id: String,
    pub observed_at: String,
    pub retrieved_at: String,
    pub location: String,
    pub quote: String,
    pub untrusted_evidence: bool,
}

/// The fixed prompt and bounded evidence payload selected exclusively by native Rust.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidencePrompt {
    pub system_prompt: &'static str,
    pub evidence_json: String,
    pub envelopes: Vec<EvidenceEnvelope>,
    pub limitations: Vec<String>,
    pub total_bytes: usize,
}

/// Builds the model-visible evidence payload without accepting a renderer prompt,
/// persona name, tool policy, or source-policy override.
pub fn build_evidence_prompt(
    persona: &'static PersonaDefinition,
    sources: &[ValidatedSource],
) -> EvidencePrompt {
    let mut ordered = sources.to_vec();
    ordered.sort_by(source_order);

    let mut envelopes = Vec::new();
    let mut limitations = Vec::new();
    for source in ordered {
        if !persona.permitted_source_kinds.contains(&source.source_kind) {
            limitations.push(format!(
                "Source {} was omitted because {:?} evidence is not permitted for this adviser.",
                source.ledger_id, source.source_kind
            ));
            continue;
        }
        let (quote, quote_was_truncated) =
            truncate_utf8(&source.quote, MAX_EVIDENCE_ENVELOPE_QUOTE_BYTES);
        if quote_was_truncated {
            limitations.push(format!(
                "Source {} quote was truncated to the evidence-envelope limit.",
                source.ledger_id
            ));
        }
        let candidate = EvidenceEnvelope {
            ledger_id: source.ledger_id,
            source_kind: source.source_kind,
            source_id: source.source_id,
            collection: source.collection,
            document_id: source.document_id,
            chunk_id: source.chunk_id,
            snapshot_id: source.snapshot_id,
            observed_at: source.observed_at,
            retrieved_at: source.retrieved_at,
            location: source.location,
            quote,
            untrusted_evidence: true,
        };
        if serialize_envelopes(std::slice::from_ref(&candidate)).len() > MAX_EVIDENCE_ENVELOPE_BYTES
        {
            limitations.push(format!(
                "Source {} was omitted because its evidence envelope exceeded the envelope budget.",
                candidate.ledger_id
            ));
            continue;
        }
        let mut with_candidate = envelopes.clone();
        with_candidate.push(candidate.clone());
        let candidate_json = serialize_envelopes(&with_candidate);
        if candidate_json.len() > MAX_PROMPT_EVIDENCE_BYTES {
            limitations.push(format!(
                "Source {} was omitted because the total evidence prompt budget was reached.",
                candidate.ledger_id
            ));
            continue;
        }
        envelopes.push(candidate);
    }
    let evidence_json = serialize_envelopes(&envelopes);
    EvidencePrompt {
        system_prompt: persona.system_prompt(),
        total_bytes: evidence_json.len(),
        evidence_json,
        envelopes,
        limitations,
    }
}

fn serialize_envelopes(envelopes: &[EvidenceEnvelope]) -> String {
    serde_json::to_string(envelopes).unwrap_or_else(|_| "[]".to_string())
}

fn source_order(left: &ValidatedSource, right: &ValidatedSource) -> Ordering {
    source_priority(left.source_kind)
        .cmp(&source_priority(right.source_kind))
        .then_with(|| left.ledger_id.cmp(&right.ledger_id))
        .then_with(|| left.source_id.cmp(&right.source_id))
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
