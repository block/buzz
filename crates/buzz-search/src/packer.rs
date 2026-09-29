#![deny(unsafe_code)]
//! Layer 5 Working Context Compiler and Greedy Knapsack Token Budget Packer.
//!
//! Formats winning retrieved chunks into bounded, deduplicated semantic XML:
//! `<orbit_context>...</orbit_context>` with strict provenance attribution.

use crate::fusion::CandidateChunk;

/// Packed working context ready for LLM consumption.
#[derive(Debug, Clone, PartialEq)]
pub struct PackedContext {
    /// Delimited semantic XML markup.
    pub context_xml: String,
    /// Total tokens allocated within the budget.
    pub allocated_tokens: usize,
    /// Number of chunks included.
    pub chunk_count: usize,
    /// Included candidate chunks in priority order.
    pub chunks: Vec<CandidateChunk>,
}

/// Token budget packer utilizing greedy knapsack packing.
pub struct TokenBudgetPacker {
    default_token_budget: usize,
}

impl Default for TokenBudgetPacker {
    fn default() -> Self {
        Self {
            default_token_budget: 4000,
        }
    }
}

impl TokenBudgetPacker {
    /// Creates a packer with a custom token budget.
    pub fn new(default_token_budget: usize) -> Self {
        Self {
            default_token_budget: default_token_budget.max(256),
        }
    }

    /// Estimates token count of a string (~4 chars per token).
    pub fn estimate_tokens(text: &str) -> usize {
        (text.len() / 4).max(1)
    }

    /// Greedily packs candidates into the token budget and generates `<orbit_context>` XML.
    pub fn pack(
        &self,
        workspace_path: &str,
        candidates: &[CandidateChunk],
        budget_override: Option<usize>,
    ) -> PackedContext {
        let budget = budget_override.unwrap_or(self.default_token_budget);
        let mut allocated_tokens = 0;
        let mut packed_chunks = Vec::new();

        // Candidates are expected to be pre-sorted in descending order of relevance
        for candidate in candidates {
            let chunk_tokens = Self::estimate_tokens(&candidate.content);
            if allocated_tokens + chunk_tokens > budget {
                if packed_chunks.is_empty() {
                    // Always include at least one top chunk even if slightly over small budget
                    packed_chunks.push(candidate.clone());
                    allocated_tokens += chunk_tokens;
                }
                break;
            }

            packed_chunks.push(candidate.clone());
            allocated_tokens += chunk_tokens;
        }

        // Build XML representation
        let mut xml = String::with_capacity(allocated_tokens * 4 + 256);
        xml.push_str(&format!(
            "<orbit_context workspace=\"{}\" total_chunks=\"{}\" budget_tokens=\"{}\" allocated_tokens=\"{}\">\n",
            workspace_path,
            packed_chunks.len(),
            budget,
            allocated_tokens
        ));

        for (idx, chunk) in packed_chunks.iter().enumerate() {
            let line_range = match (chunk.line_start, chunk.line_end) {
                (Some(s), Some(e)) => format!("lines=\"{s}-{e}\" "),
                _ => String::new(),
            };

            xml.push_str(&format!(
                "  <chunk index=\"{}\" source=\"{}\" {}score=\"{:.3}\" authority=\"{:.1}\">\n",
                idx + 1,
                chunk.source_uri,
                line_range,
                chunk.score,
                chunk.authority
            ));

            // Indent chunk content safely
            for line in chunk.content.lines() {
                xml.push_str("    ");
                xml.push_str(line);
                xml.push('\n');
            }

            xml.push_str("  </chunk>\n");
        }

        xml.push_str("</orbit_context>");

        PackedContext {
            context_xml: xml,
            allocated_tokens,
            chunk_count: packed_chunks.len(),
            chunks: packed_chunks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_test_candidate(content: &str, score: f32) -> CandidateChunk {
        CandidateChunk {
            chunk_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            content: content.to_string(),
            workspace_path: "/ws".to_string(),
            source_uri: "src/lib.rs".to_string(),
            source_type: "code".to_string(),
            line_start: Some(10),
            line_end: Some(25),
            created_at: Utc::now(),
            score,
            authority: 1.0,
        }
    }

    #[test]
    fn test_knapsack_budget_constraint() {
        let packer = TokenBudgetPacker::new(50); // small budget ~200 chars
        let c1 = make_test_candidate("fn first_func() { println!(\"1\"); }", 0.95);
        let c2 = make_test_candidate("fn second_func() { println!(\"2\"); }", 0.85);
        let c3 = make_test_candidate("fn large_func() { " .to_string().repeat(30).as_str(), 0.70);

        let packed = packer.pack("/ws", &[c1, c2, c3], Some(25));
        assert!(packed.allocated_tokens <= 40);
        assert!(packed.context_xml.starts_with("<orbit_context"));
        assert!(packed.context_xml.ends_with("</orbit_context>"));
    }

    #[test]
    fn test_xml_provenance_stamping() {
        let packer = TokenBudgetPacker::default();
        let c = make_test_candidate("pub fn authenticate() -> bool { true }", 0.98);
        let packed = packer.pack("/ws", &[c], None);

        assert!(packed.context_xml.contains("source=\"src/lib.rs\""));
        assert!(packed.context_xml.contains("lines=\"10-25\""));
        assert!(packed.context_xml.contains("score=\"0.980\""));
    }
}
