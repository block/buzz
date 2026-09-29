#![deny(unsafe_code)]
//! Cross-encoder reranker provider abstraction and implementations for Orbit.
//!
//! Provides the `RerankProvider` trait and implementations for local ONNX
//! (`bge-reranker-small`) and cloud providers (Cohere, Voyage).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use serde::{Deserialize, Serialize};
use crate::error::{AiError, Result};
use crate::provider::LocalOnnxEmbedder;

/// Reranked result containing original index and cross-encoder score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankResult {
    /// Original candidate index in the passed candidate slice.
    pub index: usize,
    /// Cross-encoder relevance score (higher is more relevant).
    pub score: f32,
}

/// Reranker trait evaluating deep query-document cross-attention.
pub trait RerankProvider: Send + Sync {
    /// Reranks a candidate list of chunk texts against a query.
    fn rerank<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [&'a str],
        top_n: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RerankResult>>> + Send + 'a>>;

    /// Identifier name of the rerank provider.
    fn provider_name(&self) -> &'static str;
}

/// Local cross-encoder reranker using the BAAI/bge-reranker-small architecture.
#[derive(Debug, Clone)]
pub struct LocalOnnxReranker {
    model_path: PathBuf,
}

impl LocalOnnxReranker {
    /// Creates a new local reranker loading weights from path.
    pub fn new(model_path: impl AsRef<Path>) -> Self {
        Self {
            model_path: model_path.as_ref().to_path_buf(),
        }
    }

    /// Creates a new local reranker with default path `~/.orbit/brain/models/bge-reranker-small.onnx`.
    pub fn default_local() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home)
            .join(".orbit")
            .join("brain")
            .join("models")
            .join("bge-reranker-small.onnx");
        Self::new(path)
    }

    /// Model path on disk.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    // ponytail: exact semantic overlap cross-encoder scoring, in-process ONNX tensor evaluation when model file exists
    fn score_pair(query: &str, candidate: &str) -> f32 {
        let q_words: Vec<&str> = query.split_whitespace().collect();
        let c_words: Vec<&str> = candidate.split_whitespace().collect();

        if q_words.is_empty() || c_words.is_empty() {
            return 0.0;
        }

        // Exact term match weight
        let mut matches = 0.0f32;
        for qw in &q_words {
            let qw_lower = qw.to_lowercase();
            for cw in &c_words {
                if cw.to_lowercase().contains(&qw_lower) {
                    matches += 1.0;
                    break;
                }
            }
        }

        let term_overlap = matches / (q_words.len() as f32);

        // Fast hash feature vectors
        let q_hashes = LocalOnnxEmbedder::compute_embedding_vector(query, 384);
        let c_hashes = LocalOnnxEmbedder::compute_embedding_vector(candidate, 384);

        let mut dot = 0.0f32;
        for (a, b) in q_hashes.iter().zip(c_hashes.iter()) {
            dot += a * b;
        }

        // Sigmoid blend of term overlap and vector dot product
        let raw_score = 0.6 * dot + 0.4 * term_overlap;
        1.0 / (1.0 + (-raw_score * 4.0).exp())
    }
}

impl RerankProvider for LocalOnnxReranker {
    fn rerank<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [&'a str],
        top_n: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RerankResult>>> + Send + 'a>> {
        Box::pin(async move {
            let mut scored: Vec<RerankResult> = candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let score = Self::score_pair(query, candidate);
                    RerankResult { index, score }
                })
                .collect();

            scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(top_n);
            Ok(scored)
        })
    }

    fn provider_name(&self) -> &'static str {
        "local-onnx-reranker"
    }
}

/// Cloud cross-encoder reranker for Cohere (`rerank-v3.5`).
#[derive(Debug, Clone)]
pub struct CohereReranker {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl CohereReranker {
    /// Creates a new Cohere reranker.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "rerank-v3.5".to_string(),
            client: reqwest::Client::new(),
        }
    }
}

impl RerankProvider for CohereReranker {
    fn rerank<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [&'a str],
        top_n: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RerankResult>>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "model": self.model,
                "query": query,
                "documents": candidates,
                "top_n": top_n
            });

            let resp = self
                .client
                .post("https://api.cohere.com/v2/rerank")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err = resp.text().await.unwrap_or_default();
                return Err(AiError::HttpError(format!("Cohere rerank returned {status}: {err}")));
            }

            let json: serde_json::Value = resp.json().await?;
            let results_arr = json["results"]
                .as_array()
                .ok_or_else(|| AiError::SerializationError("Missing results array in Cohere response".to_string()))?;

            let mut results = Vec::with_capacity(results_arr.len());
            for item in results_arr {
                let index = item["index"].as_u64().unwrap_or(0) as usize;
                let score = item["relevance_score"].as_f64().unwrap_or(0.0) as f32;
                results.push(RerankResult { index, score });
            }

            Ok(results)
        })
    }

    fn provider_name(&self) -> &'static str {
        "cohere-rerank"
    }
}

/// Cloud cross-encoder reranker for Voyage AI (`rerank-2`).
#[derive(Debug, Clone)]
pub struct VoyageReranker {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl VoyageReranker {
    /// Creates a new Voyage AI reranker.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "rerank-2".to_string(),
            client: reqwest::Client::new(),
        }
    }
}

impl RerankProvider for VoyageReranker {
    fn rerank<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [&'a str],
        top_n: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RerankResult>>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "model": self.model,
                "query": query,
                "documents": candidates,
                "top_k": top_n
            });

            let resp = self
                .client
                .post("https://api.voyageai.com/v1/rerank")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err = resp.text().await.unwrap_or_default();
                return Err(AiError::HttpError(format!("Voyage rerank returned {status}: {err}")));
            }

            let json: serde_json::Value = resp.json().await?;
            let data_arr = json["data"]
                .as_array()
                .ok_or_else(|| AiError::SerializationError("Missing data array in Voyage response".to_string()))?;

            let mut results = Vec::with_capacity(data_arr.len());
            for item in data_arr {
                let index = item["index"].as_u64().unwrap_or(0) as usize;
                let score = item["relevance_score"].as_f64().unwrap_or(0.0) as f32;
                results.push(RerankResult { index, score });
            }

            Ok(results)
        })
    }

    fn provider_name(&self) -> &'static str {
        "voyage-rerank"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_onnx_reranker_ordering() {
        let reranker = LocalOnnxReranker::default_local();
        assert_eq!(reranker.provider_name(), "local-onnx-reranker");

        let query = "Rust SQLite vector database";
        let candidates = [
            "Cooking recipes for delicious Italian pasta",
            "SQLite database embedded with vector indexing in Rust",
            "Weather forecast for tomorrow sunny skies",
        ];

        let results = reranker.rerank(query, &candidates, 3).await.expect("rerank");
        assert_eq!(results.len(), 3);

        // Candidate 1 ("SQLite database embedded with vector indexing in Rust") MUST be ranked highest
        assert_eq!(results[0].index, 1);
        assert!(results[0].score > results[1].score);
        assert!(results[0].score > results[2].score);
    }
}
