#![deny(unsafe_code)]
//! SuperRAG Layer 4 multi-stage retrieval & Layer 5 working context compiler engine.
//!
//! Orchestrates:
//! 1. Pre-retrieval query routing arbiter & semantic cache lookup (<1ms hit)
//! 2. Multi-query decomposition & expansion
//! 3. Parallel multi-modal candidate retrieval (Dense Vectors, Lexical BM25, Knowledge Graph)
//! 4. Reciprocal Rank Fusion (RRF) with exponential temporal decay & source authority weighting
//! 5. Data Re-Trial & Confidence Verification Loop (adaptive query reformulation & graph expansion)
//! 6. Cross-encoder reranking (top 50 -> top 15 precision)
//! 7. Greedy knapsack token budget packing with `<orbit_context>` semantic XML formatting.

use std::sync::Arc;
use buzz_ai::{EmbedProvider, RerankProvider};
use buzz_core::memory::VectorFilter;
use buzz_db::memory::{EmbeddedMemoryStore, MetadataStore};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cache::SemanticQueryCache;
use crate::error::SearchError;
use crate::fusion::{CandidateChunk, RrfCombiner};
use crate::packer::TokenBudgetPacker;
use crate::retrial::{ConfidenceEvaluator, ReTrialDecision};
use crate::router::{QueryIntent, QueryRouter};

/// Request parameters for a SuperRAG retrieval query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuperRagRequest {
    /// Natural language or symbolic query text.
    pub query: String,
    /// Authoritative workspace path for boundary isolation.
    pub workspace_path: String,
    /// Optional token budget override for LLM context packing.
    pub token_budget: Option<usize>,
    /// Maximum candidates to feed to the cross-encoder (default: 50).
    pub limit: Option<usize>,
    /// Optional metadata / vector filter.
    pub filter: Option<VectorFilter>,
    /// Skip checking or writing to the semantic cache.
    pub skip_cache: bool,
}

impl SuperRagRequest {
    /// Creates a new request for a workspace and query.
    pub fn new(workspace_path: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            workspace_path: workspace_path.into(),
            token_budget: None,
            limit: None,
            filter: None,
            skip_cache: false,
        }
    }

    /// Sets the token budget limit.
    pub fn with_budget(mut self, budget: usize) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Sets the candidate limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets whether to bypass the semantic cache.
    pub fn with_skip_cache(mut self, skip: bool) -> Self {
        self.skip_cache = skip;
        self
    }
}

/// Assembled response containing XML context and candidate metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuperRagResponse {
    /// Delimited semantic XML markup ready for LLM prompt injection.
    pub context_xml: String,
    /// Ranked candidate chunks included in the context.
    pub chunks: Vec<CandidateChunk>,
    /// True if served directly from the sub-1ms semantic cache.
    pub cached: bool,
    /// Highest confidence score among retrieved evidence.
    pub confidence: f32,
    /// Classified pre-retrieval query intent.
    pub intent: QueryIntent,
    /// Total tokens allocated within the requested budget.
    pub allocated_tokens: usize,
}

/// Unified SuperRAG retrieval engine for Orbit.
pub struct SuperRagEngine {
    /// Embedded persistent memory store.
    pub store: Arc<EmbeddedMemoryStore>,
    /// Embedding generation provider.
    pub embedder: Arc<dyn EmbedProvider>,
    /// Cross-encoder reranking provider.
    pub reranker: Arc<dyn RerankProvider>,
    /// Semantic query cache.
    pub cache: Arc<SemanticQueryCache>,
    combiner: RrfCombiner,
    evaluator: ConfidenceEvaluator,
    packer: TokenBudgetPacker,
}

impl SuperRagEngine {
    /// Creates a new SuperRAG engine with default parameters.
    pub fn new(
        store: Arc<EmbeddedMemoryStore>,
        embedder: Arc<dyn EmbedProvider>,
        reranker: Arc<dyn RerankProvider>,
    ) -> Self {
        Self {
            store,
            embedder,
            reranker,
            cache: Arc::new(SemanticQueryCache::default()),
            combiner: RrfCombiner::default(),
            evaluator: ConfidenceEvaluator::default(),
            packer: TokenBudgetPacker::default(),
        }
    }

    /// Creates a SuperRAG engine with custom components.
    pub fn with_components(
        store: Arc<EmbeddedMemoryStore>,
        embedder: Arc<dyn EmbedProvider>,
        reranker: Arc<dyn RerankProvider>,
        cache: Arc<SemanticQueryCache>,
        combiner: RrfCombiner,
        evaluator: ConfidenceEvaluator,
        packer: TokenBudgetPacker,
    ) -> Self {
        Self {
            store,
            embedder,
            reranker,
            cache,
            combiner,
            evaluator,
            packer,
        }
    }

    /// Access the underlying semantic cache.
    pub fn cache(&self) -> &Arc<SemanticQueryCache> {
        &self.cache
    }

    /// Executes end-to-end SuperRAG query pipeline.
    pub async fn query(&self, request: &SuperRagRequest) -> Result<SuperRagResponse, SearchError> {
        // 1. Stage 1: Pre-retrieval cache check (<1ms)
        if !request.skip_cache {
            if let Some(cached) = self.cache.get(&request.workspace_path, &request.query) {
                return Ok(SuperRagResponse {
                    context_xml: cached.compiled_xml,
                    chunks: cached.chunks,
                    cached: true,
                    confidence: cached.confidence_score,
                    intent: QueryIntent::Conceptual {
                        semantic_query: request.query.clone(),
                    },
                    allocated_tokens: cached.token_count as usize,
                });
            }
        }

        // 2. Stage 1: Intent classification & query decomposition
        let intent = QueryRouter::classify_intent(&request.query);
        let expansions = QueryRouter::decompose_and_expand(&request.query, &intent);

        let candidate_limit = request.limit.unwrap_or(50);

        // 3. Stage 2: Parallel multi-modal candidate retrieval
        let mut vector_candidates = self
            .retrieve_vector_candidates(&request.query, &request.workspace_path, request.filter.as_ref(), candidate_limit)
            .await?;

        let mut lexical_candidates = self
            .retrieve_lexical_candidates(&expansions, &request.workspace_path, candidate_limit)
            .await?;

        let mut graph_candidates = self
            .retrieve_graph_candidates(&request.query, &request.workspace_path, 2)
            .await?;

        // 4. Stage 3: Reciprocal Rank Fusion & Heuristic Decay
        let mut fused = self.combiner.fuse(
            &vector_candidates,
            &lexical_candidates,
            &graph_candidates,
            &request.workspace_path,
        );

        // 5. Stage 4: Data Re-Trial & Confidence Verification Loop
        let decision = self.evaluator.evaluate(&fused, &request.query);
        if let ReTrialDecision::NeedsReTrial {
            ref reformulated_query,
            expanded_hops,
            ..
        } = decision
        {
            // Execute adaptive re-trial pass with reformulated query and expanded graph walk
            let retrial_lex = self
                .retrieve_lexical_candidates(&[reformulated_query.clone()], &request.workspace_path, candidate_limit)
                .await?;
            let retrial_vec = self
                .retrieve_vector_candidates(reformulated_query, &request.workspace_path, request.filter.as_ref(), candidate_limit)
                .await?;
            let retrial_graph = self
                .retrieve_graph_candidates(reformulated_query, &request.workspace_path, expanded_hops)
                .await?;

            vector_candidates.extend(retrial_vec);
            lexical_candidates.extend(retrial_lex);
            graph_candidates.extend(retrial_graph);

            // Re-fuse pool with newly discovered candidates
            fused = self.combiner.fuse(
                &vector_candidates,
                &lexical_candidates,
                &graph_candidates,
                &request.workspace_path,
            );
        }

        // 6. Stage 5: Cross-Encoder Reranking
        let pool_len = fused.len().min(candidate_limit);
        let mut top_candidates = fused[..pool_len].to_vec();

        if !top_candidates.is_empty() {
            let texts: Vec<&str> = top_candidates.iter().map(|c| c.content.as_str()).collect();
            let rerank_res = self
                .reranker
                .rerank(&request.query, &texts, top_candidates.len())
                .await
                .map_err(SearchError::Ai)?;

            let mut scored_chunks = Vec::with_capacity(top_candidates.len());
            for r in rerank_res {
                if let Some(candidate) = top_candidates.get(r.index) {
                    let mut c = candidate.clone();
                    // Sigmoid / cross-attention score update
                    c.score = r.score;
                    scored_chunks.push(c);
                }
            }
            scored_chunks.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            top_candidates = scored_chunks;
        }

        // 7. Stage 6: Layer 5 Working Context Assembly & Knapsack Packing
        let packed = self.packer.pack(&request.workspace_path, &top_candidates, request.token_budget);
        let top_confidence = top_candidates.first().map(|c| c.score).unwrap_or(0.0);

        // 8. Cache the packed retrieval result
        if !request.skip_cache {
            let chunk_ids: Vec<Uuid> = packed.chunks.iter().map(|c| c.chunk_id).collect();
            self.cache.insert(
                &request.workspace_path,
                &request.query,
                chunk_ids,
                packed.chunks.clone(),
                packed.context_xml.clone(),
                packed.allocated_tokens as i32,
                top_confidence,
            );
        }

        Ok(SuperRagResponse {
            context_xml: packed.context_xml,
            chunks: packed.chunks,
            cached: false,
            confidence: top_confidence,
            intent,
            allocated_tokens: packed.allocated_tokens,
        })
    }

    async fn retrieve_vector_candidates(
        &self,
        query: &str,
        workspace_path: &str,
        filter: Option<&VectorFilter>,
        limit: usize,
    ) -> Result<Vec<CandidateChunk>, SearchError> {
        let embeddings = self
            .embedder
            .embed_batch(&[query])
            .await
            .map_err(SearchError::Ai)?;

        let query_vec = match embeddings.first() {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };

        let effective_filter = match filter {
            Some(f) => f.clone(),
            None => VectorFilter {
                workspace_path: Some(workspace_path.to_string()),
                ..Default::default()
            },
        };

        let hits = self
            .store
            .search_vectors(query_vec, &effective_filter, limit)
            .await
            .map_err(SearchError::MemoryDb)?;

        let mut candidates = Vec::with_capacity(hits.len());
        for hit in hits {
            let chunk_opt = self.store.metadata().get_chunk(&hit.chunk_id).await.map_err(SearchError::MemoryDb)?;
            let doc_opt = self.store.metadata().get_document(&hit.document_id).await.map_err(SearchError::MemoryDb)?;

            let (source_uri, source_type) = match doc_opt {
                Some(d) => (d.source_uri, d.source_type),
                None => ("unknown".to_string(), "code".to_string()),
            };

            let (line_start, line_end, created_at) = match chunk_opt {
                Some(c) => (c.line_start, c.line_end, c.created_at),
                None => (None, None, Utc::now()),
            };

            candidates.push(CandidateChunk {
                chunk_id: hit.chunk_id,
                document_id: hit.document_id,
                content: hit.content,
                workspace_path: workspace_path.to_string(),
                source_uri,
                source_type,
                line_start,
                line_end,
                created_at,
                score: hit.score,
                authority: 1.0,
            });
        }

        Ok(candidates)
    }

    async fn retrieve_lexical_candidates(
        &self,
        queries: &[String],
        workspace_path: &str,
        limit: usize,
    ) -> Result<Vec<CandidateChunk>, SearchError> {
        let mut seen = std::collections::HashSet::new();
        let mut candidates = Vec::new();

        for q in queries {
            let chunks = self
                .store
                .metadata()
                .search_chunks_keyword(workspace_path, q, limit)
                .map_err(SearchError::MemoryDb)?;

            for chunk in chunks {
                if !seen.insert(chunk.id) {
                    continue;
                }

                let doc_opt = self.store.metadata().get_document(&chunk.document_id).await.map_err(SearchError::MemoryDb)?;
                let (source_uri, source_type) = match doc_opt {
                    Some(d) => (d.source_uri, d.source_type),
                    None => ("unknown".to_string(), "code".to_string()),
                };

                candidates.push(CandidateChunk {
                    chunk_id: chunk.id,
                    document_id: chunk.document_id,
                    content: chunk.content,
                    workspace_path: workspace_path.to_string(),
                    source_uri,
                    source_type,
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    created_at: chunk.created_at,
                    score: 1.0,
                    authority: 1.0,
                });
            }
        }

        Ok(candidates)
    }

    async fn retrieve_graph_candidates(
        &self,
        query: &str,
        workspace_path: &str,
        hops: u8,
    ) -> Result<Vec<CandidateChunk>, SearchError> {
        let entities = self.store.find_entities_by_name(workspace_path, query);
        if entities.is_empty() {
            return Ok(Vec::new());
        }

        let seed_ids: Vec<Uuid> = entities.iter().map(|e| e.id).collect();
        let hits = self
            .store
            .graph_neighborhood(&seed_ids, hops)
            .await
            .map_err(SearchError::MemoryDb)?;

        let mut seen = std::collections::HashSet::new();
        let mut candidates = Vec::new();

        for hit in hits {
            if let Some(rel) = hit.relation {
                if let Some(chunk_id) = rel.provenance_chunk_id {
                    if !seen.insert(chunk_id) {
                        continue;
                    }

                    if let Some(chunk) = self.store.metadata().get_chunk(&chunk_id).await.map_err(SearchError::MemoryDb)? {
                        let doc_opt = self.store.metadata().get_document(&chunk.document_id).await.map_err(SearchError::MemoryDb)?;
                        let (source_uri, source_type) = match doc_opt {
                            Some(d) => (d.source_uri, d.source_type),
                            None => ("graph".to_string(), "entity".to_string()),
                        };

                        candidates.push(CandidateChunk {
                            chunk_id: chunk.id,
                            document_id: chunk.document_id,
                            content: chunk.content,
                            workspace_path: workspace_path.to_string(),
                            source_uri,
                            source_type,
                            line_start: chunk.line_start,
                            line_end: chunk.line_end,
                            created_at: chunk.created_at,
                            score: 1.0 - (hit.depth as f32 * 0.2).clamp(0.0, 0.8),
                            authority: 1.1,
                        });
                    }
                }
            }
        }

        Ok(candidates)
    }
}
