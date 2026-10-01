#![deny(unsafe_code)]
//! Reciprocal Rank Fusion (RRF) and heuristic decay modulation for SuperRAG.
//!
//! Fuses rankings across:
//! - Dense Vector retrieval (cosine similarity)
//! - Lexical keyword search (BM25 / FTS)
//! - Knowledge graph traversal (2-hop / 3-hop walks)
//!
//! Modulates candidate scores with:
//! - Exponential temporal decay: $e^{-\lambda \cdot \Delta t}$
//! - Source authority weighting ($A_{\text{source}}$)
//! - Active workspace matching boost ($W_{\text{workspace}}$)

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Candidate chunk retrieved from one or more modalities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateChunk {
    /// Authoritative chunk identifier.
    pub chunk_id: Uuid,
    /// Parent document identifier.
    pub document_id: Uuid,
    /// Text snippet content.
    pub content: String,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Source file path or URI.
    pub source_uri: String,
    /// Source classification (`git`, `file`, `adr`, `chat`).
    pub source_type: String,
    /// Line start in source file.
    pub line_start: Option<i32>,
    /// Line end in source file.
    pub line_end: Option<i32>,
    /// Creation timestamp for temporal decay calculation.
    pub created_at: DateTime<Utc>,
    /// Fused relevance score.
    pub score: f32,
    /// Source authority weight.
    pub authority: f32,
}

/// Fuses and ranks candidates using weighted Reciprocal Rank Fusion.
pub struct RrfCombiner {
    k: f32,
    weight_vector: f32,
    weight_lexical: f32,
    weight_graph: f32,
}

impl Default for RrfCombiner {
    fn default() -> Self {
        Self {
            k: 60.0,
            weight_vector: 0.45,
            weight_lexical: 0.35,
            weight_graph: 0.20,
        }
    }
}

impl RrfCombiner {
    /// Creates a custom RRF combiner.
    pub fn new(k: f32, weight_vector: f32, weight_lexical: f32, weight_graph: f32) -> Self {
        Self {
            k: k.max(1.0),
            weight_vector,
            weight_lexical,
            weight_graph,
        }
    }

    /// Calculates temporal decay factor: $e^{-\lambda \cdot \Delta t}$
    pub fn calculate_temporal_decay(created_at: DateTime<Utc>, source_type: &str, now: DateTime<Utc>) -> f32 {
        let elapsed_seconds = (now - created_at).num_seconds().max(0) as f32;
        let elapsed_days = elapsed_seconds / 86400.0;

        // Lambda decay constant based on source type
        let lambda = match source_type.to_ascii_lowercase().as_str() {
            "adr" | "decision" => 0.0,       // ADRs do not decay
            "git" | "commit" => 0.005,       // Commits decay very slowly
            "file" | "code" => 0.015,        // Code decays slowly
            "chat" | "session" => 0.05,      // Chats decay faster
            _ => 0.02,
        };

        (-lambda * elapsed_days).exp().clamp(0.1, 1.0)
    }

    /// Resolves source authority multiplier.
    pub fn resolve_source_authority(source_type: &str) -> f32 {
        match source_type.to_ascii_lowercase().as_str() {
            "git" | "commit" => 1.2,
            "adr" | "decision" => 1.15,
            "file" | "code" => 1.0,
            "chat" | "session" => 0.8,
            _ => 1.0,
        }
    }

    /// Fuses candidate lists and applies modulation.
    pub fn fuse(
        &self,
        vector_candidates: &[CandidateChunk],
        lexical_candidates: &[CandidateChunk],
        graph_candidates: &[CandidateChunk],
        active_workspace: &str,
    ) -> Vec<CandidateChunk> {
        let now = Utc::now();
        let mut chunks_by_id: HashMap<Uuid, CandidateChunk> = HashMap::new();
        let mut rrf_scores: HashMap<Uuid, f32> = HashMap::new();

        // 1. Vector scores
        for (rank, item) in vector_candidates.iter().enumerate() {
            chunks_by_id.entry(item.chunk_id).or_insert_with(|| item.clone());
            let rrf = self.weight_vector / (self.k + (rank + 1) as f32);
            *rrf_scores.entry(item.chunk_id).or_default() += rrf;
        }

        // 2. Lexical scores
        for (rank, item) in lexical_candidates.iter().enumerate() {
            chunks_by_id.entry(item.chunk_id).or_insert_with(|| item.clone());
            let rrf = self.weight_lexical / (self.k + (rank + 1) as f32);
            *rrf_scores.entry(item.chunk_id).or_default() += rrf;
        }

        // 3. Graph scores
        for (rank, item) in graph_candidates.iter().enumerate() {
            chunks_by_id.entry(item.chunk_id).or_insert_with(|| item.clone());
            let rrf = self.weight_graph / (self.k + (rank + 1) as f32);
            *rrf_scores.entry(item.chunk_id).or_default() += rrf;
        }

        // 4. Modulate with Temporal Decay + Authority + Workspace Boost
        let mut results = Vec::with_capacity(chunks_by_id.len());
        for (id, mut chunk) in chunks_by_id {
            let base_rrf = rrf_scores.get(&id).copied().unwrap_or(0.0);
            let decay = Self::calculate_temporal_decay(chunk.created_at, &chunk.source_type, now);
            let authority = Self::resolve_source_authority(&chunk.source_type);
            let workspace_boost = if chunk.workspace_path == active_workspace { 1.5 } else { 1.0 };

            chunk.authority = authority;
            chunk.score = base_rrf * decay * authority * workspace_boost;
            results.push(chunk);
        }

        // Sort descending by modulated score
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_chunk(id: Uuid, source_type: &str, content: &str) -> CandidateChunk {
        CandidateChunk {
            chunk_id: id,
            document_id: Uuid::new_v4(),
            content: content.to_string(),
            workspace_path: "/workspace".to_string(),
            source_uri: "src/test.rs".to_string(),
            source_type: source_type.to_string(),
            line_start: Some(1),
            line_end: Some(10),
            created_at: Utc::now(),
            score: 0.0,
            authority: 1.0,
        }
    }

    #[test]
    fn test_rrf_multi_modal_boost() {
        let combiner = RrfCombiner::default();
        let chunk1 = make_test_chunk(Uuid::new_v4(), "code", "fn auth() {}");
        let chunk2 = make_test_chunk(Uuid::new_v4(), "code", "fn helper() {}");

        // Chunk 1 is present in BOTH vector and lexical results
        let vec_list = vec![chunk1.clone(), chunk2.clone()];
        let lex_list = vec![chunk1.clone()];
        let graph_list = vec![];

        let fused = combiner.fuse(&vec_list, &lex_list, &graph_list, "/workspace");
        assert_eq!(fused[0].chunk_id, chunk1.chunk_id, "Item present in multiple modalities must rank first");
        assert!(fused[0].score > fused[1].score);
    }

    #[test]
    fn test_temporal_decay_calculation() {
        let now = Utc::now();
        let old_time = now - chrono::Duration::days(100);

        // ADR should not decay
        let adr_decay = RrfCombiner::calculate_temporal_decay(old_time, "adr", now);
        assert!((adr_decay - 1.0).abs() < 1e-4);

        // Chat should decay
        let chat_decay = RrfCombiner::calculate_temporal_decay(old_time, "chat", now);
        assert!(chat_decay < 0.5);
    }
}
