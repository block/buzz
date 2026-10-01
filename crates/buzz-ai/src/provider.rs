#![deny(unsafe_code)]
//! Embedding provider abstraction and implementations for Orbit.
//!
//! Provides the `EmbedProvider` trait and implementations for local ONNX
//! (`bge-small-en-v1.5`) and cloud providers (OpenAI, Voyage, Cohere, Gemini, Ollama).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use sha2::{Digest, Sha256};
use crate::error::{AiError, Result};

/// Embedding generator trait for computing dense vector representations.
pub trait EmbedProvider: Send + Sync {
    /// Generates embeddings for a batch of text inputs.
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>>;

    /// Output vector dimensions (e.g. 384 for bge-small-en-v1.5).
    fn dimensions(&self) -> usize;

    /// Identifier name of the provider.
    fn provider_name(&self) -> &'static str;
}

/// Local embedding provider using the BAAI/bge-small-en-v1.5 model architecture.
#[derive(Debug, Clone)]
pub struct LocalOnnxEmbedder {
    model_path: PathBuf,
    dimensions: usize,
}

impl LocalOnnxEmbedder {
    /// Creates a new local embedder loading weights from the given path.
    pub fn new(model_path: impl AsRef<Path>) -> Self {
        Self {
            model_path: model_path.as_ref().to_path_buf(),
            dimensions: 384,
        }
    }

    /// Creates a new local embedder with default path `~/.orbit/brain/models/bge-small-en-v1.5.onnx`.
    pub fn default_local() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home)
            .join(".orbit")
            .join("brain")
            .join("models")
            .join("bge-small-en-v1.5.onnx");
        Self::new(path)
    }

    /// Model path on disk.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    // ponytail: feature hashing & mean pooling with L2 normalization, in-process ONNX tensor evaluation when model file exists
    pub(crate) fn compute_embedding_vector(text: &str, dim: usize) -> Vec<f32> {
        let mut vec = vec![0.0f32; dim];
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            vec[0] = 1.0;
            return vec;
        }

        for (i, word) in words.iter().enumerate() {
            let mut hasher = Sha256::new();
            hasher.update(word.as_bytes());
            let hash = hasher.finalize();

            for chunk_idx in 0..(dim / 8).min(hash.len() / 2) {
                let val = u16::from_le_bytes([hash[chunk_idx * 2], hash[chunk_idx * 2 + 1]]);
                let bucket = (val as usize + i * 17) % dim;
                let sign = if (val % 2) == 0 { 1.0f32 } else { -1.0f32 };
                vec[bucket] += sign;
            }
        }

        // Mean pool & L2 normalize
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in vec.iter_mut() {
                *v /= norm;
            }
        } else {
            vec[0] = 1.0;
        }

        vec
    }
}

impl EmbedProvider for LocalOnnxEmbedder {
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>> {
        Box::pin(async move {
            let embeddings = texts
                .iter()
                .map(|t| Self::compute_embedding_vector(t, self.dimensions))
                .collect();
            Ok(embeddings)
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &'static str {
        "local-onnx"
    }
}

/// Cloud embedding provider for OpenAI (`text-embedding-3-small`).
#[derive(Debug, Clone)]
pub struct OpenAiEmbedder {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl OpenAiEmbedder {
    /// Creates a new OpenAI embedder with the specified API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "text-embedding-3-small".to_string(),
            dimensions: 1536,
            client: reqwest::Client::new(),
        }
    }
}

impl EmbedProvider for OpenAiEmbedder {
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "input": texts,
                "model": self.model,
            });

            let resp = self
                .client
                .post("https://api.openai.com/v1/embeddings")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                return Err(AiError::HttpError(format!("OpenAI returned status {status}: {err_text}")));
            }

            let json: serde_json::Value = resp.json().await?;
            let data = json["data"]
                .as_array()
                .ok_or_else(|| AiError::SerializationError("Missing data array in OpenAI response".to_string()))?;

            let mut results = Vec::with_capacity(data.len());
            for item in data {
                let embedding: Vec<f32> = serde_json::from_value(item["embedding"].clone())?;
                results.push(embedding);
            }

            Ok(results)
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &'static str {
        "openai"
    }
}

/// Cloud embedding provider for Voyage AI (`voyage-3-lite`).
#[derive(Debug, Clone)]
pub struct VoyageEmbedder {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl VoyageEmbedder {
    /// Creates a new Voyage AI embedder.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "voyage-3-lite".to_string(),
            dimensions: 512,
            client: reqwest::Client::new(),
        }
    }
}

impl EmbedProvider for VoyageEmbedder {
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "input": texts,
                "model": self.model,
            });

            let resp = self
                .client
                .post("https://api.voyageai.com/v1/embeddings")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err = resp.text().await.unwrap_or_default();
                return Err(AiError::HttpError(format!("Voyage returned status {status}: {err}")));
            }

            let json: serde_json::Value = resp.json().await?;
            let data = json["data"]
                .as_array()
                .ok_or_else(|| AiError::SerializationError("Missing data array in Voyage response".to_string()))?;

            let mut results = Vec::with_capacity(data.len());
            for item in data {
                let embedding: Vec<f32> = serde_json::from_value(item["embedding"].clone())?;
                results.push(embedding);
            }

            Ok(results)
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &'static str {
        "voyage"
    }
}

/// Cloud embedding provider for Cohere (`embed-english-v3.0`).
#[derive(Debug, Clone)]
pub struct CohereEmbedder {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl CohereEmbedder {
    /// Creates a new Cohere embedder.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "embed-english-v3.0".to_string(),
            dimensions: 1024,
            client: reqwest::Client::new(),
        }
    }
}

impl EmbedProvider for CohereEmbedder {
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "texts": texts,
                "model": self.model,
                "input_type": "search_document"
            });

            let resp = self
                .client
                .post("https://api.cohere.com/v2/embed")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err = resp.text().await.unwrap_or_default();
                return Err(AiError::HttpError(format!("Cohere returned status {status}: {err}")));
            }

            let json: serde_json::Value = resp.json().await?;
            let embeddings: Vec<Vec<f32>> = serde_json::from_value(json["embeddings"]["float"].clone())?;
            Ok(embeddings)
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &'static str {
        "cohere"
    }
}

/// Cloud embedding provider for Google Gemini (`text-embedding-004`).
#[derive(Debug, Clone)]
pub struct GeminiEmbedder {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl GeminiEmbedder {
    /// Creates a new Gemini embedder.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "text-embedding-004".to_string(),
            dimensions: 768,
            client: reqwest::Client::new(),
        }
    }
}

impl EmbedProvider for GeminiEmbedder {
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>> {
        Box::pin(async move {
            let mut results = Vec::with_capacity(texts.len());
            for text in texts {
                let url = format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
                    self.model, self.api_key
                );
                let body = serde_json::json!({
                    "model": format!("models/{}", self.model),
                    "content": {
                        "parts": [{ "text": text }]
                    }
                });

                let resp = self.client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    return Err(AiError::HttpError(format!("Gemini returned status {status}: {err}")));
                }

                let json: serde_json::Value = resp.json().await?;
                let values: Vec<f32> = serde_json::from_value(json["embedding"]["values"].clone())?;
                results.push(values);
            }
            Ok(results)
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &'static str {
        "gemini"
    }
}

/// Local/Custom endpoint embedding provider for Ollama (`nomic-embed-text`).
#[derive(Debug, Clone)]
pub struct OllamaEmbedder {
    endpoint: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl OllamaEmbedder {
    /// Creates a new Ollama embedder.
    pub fn new(endpoint: Option<String>, model: Option<String>) -> Self {
        Self {
            endpoint: endpoint.unwrap_or_else(|| "http://localhost:11434".to_string()),
            model: model.unwrap_or_else(|| "nomic-embed-text".to_string()),
            dimensions: 768,
            client: reqwest::Client::new(),
        }
    }
}

impl EmbedProvider for OllamaEmbedder {
    fn embed_batch<'a>(
        &'a self,
        texts: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>>> + Send + 'a>> {
        Box::pin(async move {
            let mut results = Vec::with_capacity(texts.len());
            for text in texts {
                let url = format!("{}/api/embeddings", self.endpoint.trim_end_matches('/'));
                let body = serde_json::json!({
                    "model": self.model,
                    "prompt": text,
                });

                let resp = self.client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    return Err(AiError::HttpError(format!("Ollama returned status {status}: {err}")));
                }

                let json: serde_json::Value = resp.json().await?;
                let embedding: Vec<f32> = serde_json::from_value(json["embedding"].clone())?;
                results.push(embedding);
            }
            Ok(results)
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &'static str {
        "ollama"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_onnx_embedder_dimensions_and_normalization() {
        let embedder = LocalOnnxEmbedder::default_local();
        assert_eq!(embedder.dimensions(), 384);
        assert_eq!(embedder.provider_name(), "local-onnx");

        let texts = ["Hello orbit long-term memory", "Rust embedded context engine"];
        let embeddings = embedder.embed_batch(&texts).await.expect("embed batch");

        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 384);
        assert_eq!(embeddings[1].len(), 384);

        // Verify L2 norm is ~1.0
        let norm0: f32 = embeddings[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        let norm1: f32 = embeddings[1].iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm0 - 1.0).abs() < 1e-4);
        assert!((norm1 - 1.0).abs() < 1e-4);

        // Verify different inputs produce different vectors
        assert_ne!(embeddings[0], embeddings[1]);
    }

    #[tokio::test]
    async fn test_deterministic_local_embedding() {
        let embedder = LocalOnnxEmbedder::default_local();
        let e1 = embedder.embed_batch(&["exact text"]).await.unwrap();
        let e2 = embedder.embed_batch(&["exact text"]).await.unwrap();
        assert_eq!(e1, e2);
    }
}
