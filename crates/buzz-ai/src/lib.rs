#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-ai` — Embedding & Cross-Encoder Reranker Engine for Orbit.
//!
//! Provides zero-network local ONNX inference (`bge-small-en-v1.5` embeddings,
//! `bge-reranker-small` cross-encoder) alongside cloud providers (OpenAI, Voyage,
//! Cohere, Gemini, Ollama) behind unified async traits.

/// Configuration and dynamic provider builders.
pub mod config;
/// Error types for the AI engine.
pub mod error;
/// Embedding provider abstraction and implementations.
pub mod provider;
/// Cross-encoder reranker provider abstraction and implementations.
pub mod rerank;

pub use config::{AiConfig, EmbedProviderType, RerankProviderType};
pub use error::{AiError, Result};
pub use provider::{
    CohereEmbedder, EmbedProvider, GeminiEmbedder, LocalOnnxEmbedder, OllamaEmbedder, OpenAiEmbedder,
    VoyageEmbedder,
};
pub use rerank::{CohereReranker, LocalOnnxReranker, RerankProvider, RerankResult, VoyageReranker};
