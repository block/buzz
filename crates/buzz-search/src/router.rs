#![deny(unsafe_code)]
//! Pre-retrieval query routing arbiter and intent classifier for SuperRAG.
//!
//! Intelligently classifies developer and agent queries into:
//! - `Symbolic`: code symbols, function signatures, paths, commit SHAs, UUIDs.
//! - `Conceptual`: architectural concepts, natural language feature descriptions.
//! - `TemporalDecision`: ADRs, why decisions were made, superseded facts.
//! - `MultiHopRelationship`: dependencies, interactions across services and files.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Classified intent of an incoming search or recall query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryIntent {
    /// Exact code symbol, identifier, error message, or commit SHA.
    Symbolic {
        /// Extracted exact identifier.
        identifier: String,
    },
    /// Architectural pattern, system design, or concept.
    Conceptual {
        /// Normalized semantic query.
        semantic_query: String,
    },
    /// Historical decision, ADR, or superseded workflow.
    TemporalDecision {
        /// Decision topic or rationale question.
        topic: String,
        /// Optional temporal horizon upper bound.
        before_time: Option<DateTime<Utc>>,
    },
    /// Multi-hop structural dependency or graph neighborhood query.
    MultiHopRelationship {
        /// Seed entity name or identifier.
        seed_entity: String,
    },
}

/// Pre-retrieval routing arbiter.
pub struct QueryRouter;

impl QueryRouter {
    /// Classifies an incoming query string into a targeted `QueryIntent`.
    pub fn classify_intent(query: &str) -> QueryIntent {
        let trimmed = query.trim();

        // 1. Detect commit SHA (40-char hex or 7-char short SHA) or UUID
        if Self::is_hex_sha_or_uuid(trimmed) {
            return QueryIntent::Symbolic {
                identifier: trimmed.to_string(),
            };
        }

        // 2. Detect code identifier with path, scope operator, or function call
        // (e.g. `buzz::auth`, `verify_nip42()`, `crates/buzz-relay/src/lib.rs`, `struct PaymentService`)
        if trimmed.contains("::")
            || trimmed.contains("()")
            || (trimmed.contains('/') && !trimmed.contains(' '))
            || (trimmed.contains('\\') && !trimmed.contains(' '))
            || (trimmed.contains('.') && !trimmed.contains(' ') && (trimmed.ends_with(".rs") || trimmed.ends_with(".ts") || trimmed.ends_with(".py") || trimmed.ends_with(".go") || trimmed.ends_with(".md")))
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("class ")
        {
            return QueryIntent::Symbolic {
                identifier: trimmed.to_string(),
            };
        }

        let lower = trimmed.to_ascii_lowercase();

        // 3. Detect temporal questions and architectural decisions
        if lower.starts_with("why did")
            || lower.starts_with("why was")
            || lower.contains("decision")
            || lower.contains("adr")
            || lower.contains("superseded")
            || lower.contains("deprecated")
            || lower.contains("history of")
        {
            return QueryIntent::TemporalDecision {
                topic: trimmed.to_string(),
                before_time: None,
            };
        }

        // 4. Detect multi-hop structural relationships
        if lower.contains("depends on")
            || lower.contains("dependencies of")
            || lower.contains("connected to")
            || lower.contains("called by")
            || lower.contains("calls to")
            || lower.contains("related to")
        {
            return QueryIntent::MultiHopRelationship {
                seed_entity: trimmed.to_string(),
            };
        }

        // Default to conceptual search
        QueryIntent::Conceptual {
            semantic_query: trimmed.to_string(),
        }
    }

    /// Decomposes and expands ambiguous or broad queries into sub-queries.
    pub fn decompose_and_expand(query: &str, intent: &QueryIntent) -> Vec<String> {
        let mut expansions = vec![query.trim().to_string()];

        match intent {
            QueryIntent::Symbolic { identifier } => {
                // If scoped e.g. `buzz::auth::verify`, also search for leaf `verify`
                if let Some(leaf) = identifier.rsplit("::").next() {
                    if leaf != identifier && !leaf.is_empty() {
                        expansions.push(leaf.to_string());
                    }
                }
            }
            QueryIntent::TemporalDecision { topic, .. } => {
                let stripped = topic
                    .replace("why did we ", "")
                    .replace("why was ", "")
                    .replace("decision on ", "");
                if !stripped.is_empty() && stripped != *topic {
                    expansions.push(stripped);
                }
            }
            QueryIntent::Conceptual { semantic_query } => {
                // Generate keyword-dense variation by removing stop-words
                let keywords: Vec<&str> = semantic_query
                    .split_whitespace()
                    .filter(|w| !["how", "does", "the", "a", "an", "is", "what", "to", "in"].contains(&w.to_ascii_lowercase().as_str()))
                    .collect();
                if keywords.len() >= 2 {
                    expansions.push(keywords.join(" "));
                }
            }
            QueryIntent::MultiHopRelationship { seed_entity } => {
                expansions.push(seed_entity.clone());
            }
        }

        expansions
    }

    fn is_hex_sha_or_uuid(s: &str) -> bool {
        // UUID format check (36 chars with dashes)
        if uuid::Uuid::parse_str(s).is_ok() {
            return true;
        }

        // Git SHA check (40-char or 7-to-12 char hex)
        if (s.len() == 40 || (s.len() >= 7 && s.len() <= 12)) && s.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbolic_intent_classification() {
        let q1 = QueryRouter::classify_intent("buzz_core::memory::Document");
        assert!(matches!(q1, QueryIntent::Symbolic { .. }));

        let q2 = QueryRouter::classify_intent("verify_nip42()");
        assert!(matches!(q2, QueryIntent::Symbolic { .. }));

        let q3 = QueryRouter::classify_intent("crates/buzz-relay/src/main.rs");
        assert!(matches!(q3, QueryIntent::Symbolic { .. }));

        let q4 = QueryRouter::classify_intent("c0ffee0123456789abcdef0123456789abcdef01");
        assert!(matches!(q4, QueryIntent::Symbolic { .. }));
    }

    #[test]
    fn test_temporal_decision_intent_classification() {
        let q = QueryRouter::classify_intent("Why did we switch to SQLite instead of PostgreSQL?");
        assert!(matches!(q, QueryIntent::TemporalDecision { .. }));

        let q2 = QueryRouter::classify_intent("What is the ADR on vector storage?");
        assert!(matches!(q2, QueryIntent::TemporalDecision { .. }));
    }

    #[test]
    fn test_conceptual_and_multihop_classification() {
        let q = QueryRouter::classify_intent("How does the relay authenticate incoming WebSocket connections?");
        assert!(matches!(q, QueryIntent::Conceptual { .. }));

        let q2 = QueryRouter::classify_intent("What are the dependencies of PaymentService?");
        assert!(matches!(q2, QueryIntent::MultiHopRelationship { .. }));
    }

    #[test]
    fn test_query_expansion() {
        let intent = QueryIntent::Symbolic {
            identifier: "buzz::auth::keyring::resolve_secret".to_string(),
        };
        let expansions = QueryRouter::decompose_and_expand("buzz::auth::keyring::resolve_secret", &intent);
        assert!(expansions.contains(&"resolve_secret".to_string()));
    }
}
