//! Knowledge base helpers (P3 — pgvector read-model).

pub mod embed;

use serde::{Deserialize, Serialize};

/// In-memory document record before projector persistence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDocument {
    /// Knowledge base identifier.
    pub knowledge_base_id: String,
    /// Document identifier.
    pub document_id: String,
    /// Original filename.
    pub filename: String,
    /// MIME type of the document body.
    pub mime_type: String,
    /// Plain-text document content.
    pub content: String,
}

/// Semantic search hit (MVP — full vector search wired in P3 projector).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticSearchHit {
    /// Matching document identifier.
    pub document_id: String,
    /// Chunk index within the document.
    pub chunk_index: i32,
    /// Matching chunk text.
    pub content: String,
    /// Relevance score (higher is better).
    pub score: f32,
}

/// Naive keyword fallback when pgvector is unavailable.
pub fn keyword_search(documents: &[KnowledgeDocument], query: &str) -> Vec<SemanticSearchHit> {
    let needle = query.to_ascii_lowercase();
    let mut hits = Vec::new();
    for doc in documents {
        if doc.content.to_ascii_lowercase().contains(&needle) {
            hits.push(SemanticSearchHit {
                document_id: doc.document_id.clone(),
                chunk_index: 0,
                content: doc.content.chars().take(200).collect(),
                score: 1.0,
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_search_finds_match() {
        let docs = vec![KnowledgeDocument {
            knowledge_base_id: "kb1".into(),
            document_id: "d1".into(),
            filename: "readme.txt".into(),
            mime_type: "text/plain".into(),
            content: "Buzz Hive knowledge base".into(),
        }];
        let hits = keyword_search(&docs, "hive");
        assert_eq!(hits.len(), 1);
    }
}
