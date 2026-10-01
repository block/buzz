#![deny(unsafe_code)]
//! Data Re-Trial & Confidence Verification Loop for SuperRAG.
//!
//! Automatically inspects candidate retrieval distributions and triggers
//! adaptive re-trial passes when confidence falls below the target threshold.

use crate::fusion::CandidateChunk;

/// Decision produced by the confidence evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum ReTrialDecision {
    /// High-confidence retrieval result; proceed directly to cross-encoder reranking.
    Confident,
    /// Low confidence or sparse result pool; execute adaptive re-trial pass.
    NeedsReTrial {
        /// Reformulated, keyword-stemmed query.
        reformulated_query: String,
        /// Expanded graph neighborhood hops (e.g. 3 hops).
        expanded_hops: u8,
        /// Rationale for triggering re-trial.
        reason: &'static str,
    },
}

/// Evaluator that guards retrieval quality before context compilation.
pub struct ConfidenceEvaluator {
    min_confidence_threshold: f32,
    min_candidate_count: usize,
}

impl Default for ConfidenceEvaluator {
    fn default() -> Self {
        Self {
            min_confidence_threshold: 0.65,
            min_candidate_count: 3,
        }
    }
}

impl ConfidenceEvaluator {
    /// Creates a custom confidence evaluator.
    pub fn new(min_confidence_threshold: f32, min_candidate_count: usize) -> Self {
        Self {
            min_confidence_threshold,
            min_candidate_count,
        }
    }

    /// Evaluates a candidate pool and determines whether to proceed or re-trial.
    pub fn evaluate(&self, candidates: &[CandidateChunk], original_query: &str) -> ReTrialDecision {
        if candidates.is_empty() {
            return ReTrialDecision::NeedsReTrial {
                reformulated_query: Self::reformulate(original_query),
                expanded_hops: 3,
                reason: "Empty candidate pool",
            };
        }

        if candidates.len() < self.min_candidate_count {
            return ReTrialDecision::NeedsReTrial {
                reformulated_query: Self::reformulate(original_query),
                expanded_hops: 3,
                reason: "Sparse candidate pool (< 3 hits)",
            };
        }

        // Relative confidence metric: ratio between top score and mean
        let top_score = candidates[0].score;
        let avg_score: f32 = candidates.iter().map(|c| c.score).sum::<f32>() / candidates.len() as f32;
        let relative_confidence = if avg_score > 0.0 {
            (top_score / (top_score + avg_score)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        if relative_confidence < self.min_confidence_threshold && candidates.len() < 5 {
            return ReTrialDecision::NeedsReTrial {
                reformulated_query: Self::reformulate(original_query),
                expanded_hops: 3,
                reason: "Low relative confidence distribution",
            };
        }

        ReTrialDecision::Confident
    }

    /// Generates a relaxed, keyword-dense reformulation.
    pub fn reformulate(query: &str) -> String {
        let clean: String = query
            .chars()
            .map(|c| if c.is_alphanumeric() || c.is_whitespace() || c == '_' { c } else { ' ' })
            .collect();

        let stop_words = [
            "the", "a", "an", "how", "what", "where", "why", "when", "does", "is", "are",
            "in", "on", "to", "for", "with", "we", "can", "please", "find",
        ];

        let keywords: Vec<&str> = clean
            .split_whitespace()
            .filter(|w| !stop_words.contains(&w.to_ascii_lowercase().as_str()))
            .collect();

        if keywords.is_empty() {
            query.trim().to_string()
        } else {
            keywords.join(" ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_candidate(score: f32) -> CandidateChunk {
        CandidateChunk {
            chunk_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            content: "test".to_string(),
            workspace_path: "/ws".to_string(),
            source_uri: "test.rs".to_string(),
            source_type: "code".to_string(),
            line_start: Some(1),
            line_end: Some(5),
            created_at: Utc::now(),
            score,
            authority: 1.0,
        }
    }

    #[test]
    fn test_retrial_triggered_on_empty_pool() {
        let eval = ConfidenceEvaluator::default();
        let res = eval.evaluate(&[], "how does the auth work?");
        assert!(matches!(res, ReTrialDecision::NeedsReTrial { .. }));
    }

    #[test]
    fn test_retrial_triggered_on_sparse_pool() {
        let eval = ConfidenceEvaluator::default();
        let pool = vec![make_candidate(0.9)];
        let res = eval.evaluate(&pool, "how does the auth work?");
        assert!(matches!(res, ReTrialDecision::NeedsReTrial { .. }));
    }

    #[test]
    fn test_confident_pool_proceeds() {
        let eval = ConfidenceEvaluator::default();
        let pool = vec![
            make_candidate(0.95),
            make_candidate(0.4),
            make_candidate(0.3),
            make_candidate(0.2),
        ];
        let res = eval.evaluate(&pool, "auth");
        assert_eq!(res, ReTrialDecision::Confident);
    }

    #[test]
    fn test_reformulate_query() {
        let clean = ConfidenceEvaluator::reformulate("How does the relay authenticate connections?");
        assert_eq!(clean, "relay authenticate connections");
    }
}
