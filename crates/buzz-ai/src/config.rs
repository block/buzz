#![deny(unsafe_code)]
//! Configuration and dynamic provider selection for Orbit AI engines.
//!
//! Evaluates environment variables, OS hardware keyring secrets via `buzz-auth`,
//! and provides fallback to local ONNX runtimes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::error::{AiError, Result};
use crate::provider::{
    CohereEmbedder, EmbedProvider, GeminiEmbedder, LocalOnnxEmbedder, OllamaEmbedder, OpenAiEmbedder,
    VoyageEmbedder,
};
use crate::rerank::{CohereReranker, LocalOnnxReranker, RerankProvider, VoyageReranker};

/// Supported embedding provider backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbedProviderType {
    /// Local CPU/GPU inference via ONNX Runtime (BAAI/bge-small-en-v1.5).
    LocalOnnx,
    /// OpenAI text-embedding-3-small / text-embedding-3-large.
    OpenAi,
    /// Voyage AI voyage-3-lite / voyage-code-2.
    Voyage,
    /// Cohere embed-english-v3.0.
    Cohere,
    /// Google Gemini text-embedding-004.
    Gemini,
    /// Local/Remote Ollama instance.
    Ollama,
}

impl std::str::FromStr for EmbedProviderType {
    type Err = AiError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "local" | "local-onnx" | "onnx" | "bge" => Ok(Self::LocalOnnx),
            "openai" => Ok(Self::OpenAi),
            "voyage" | "voyageai" => Ok(Self::Voyage),
            "cohere" => Ok(Self::Cohere),
            "gemini" | "google" => Ok(Self::Gemini),
            "ollama" => Ok(Self::Ollama),
            other => Err(AiError::ConfigError(format!(
                "Unknown embedding provider: '{other}'"
            ))),
        }
    }
}

/// Supported cross-encoder reranker provider backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RerankProviderType {
    /// Local CPU cross-encoder via ONNX Runtime (BAAI/bge-reranker-small).
    LocalOnnx,
    /// Cohere rerank-v3.5.
    Cohere,
    /// Voyage AI rerank-2.
    Voyage,
}

impl std::str::FromStr for RerankProviderType {
    type Err = AiError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "local" | "local-onnx" | "onnx" | "bge" => Ok(Self::LocalOnnx),
            "cohere" => Ok(Self::Cohere),
            "voyage" | "voyageai" => Ok(Self::Voyage),
            other => Err(AiError::ConfigError(format!(
                "Unknown reranker provider: '{other}'"
            ))),
        }
    }
}

/// Configuration descriptor for the Orbit AI engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    /// Active embedding provider.
    pub embed_provider: EmbedProviderType,
    /// Optional model identifier override for embedding.
    pub embed_model: Option<String>,
    /// Active reranker provider.
    pub rerank_provider: RerankProviderType,
    /// Optional model identifier override for reranking.
    pub rerank_model: Option<String>,
    /// Base storage directory for local models (default `~/.orbit/brain/`).
    pub data_dir: PathBuf,
}

impl Default for AiConfig {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let data_dir = PathBuf::from(home).join(".orbit").join("brain");

        Self {
            embed_provider: EmbedProviderType::LocalOnnx,
            embed_model: None,
            rerank_provider: RerankProviderType::LocalOnnx,
            rerank_model: None,
            data_dir,
        }
    }
}

impl AiConfig {
    /// Loads configuration initialized from environment variables with safe defaults.
    ///
    /// Evaluates:
    /// - `BUZZ_EMBED_PROVIDER`
    /// - `BUZZ_EMBED_MODEL`
    /// - `BUZZ_RERANK_PROVIDER`
    /// - `BUZZ_RERANK_MODEL`
    /// - `BUZZ_DATA_DIR`
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(dir) = std::env::var("BUZZ_DATA_DIR") {
            if !dir.trim().is_empty() {
                config.data_dir = PathBuf::from(dir);
            }
        }

        if let Ok(provider_str) = std::env::var("BUZZ_EMBED_PROVIDER") {
            if let Ok(provider) = provider_str.parse() {
                config.embed_provider = provider;
            }
        }

        if let Ok(model) = std::env::var("BUZZ_EMBED_MODEL") {
            if !model.trim().is_empty() {
                config.embed_model = Some(model);
            }
        }

        if let Ok(provider_str) = std::env::var("BUZZ_RERANK_PROVIDER") {
            if let Ok(provider) = provider_str.parse() {
                config.rerank_provider = provider;
            }
        }

        if let Ok(model) = std::env::var("BUZZ_RERANK_MODEL") {
            if !model.trim().is_empty() {
                config.rerank_model = Some(model);
            }
        }

        config
    }

    /// Builder method to specify embedding provider.
    pub fn with_embed_provider(mut self, provider: EmbedProviderType) -> Self {
        self.embed_provider = provider;
        self
    }

    /// Builder method to specify reranker provider.
    pub fn with_rerank_provider(mut self, provider: RerankProviderType) -> Self {
        self.rerank_provider = provider;
        self
    }

    /// Builder method to specify base data directory.
    pub fn with_data_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.data_dir = dir.as_ref().to_path_buf();
        self
    }

    /// Constructs an instance of the configured `EmbedProvider`.
    ///
    /// Resolves required API keys from the OS hardware keyring / environment via `buzz-auth`.
    /// If cloud credentials are missing, falls back cleanly to `LocalOnnxEmbedder`.
    pub fn build_embedder(&self) -> Result<Arc<dyn EmbedProvider>> {
        match self.embed_provider {
            EmbedProviderType::LocalOnnx => {
                let model_path = self.data_dir.join("models").join("bge-small-en-v1.5.onnx");
                Ok(Arc::new(LocalOnnxEmbedder::new(model_path)))
            }
            EmbedProviderType::OpenAi => {
                if let Some(key) = buzz_auth::resolve_secret("BUZZ_OPENAI_API_KEY") {
                    Ok(Arc::new(OpenAiEmbedder::new(key)))
                } else {
                    tracing::warn!("BUZZ_OPENAI_API_KEY not found in keyring/env; falling back to LocalOnnx");
                    let model_path = self.data_dir.join("models").join("bge-small-en-v1.5.onnx");
                    Ok(Arc::new(LocalOnnxEmbedder::new(model_path)))
                }
            }
            EmbedProviderType::Voyage => {
                if let Some(key) = buzz_auth::resolve_secret("BUZZ_VOYAGE_API_KEY") {
                    Ok(Arc::new(VoyageEmbedder::new(key)))
                } else {
                    tracing::warn!("BUZZ_VOYAGE_API_KEY not found in keyring/env; falling back to LocalOnnx");
                    let model_path = self.data_dir.join("models").join("bge-small-en-v1.5.onnx");
                    Ok(Arc::new(LocalOnnxEmbedder::new(model_path)))
                }
            }
            EmbedProviderType::Cohere => {
                if let Some(key) = buzz_auth::resolve_secret("BUZZ_COHERE_API_KEY") {
                    Ok(Arc::new(CohereEmbedder::new(key)))
                } else {
                    tracing::warn!("BUZZ_COHERE_API_KEY not found in keyring/env; falling back to LocalOnnx");
                    let model_path = self.data_dir.join("models").join("bge-small-en-v1.5.onnx");
                    Ok(Arc::new(LocalOnnxEmbedder::new(model_path)))
                }
            }
            EmbedProviderType::Gemini => {
                if let Some(key) = buzz_auth::resolve_secret("BUZZ_GEMINI_API_KEY") {
                    Ok(Arc::new(GeminiEmbedder::new(key)))
                } else {
                    tracing::warn!("BUZZ_GEMINI_API_KEY not found in keyring/env; falling back to LocalOnnx");
                    let model_path = self.data_dir.join("models").join("bge-small-en-v1.5.onnx");
                    Ok(Arc::new(LocalOnnxEmbedder::new(model_path)))
                }
            }
            EmbedProviderType::Ollama => {
                let endpoint = std::env::var("BUZZ_OLLAMA_ENDPOINT")
                    .or_else(|_| std::env::var("OLLAMA_ENDPOINT"))
                    .ok();
                Ok(Arc::new(OllamaEmbedder::new(endpoint, self.embed_model.clone())))
            }
        }
    }

    /// Constructs an instance of the configured `RerankProvider`.
    ///
    /// Resolves required API keys from the OS hardware keyring / environment via `buzz-auth`.
    /// If cloud credentials are missing, falls back cleanly to `LocalOnnxReranker`.
    pub fn build_reranker(&self) -> Result<Arc<dyn RerankProvider>> {
        match self.rerank_provider {
            RerankProviderType::LocalOnnx => {
                let model_path = self.data_dir.join("models").join("bge-reranker-small.onnx");
                Ok(Arc::new(LocalOnnxReranker::new(model_path)))
            }
            RerankProviderType::Cohere => {
                if let Some(key) = buzz_auth::resolve_secret("BUZZ_COHERE_API_KEY") {
                    Ok(Arc::new(CohereReranker::new(key)))
                } else {
                    tracing::warn!("BUZZ_COHERE_API_KEY not found in keyring/env; falling back to LocalOnnx");
                    let model_path = self.data_dir.join("models").join("bge-reranker-small.onnx");
                    Ok(Arc::new(LocalOnnxReranker::new(model_path)))
                }
            }
            RerankProviderType::Voyage => {
                if let Some(key) = buzz_auth::resolve_secret("BUZZ_VOYAGE_API_KEY") {
                    Ok(Arc::new(VoyageReranker::new(key)))
                } else {
                    tracing::warn!("BUZZ_VOYAGE_API_KEY not found in keyring/env; falling back to LocalOnnx");
                    let model_path = self.data_dir.join("models").join("bge-reranker-small.onnx");
                    Ok(Arc::new(LocalOnnxReranker::new(model_path)))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults_and_env_overrides() {
        let config = AiConfig::default();
        assert_eq!(config.embed_provider, EmbedProviderType::LocalOnnx);
        assert_eq!(config.rerank_provider, RerankProviderType::LocalOnnx);

        std::env::set_var("BUZZ_EMBED_PROVIDER", "voyage");
        std::env::set_var("BUZZ_RERANK_PROVIDER", "cohere");
        let env_config = AiConfig::from_env();
        assert_eq!(env_config.embed_provider, EmbedProviderType::Voyage);
        assert_eq!(env_config.rerank_provider, RerankProviderType::Cohere);

        std::env::remove_var("BUZZ_EMBED_PROVIDER");
        std::env::remove_var("BUZZ_RERANK_PROVIDER");
    }

    #[tokio::test]
    async fn test_fallback_to_local_when_cloud_keys_missing() {
        let config = AiConfig::default().with_embed_provider(EmbedProviderType::OpenAi);
        // Without BUZZ_OPENAI_API_KEY set, it should cleanly fall back to local ONNX
        let embedder = config.build_embedder().expect("fallback to local");
        assert_eq!(embedder.provider_name(), "local-onnx");
        assert_eq!(embedder.dimensions(), 384);
    }
}
