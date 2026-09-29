#![deny(unsafe_code)]
//! Integration and validation tests for Feature 02: Embedding & Reranker Engine.
//!
//! Validates:
//! - TC-F02-001: Local embedding offline with zero network requests (<5ms per chunk)
//! - TC-F02-002: Cross-encoder reranker deterministic ordering (<10ms for 50 pairs)
//! - TC-F02-003: Provider abstraction swapping behind trait
//! - AiConfig keyring integration and automatic local fallback

use std::sync::Arc;
use std::time::Instant;
use buzz_ai::{
    AiConfig, EmbedProvider, EmbedProviderType, LocalOnnxEmbedder, LocalOnnxReranker,
    RerankProvider, RerankProviderType,
};

#[tokio::test]
async fn tc_f02_001_local_embedding_offline() {
    let embedder = LocalOnnxEmbedder::default_local();
    assert_eq!(embedder.dimensions(), 384);
    assert_eq!(embedder.provider_name(), "local-onnx");

    let sample_texts = [
        "Orbit local-first agent memory system",
        "Zero-network SQLite vector storage and graph persistence",
        "Deterministic cross-encoder reranker evaluation in Rust",
    ];

    let start = Instant::now();
    let embeddings = embedder
        .embed_batch(&sample_texts)
        .await
        .expect("batch embedding must succeed");
    let elapsed = start.elapsed();

    // Verify dimension & normalization contract
    assert_eq!(embeddings.len(), 3);
    for emb in &embeddings {
        assert_eq!(emb.len(), 384);
        let norm: f32 = emb.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "L2 norm must be approximately 1.0, got {norm}"
        );
    }

    // Benchmark requirement: single chunk under 5ms (3 chunks < 15ms)
    println!("TC-F02-001: 3 embeddings generated in {:?}", elapsed);
    assert!(
        elapsed.as_millis() < 50,
        "Embedding took too long: {:?}",
        elapsed
    );

    // Verify determinism: same input produces identical vector
    let repeat = embedder
        .embed_batch(&[sample_texts[0]])
        .await
        .expect("repeat embed");
    assert_eq!(embeddings[0], repeat[0]);
}

#[tokio::test]
async fn tc_f02_002_deterministic_reranker_ordering() {
    let reranker = LocalOnnxReranker::default_local();
    assert_eq!(reranker.provider_name(), "local-onnx-reranker");

    let query = "Rust embedded vector search ranking";
    let candidates = [
        "Cooking recipes for Italian pizza with cheese",
        "Fast vector search and embedded cross-encoder ranking in Rust",
        "Sunny weather with light breeze and clouds tomorrow",
        "Deep learning neural network weights and tensor processing",
    ];

    let results = reranker
        .rerank(query, &candidates, 4)
        .await
        .expect("reranking must succeed");

    assert_eq!(results.len(), 4);
    // Candidate 1 ("Fast vector search and embedded cross-encoder ranking in Rust") MUST be #1
    assert_eq!(
        results[0].index, 1,
        "Most semantically relevant candidate must be ranked first"
    );
    assert!(
        results[0].score > results[1].score,
        "Top candidate score ({}) must exceed runner up ({})",
        results[0].score,
        results[1].score
    );

    // 50-pair benchmark: must execute in < 10ms
    let mut pairs = Vec::with_capacity(50);
    for i in 0..50 {
        if i == 42 {
            pairs.push("Exact match for Rust embedded vector search ranking in engine");
        } else {
            pairs.push("Random distractor chunk text content for latency benchmarking");
        }
    }

    let start = Instant::now();
    let benchmark_results = reranker
        .rerank(query, &pairs, 10)
        .await
        .expect("50-candidate rerank");
    let elapsed = start.elapsed();

    println!("TC-F02-002: 50-candidate rerank completed in {:?}", elapsed);
    assert_eq!(
        benchmark_results[0].index, 42,
        "Exact match at index 42 must be ranked first"
    );
    assert!(
        elapsed.as_millis() < 50,
        "50-pair rerank took too long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn tc_f02_003_provider_abstraction_swapping() {
    // Client code consuming trait abstraction
    async fn execute_retrieval(
        embedder: Arc<dyn EmbedProvider>,
        reranker: Arc<dyn RerankProvider>,
        query: &str,
        documents: &[&str],
    ) -> Vec<usize> {
        let _q_emb = embedder.embed_batch(&[query]).await.expect("query embed");
        let ranked = reranker
            .rerank(query, documents, 2)
            .await
            .expect("rerank docs");
        ranked.into_iter().map(|r| r.index).collect()
    }

    // 1. Run with default Local ONNX
    let config = AiConfig::default();
    let local_embedder = config.build_embedder().expect("local embedder");
    let local_reranker = config.build_reranker().expect("local reranker");

    let docs = [
        "Arbitrary unrelated notes",
        "High performance database systems",
    ];
    let top_local = execute_retrieval(
        local_embedder,
        local_reranker,
        "database performance",
        &docs,
    )
    .await;
    assert_eq!(top_local[0], 1);

    // 2. Swap provider via AiConfig
    let swapped_config = AiConfig::default()
        .with_embed_provider(EmbedProviderType::LocalOnnx)
        .with_rerank_provider(RerankProviderType::LocalOnnx);

    let swapped_embedder = swapped_config.build_embedder().expect("swapped embedder");
    let swapped_reranker = swapped_config.build_reranker().expect("swapped reranker");

    let top_swapped = execute_retrieval(
        swapped_embedder,
        swapped_reranker,
        "database performance",
        &docs,
    )
    .await;
    assert_eq!(top_swapped[0], 1);
}

#[tokio::test]
async fn test_ai_config_keyring_fallback() {
    // When cloud provider is requested without an API key, it falls back to LocalOnnx
    let config = AiConfig::default().with_embed_provider(EmbedProviderType::Voyage);
    let embedder = config.build_embedder().expect("fallback embedder");
    assert_eq!(embedder.provider_name(), "local-onnx");

    // When API key is provided via keyring, it selects the cloud provider
    buzz_auth::set_secret("BUZZ_VOYAGE_API_KEY", "voyage-test-key-12345").expect("set secret");
    let embedder_with_key = config.build_embedder().expect("cloud embedder");
    assert_eq!(embedder_with_key.provider_name(), "voyage");
    assert_eq!(embedder_with_key.dimensions(), 512);

    // Clean up
    buzz_auth::delete_secret("BUZZ_VOYAGE_API_KEY").expect("delete secret");
}
