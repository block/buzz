use super::*;

pub(super) fn extract_rag_records(
    snapshot: &VerifiedRagSnapshot,
    query: &str,
    value: &Value,
    observed_at: &str,
    collections: &[String],
) -> Result<Vec<CandidateSource>, SourceReadError> {
    match snapshot.assurance() {
        RagSnapshotAssurance::SignedSnapshot => {
            extract_verified_rag_evidence(snapshot, query, value)
                .map(|records| {
                    records
                        .into_iter()
                        .map(|record| CandidateSource {
                            source_id: record.source_id,
                            source_kind: SourceKind::Rag,
                            collection: record.collection,
                            document_id: record.document_id,
                            chunk_id: record.chunk_id,
                            timestamp: record.retrieved_at.clone(),
                            location: record.location,
                            retrieved_at: record.retrieved_at,
                            observed_at: observed_at.to_string(),
                            quote: record.quote,
                        })
                        .collect()
                })
                .map_err(|_| SourceReadError::new("rag_evidence_invalid"))
        }
        RagSnapshotAssurance::TrustedLanObserved => {
            extract_trusted_lan_rag_evidence(value, query, observed_at, collections)
                .map_err(|_| SourceReadError::new("rag_evidence_invalid"))
        }
    }
}
