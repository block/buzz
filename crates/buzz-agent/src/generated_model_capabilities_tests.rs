//! Tests for generated model capabilities — normative corpus + handwritten supplements.
//!
//! This module is conditionally compiled as #[cfg(test)] from generated_model_capabilities.rs.
//!
//! Three layers:
//! 1. Shared normative corpus executed against the Rust interpreter — every vector in
//!    `scripts/normative-corpus.json` is deserialized and asserted here. This is the
//!    cross-interpreter conformance gate; the JS runner executes the same file.
//! 2. Handwritten supplement tests — adversarial cases and completeness checks that
//!    benefit from Rust-specific assertion ergonomics.
//! 3. Per-interpreter mutation evidence — all 7 mutations were killed by both interpreters
//!    (2026-07-31, Phase 2). The mutation runner was deleted in Phase 3; see
//!    `scripts/MODEL_CAPABILITIES.md` for the historical record.

#[cfg(test)]
mod shared_corpus_tests {
    //! Executes every test vector in `scripts/normative-corpus.json` against the
    //! Rust `resolve_model_capabilities` interpreter.
    //!
    //! The corpus JSON is located at `../../scripts/normative-corpus.json` relative
    //! to the crate root (i.e., `<repo>/scripts/normative-corpus.json`). In the test
    //! binary, `CARGO_MANIFEST_DIR` points at the crate directory.

    use crate::config::ThinkingEffort;
    use crate::generated_model_capabilities::{
        resolve_model_capabilities, DatabricksV2Route, ThinkingMode,
    };
    use serde::Deserialize;
    use std::path::Path;

    // ---------------------------------------------------------------------------
    // Corpus schema (mirrors normative-corpus.json structure)
    // ---------------------------------------------------------------------------

    #[derive(Deserialize)]
    struct CorpusEntry {
        id: Option<String>,
        provider: Option<String>,
        raw_model_id: Option<String>,
        expect: Option<CorpusExpect>,
    }

    #[derive(Deserialize)]
    struct CorpusExpect {
        thinking_mode: Option<String>,
        supported_efforts: Option<Vec<String>>,
        default_effort: Option<serde_json::Value>, // string or null
        databricks_v2_wire_route: Option<String>,
    }

    // ---------------------------------------------------------------------------
    // Conversion helpers
    // ---------------------------------------------------------------------------

    fn parse_thinking_mode(s: &str) -> ThinkingMode {
        match s {
            "manual-budget" => ThinkingMode::ManualBudget,
            "adaptive" => ThinkingMode::Adaptive,
            "omit-fields" => ThinkingMode::OmitFields,
            "none" => ThinkingMode::None,
            "not-applicable" => ThinkingMode::NotApplicable,
            other => panic!("unknown thinking_mode in corpus: {other}"),
        }
    }

    fn parse_effort(s: &str) -> ThinkingEffort {
        match s {
            "minimal" => ThinkingEffort::Minimal,
            "none" => ThinkingEffort::None,
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "xhigh" => ThinkingEffort::XHigh,
            "max" => ThinkingEffort::Max,
            other => panic!("unknown effort in corpus: {other}"),
        }
    }

    fn parse_route(s: &str) -> DatabricksV2Route {
        match s {
            "openai-responses" => DatabricksV2Route::OpenAiResponses,
            "anthropic-messages" => DatabricksV2Route::AnthropicMessages,
            "mlflow-chat" => DatabricksV2Route::MlflowChatCompletions,
            "route-unknown" => DatabricksV2Route::RouteUnknown,
            "not-applicable" => DatabricksV2Route::NotApplicable,
            other => panic!("unknown databricks_v2_wire_route in corpus: {other}"),
        }
    }

    // ---------------------------------------------------------------------------
    // Corpus loader
    // ---------------------------------------------------------------------------

    fn load_corpus() -> Vec<CorpusEntry> {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let corpus_path = Path::new(manifest_dir)
            .join("..") // crates/buzz-agent → crates
            .join("..") // crates → repo root
            .join("scripts")
            .join("normative-corpus.json");
        let corpus_str = std::fs::read_to_string(&corpus_path).unwrap_or_else(|e| {
            panic!(
                "failed to read normative-corpus.json at {}: {e}",
                corpus_path.display()
            )
        });
        serde_json::from_str(&corpus_str)
            .unwrap_or_else(|e| panic!("failed to parse normative-corpus.json: {e}"))
    }

    // ---------------------------------------------------------------------------
    // Corpus runner
    // ---------------------------------------------------------------------------

    #[test]
    fn test_shared_corpus_all_vectors() {
        let entries = load_corpus();
        let mut ran = 0;
        let mut failures: Vec<String> = vec![];

        for entry in &entries {
            // Skip group-marker objects (no id field)
            let id = match &entry.id {
                Some(id) => id.clone(),
                None => continue,
            };
            let provider = entry
                .provider
                .as_deref()
                .unwrap_or_else(|| panic!("corpus vector {id} missing provider"));
            let raw_model_id = entry
                .raw_model_id
                .as_deref()
                .unwrap_or_else(|| panic!("corpus vector {id} missing raw_model_id"));
            let expect = entry
                .expect
                .as_ref()
                .unwrap_or_else(|| panic!("corpus vector {id} missing expect"));

            // Canonicalize provider aliases before resolving — mirrors the
            // production path where Rust normalizes "openai-compat" → Provider::OpenAi
            // (config.rs) and the TS canonicalizeProvider() resolves "databricks-v2"
            // → "databricks_v2" before generated lookups.
            let canonical_provider = match provider {
                "openai-compat" => "openai",
                "databricks-v2" => "databricks_v2",
                other => other,
            };
            let result = resolve_model_capabilities(canonical_provider, raw_model_id);
            ran += 1;

            // Check thinking_mode if present in expect
            if let Some(expected_mode) = &expect.thinking_mode {
                let expected = parse_thinking_mode(expected_mode);
                if result.thinking_mode != expected {
                    failures.push(format!(
                        "[{id}] thinking_mode: got {:?}, expected {:?}",
                        result.thinking_mode, expected
                    ));
                }
            }

            // Check supported_efforts if present
            if let Some(expected_efforts) = &expect.supported_efforts {
                let expected: Vec<ThinkingEffort> =
                    expected_efforts.iter().map(|s| parse_effort(s)).collect();
                let actual: Vec<ThinkingEffort> =
                    result.supported_efforts.iter().cloned().collect();
                if actual != expected {
                    failures.push(format!(
                        "[{id}] supported_efforts: got {actual:?}, expected {expected:?}"
                    ));
                }
            }

            // Check default_effort if present
            if let Some(expected_de) = &expect.default_effort {
                let expected_parsed = match expected_de {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => Some(parse_effort(s)),
                    other => {
                        panic!("unexpected default_effort value in corpus vector {id}: {other:?}")
                    }
                };
                if result.default_effort != expected_parsed {
                    failures.push(format!(
                        "[{id}] default_effort: got {:?}, expected {:?}",
                        result.default_effort, expected_parsed
                    ));
                }
            }

            // Check databricks_v2_wire_route if present
            if let Some(expected_route) = &expect.databricks_v2_wire_route {
                let expected = parse_route(expected_route);
                if result.databricks_v2_wire_route != expected {
                    failures.push(format!(
                        "[{id}] databricks_v2_wire_route: got {:?}, expected {:?}",
                        result.databricks_v2_wire_route, expected
                    ));
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "{}/{} corpus vectors FAILED:\n{}",
                failures.len(),
                ran,
                failures.join("\n")
            );
        }
        assert!(
            ran > 0,
            "corpus was empty — check normative-corpus.json path"
        );
        println!("shared corpus: {ran} vectors all passed");
    }
}

#[cfg(test)]
mod corpus_tests {
    use crate::config::ThinkingEffort;
    use crate::generated_model_capabilities::{
        resolve_model_capabilities, DatabricksV2Route, ThinkingMode,
    };

    fn resolve(
        provider: &str,
        model: &str,
    ) -> crate::generated_model_capabilities::CapabilityResult {
        resolve_model_capabilities(provider, model)
    }

    // ---------------------------------------------------------------------------
    // Anthropic manual-budget family
    // ---------------------------------------------------------------------------

    #[test]
    fn test_anthropic_claude3_family_manual_budget() {
        let r = resolve("anthropic", "claude-3-7-sonnet-20250219");
        assert_eq!(r.thinking_mode, ThinkingMode::ManualBudget);
        assert_eq!(
            r.supported_efforts.as_ref(),
            &[
                ThinkingEffort::Low,
                ThinkingEffort::Medium,
                ThinkingEffort::High
            ]
        );
        assert_eq!(r.default_effort, None);
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_claude_opus_4_5_manual_budget() {
        let r = resolve("anthropic", "claude-opus-4-5");
        assert_eq!(r.thinking_mode, ThinkingMode::ManualBudget);
        assert_eq!(
            r.supported_efforts.as_ref(),
            &[
                ThinkingEffort::Low,
                ThinkingEffort::Medium,
                ThinkingEffort::High
            ]
        );
        assert_eq!(r.default_effort, None);
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    // ---------------------------------------------------------------------------
    // Anthropic adaptive xhigh-capable
    // ---------------------------------------------------------------------------

    #[test]
    fn test_anthropic_claude_opus_4_7_adaptive_xhigh() {
        let r = resolve("anthropic", "claude-opus-4-7");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert_eq!(
            r.supported_efforts.as_ref(),
            &[
                ThinkingEffort::Low,
                ThinkingEffort::Medium,
                ThinkingEffort::High,
                ThinkingEffort::XHigh,
                ThinkingEffort::Max
            ]
        );
        assert_eq!(r.default_effort, Some(ThinkingEffort::High));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_claude_sonnet_5_adaptive_xhigh() {
        let r = resolve("anthropic", "claude-sonnet-5-20260101");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert!(r.supported_efforts.contains(&ThinkingEffort::Max));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_claude_fable_5_adaptive_xhigh() {
        let r = resolve("anthropic", "claude-fable-5");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_claude_mythos_5_adaptive_xhigh() {
        let r = resolve("anthropic", "claude-mythos-5");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    // ---------------------------------------------------------------------------
    // Anthropic adaptive no-xhigh
    // ---------------------------------------------------------------------------

    #[test]
    fn test_anthropic_claude_opus_4_6_adaptive_no_xhigh() {
        let r = resolve("anthropic", "claude-opus-4-6");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(!r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert!(r.supported_efforts.contains(&ThinkingEffort::Max));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_claude_sonnet_4_6_adaptive_no_xhigh() {
        let r = resolve("anthropic", "claude-sonnet-4-6");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(!r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_claude_mythos_preview_adaptive_no_xhigh() {
        let r = resolve("anthropic", "claude-mythos-preview");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(!r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    // ---------------------------------------------------------------------------
    // Anthropic fallbacks
    // ---------------------------------------------------------------------------

    #[test]
    fn test_anthropic_blank_adaptive_fallback() {
        let r = resolve("anthropic", "");
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.default_effort, Some(ThinkingEffort::High));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_anthropic_concrete_unknown_omit_fields() {
        let r = resolve("anthropic", "claude-ultra-9000");
        assert_eq!(r.thinking_mode, ThinkingMode::OmitFields);
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    // ---------------------------------------------------------------------------
    // OpenAI gpt-5 family
    // ---------------------------------------------------------------------------

    #[test]
    fn test_openai_gpt5_pro_high_only() {
        let r = resolve("openai", "gpt-5-pro");
        assert_eq!(r.supported_efforts.as_ref(), &[ThinkingEffort::High]);
        assert_eq!(r.default_effort, Some(ThinkingEffort::High));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_openai_gpt5_6_max_capable() {
        let r = resolve("openai", "gpt-5.6");
        assert!(r.supported_efforts.contains(&ThinkingEffort::Max));
        assert!(r.supported_efforts.contains(&ThinkingEffort::None));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_openai_gpt5_5_no_max() {
        let r = resolve("openai", "gpt-5.5");
        assert!(!r.supported_efforts.contains(&ThinkingEffort::Max));
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_openai_gpt5_1_no_xhigh() {
        let r = resolve("openai", "gpt-5.1");
        assert!(!r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert!(r.supported_efforts.contains(&ThinkingEffort::None));
        assert_eq!(r.default_effort, Some(ThinkingEffort::None));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    #[test]
    fn test_openai_gpt5_base_minimal() {
        let r = resolve("openai", "gpt-5");
        assert!(r.supported_efforts.contains(&ThinkingEffort::Minimal));
        assert!(!r.supported_efforts.contains(&ThinkingEffort::None));
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
    }

    // ---------------------------------------------------------------------------
    // OpenAI adversarial boundary cases
    // ---------------------------------------------------------------------------

    #[test]
    fn test_openai_gpt5_1106_matches_base_not_version() {
        // 4-digit suffix is a date, not a version number — must match base table
        let r = resolve("openai", "gpt-5-1106");
        let base = resolve("openai", "gpt-5");
        assert_eq!(
            r.supported_efforts, base.supported_efforts,
            "gpt-5-1106 must match base table (4-digit date segment)"
        );
        assert!(
            !r.supported_efforts.contains(&ThinkingEffort::None),
            "gpt-5-1106 base must NOT have none"
        );
        assert!(
            r.supported_efforts.contains(&ThinkingEffort::Minimal),
            "gpt-5-1106 base must have minimal"
        );
    }

    #[test]
    fn test_openai_gpt5_4o_matches_base_not_gpt5_4() {
        // gpt-5-4o: '4o' after '-' is not a short version suffix (letter present)
        // Must NOT match gpt-5.4 (would add xhigh), must match gpt-5 base
        let r = resolve("openai", "gpt-5-4o");
        let base = resolve("openai", "gpt-5");
        assert_eq!(
            r.supported_efforts, base.supported_efforts,
            "gpt-5-4o must match base table"
        );
        assert!(
            !r.supported_efforts.contains(&ThinkingEffort::XHigh),
            "gpt-5-4o must NOT have xhigh (that's gpt-5.4 territory)"
        );
    }

    #[test]
    fn test_openai_gpt5_pro_before_base() {
        let pro = resolve("openai", "gpt-5-pro");
        let base = resolve("openai", "gpt-5");
        assert_ne!(
            pro.supported_efforts, base.supported_efforts,
            "gpt-5-pro must hit -pro table, not base"
        );
        assert_eq!(pro.supported_efforts.as_ref(), &[ThinkingEffort::High]);
    }

    #[test]
    fn test_openai_gpt5_date_suffix_still_matches_base() {
        // gpt-5-20260101: 8-digit date suffix, must match base
        let r = resolve("openai", "gpt-5-20260101");
        let base = resolve("openai", "gpt-5");
        assert_eq!(
            r.supported_efforts, base.supported_efforts,
            "gpt-5-20260101 must match base (4+ digit date)"
        );
    }

    // ---------------------------------------------------------------------------
    // DatabricksV2 routing
    // ---------------------------------------------------------------------------

    #[test]
    fn test_dbv2_gpt5_routes_openai_responses() {
        let r = resolve("databricks_v2", "gpt-5.5");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::OpenAiResponses
        );
        assert!(!r.supported_efforts.contains(&ThinkingEffort::Max));
    }

    #[test]
    fn test_dbv2_claude_routes_anthropic_messages() {
        let r = resolve("databricks_v2", "claude-opus-4-7");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::AnthropicMessages
        );
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
    }

    #[test]
    fn test_dbv2_databricks_prefix_stripped() {
        let r = resolve("databricks_v2", "databricks-claude-opus-4-7");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::AnthropicMessages
        );
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
    }

    #[test]
    fn test_dbv2_goose_claude_prefix_stripped() {
        let r = resolve("databricks_v2", "goose-claude-fable-5");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::AnthropicMessages
        );
        assert_eq!(r.thinking_mode, ThinkingMode::Adaptive);
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
    }

    #[test]
    fn test_dbv2_team_prefix_stripped() {
        let r = resolve("databricks_v2", "team-x-claude-opus-4-7");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::AnthropicMessages
        );
    }

    // Segment collision tests — these must NOT match named code names as substrings
    #[test]
    fn test_dbv2_consolidated_llama_not_sol_segment() {
        // 'sol' is a substring of 'consolidated' — segment match must prevent this
        let r = resolve("databricks_v2", "consolidated-llama");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::MlflowChatCompletions,
            "consolidated-llama must NOT route OpenAI via 'sol' substring"
        );
    }

    #[test]
    fn test_dbv2_terraform_coder_not_terra_segment() {
        // 'terra' is a prefix of 'terraform' — segment match must prevent this
        let r = resolve("databricks_v2", "terraform-coder");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::MlflowChatCompletions,
            "terraform-coder must NOT route OpenAI via 'terra' prefix-of-'terraform'"
        );
    }

    #[test]
    fn test_dbv2_corpus_reranker_not_opus_segment() {
        // 'opus' must be a SEGMENT (full token), not a substring of 'corpus'
        let r = resolve("databricks_v2", "corpus-reranker");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::MlflowChatCompletions,
            "corpus-reranker must NOT route Anthropic via 'opus' substring of 'corpus'"
        );
    }

    #[test]
    fn test_dbv2_octopus_model_not_opus_segment() {
        let r = resolve("databricks_v2", "octopus-model");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::MlflowChatCompletions,
            "octopus-model must NOT route Anthropic (no 'opus' segment)"
        );
    }

    #[test]
    fn test_dbv2_goose_opus_5_is_anthropic() {
        // goose-opus-5: segments are [goose, opus, 5]; 'opus' is a segment → Anthropic route
        let r = resolve("databricks_v2", "goose-opus-5");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::AnthropicMessages,
            "goose-opus-5 must route Anthropic ('opus' is a segment)"
        );
    }

    // ---------------------------------------------------------------------------
    // P2-A resolver contract vectors
    // ---------------------------------------------------------------------------

    #[test]
    fn test_resolver_exact_raw_id_hit() {
        // databricks-gpt-5-4-mini has an exact record with low|medium|high override
        let r = resolve("databricks_v2", "databricks-gpt-5-4-mini");
        assert_eq!(
            r.supported_efforts.as_ref(),
            &[
                ThinkingEffort::Low,
                ThinkingEffort::Medium,
                ThinkingEffort::High
            ],
            "exact record must override family rule (models.dev: low|medium|high, not none+xhigh)"
        );
    }

    #[test]
    fn test_resolver_prefixed_alias_misses_exact() {
        // team-x-databricks-gpt-5-4-mini: raw exact lookup MUST miss (different raw ID)
        // Falls to family rules (gpt5-4 → none+xhigh)
        let r = resolve("databricks_v2", "team-x-databricks-gpt-5-4-mini");
        assert!(
            r.supported_efforts.contains(&ThinkingEffort::XHigh),
            "prefixed alias must miss exact record and fall to family (which has xhigh)"
        );
        assert!(
            r.supported_efforts.contains(&ThinkingEffort::None),
            "prefixed alias must fall to gpt5-4 family (which has none)"
        );
    }

    #[test]
    fn test_resolver_cross_provider_misses_exact() {
        // Same raw ID but different provider — exact record is databricks_v2-scoped
        let r = resolve("openai", "databricks-gpt-5-4-mini");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::NotApplicable,
            "openai provider must get not-applicable (record is databricks_v2-scoped)"
        );
    }

    #[test]
    fn test_resolver_exact_record_complete_both_axes() {
        // databricks-gpt-5-6-sol: exact record with efforts from models.dev +
        // route materialized from gpt5-6 family rule
        let r = resolve("databricks_v2", "databricks-gpt-5-6-sol");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::OpenAiResponses,
            "exact record must have route from family rule materialization"
        );
        assert!(
            r.supported_efforts.contains(&ThinkingEffort::Max),
            "exact record must have max (from models.dev override)"
        );
    }

    // ---------------------------------------------------------------------------
    // P2-B blank vs concrete-unknown fallback vectors
    // ---------------------------------------------------------------------------

    #[test]
    fn test_dbv2_blank_route_unknown_all7() {
        let r = resolve("databricks_v2", "");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::RouteUnknown,
            "DBv2 blank must return route-unknown"
        );
        assert_eq!(
            r.supported_efforts.len(),
            7,
            "DBv2 blank must expose all 7 efforts"
        );
        assert_eq!(r.default_effort, Some(ThinkingEffort::Medium));
    }

    #[test]
    fn test_dbv2_concrete_unknown_mlflow_no_max() {
        let r = resolve("databricks_v2", "some-unknown-model-xyz");
        assert_eq!(
            r.databricks_v2_wire_route,
            DatabricksV2Route::MlflowChatCompletions,
            "DBv2 concrete unknown must return mlflow-chat"
        );
        assert!(
            !r.supported_efforts.contains(&ThinkingEffort::Max),
            "DBv2 concrete unknown must NOT expose max"
        );
        assert_eq!(r.supported_efforts.len(), 6);
    }

    #[test]
    fn test_openai_blank_all_except_max() {
        let r = resolve("openai", "");
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
        assert!(!r.supported_efforts.contains(&ThinkingEffort::Max));
        assert_eq!(r.default_effort, Some(ThinkingEffort::Medium));
    }

    #[test]
    fn test_openai_concrete_unknown_all_except_max() {
        let r = resolve("openai", "gpt-4o");
        assert_eq!(r.databricks_v2_wire_route, DatabricksV2Route::NotApplicable);
        assert!(!r.supported_efforts.contains(&ThinkingEffort::Max));
        assert_eq!(r.default_effort, Some(ThinkingEffort::Medium));
    }

    #[test]
    fn test_anthropic_blank_adaptive_full_fallback() {
        let r = resolve("anthropic", "");
        assert_eq!(
            r.thinking_mode,
            ThinkingMode::Adaptive,
            "Anthropic blank must assume adaptive"
        );
        assert!(r.supported_efforts.contains(&ThinkingEffort::XHigh));
        assert_eq!(r.default_effort, Some(ThinkingEffort::High));
    }

    #[test]
    fn test_anthropic_concrete_unknown_omit_fields_fallback() {
        let r = resolve("anthropic", "claude-ultra-9000");
        assert_eq!(
            r.thinking_mode,
            ThinkingMode::OmitFields,
            "Anthropic concrete unknown must omit fields (never guess request shape)"
        );
    }

    // ---------------------------------------------------------------------------
    // Completeness: every result has every axis populated
    // ---------------------------------------------------------------------------

    #[test]
    fn test_every_result_is_complete() {
        let test_cases = vec![
            ("anthropic", ""),
            ("anthropic", "claude-3-7-sonnet-20250219"),
            ("anthropic", "claude-opus-4-7"),
            ("anthropic", "claude-ultra-9000"),
            ("openai", ""),
            ("openai", "gpt-5"),
            ("openai", "gpt-5.6"),
            ("openai", "unknown-model"),
            ("databricks_v2", ""),
            ("databricks_v2", "databricks-gpt-5-4-mini"),
            ("databricks_v2", "goose-claude-fable-5"),
            ("databricks_v2", "unknown-model"),
            ("databricks", ""),
            ("openrouter", ""),
            ("openai-compat", ""),
            ("", ""),
        ];
        for (provider, model) in test_cases {
            let r = resolve_model_capabilities(provider, model);
            assert!(
                !r.supported_efforts.is_empty(),
                "supported_efforts must be non-empty for ({provider}, {model})"
            );
            // default_effort is allowed to be None (manual-budget "Inherit")
            // registry_label is allowed to be None
        }
    }
}
