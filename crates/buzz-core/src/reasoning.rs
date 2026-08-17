//! Provider-neutral reasoning effort and Anthropic model-family resolution.
//!
//! Both the native `buzz-agent` runner and external ACP harnesses consume this
//! module. Keeping Anthropic's model table here prevents the two launch paths
//! from silently choosing different thinking request shapes.

use std::str::FromStr;

/// Values accepted by `BUZZ_AGENT_THINKING_EFFORT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThinkingEffort {
    /// Disable reasoning when the provider supports that distinction.
    None,
    /// The provider's smallest non-zero reasoning level.
    Minimal,
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort.
    Medium,
    /// High reasoning effort.
    High,
    /// Extra-high reasoning effort.
    XHigh,
    /// Maximum reasoning effort.
    Max,
}

impl ThinkingEffort {
    /// Canonical environment/provider spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Compatibility name used by provider request builders.
    pub const fn openai_effort_str(self) -> &'static str {
        self.as_str()
    }

    /// Legacy Anthropic manual-thinking budget for this effort.
    pub const fn anthropic_budget_tokens(self) -> u32 {
        match self {
            Self::Low => 1_024,
            Self::Medium => 8_192,
            Self::High | Self::XHigh | Self::Max => 32_768,
            Self::None | Self::Minimal => 0,
        }
    }

    /// Anthropic adaptive-effort spelling.
    pub const fn anthropic_effort_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            // These values are rejected before Anthropic translation. Keep a
            // defensive low fallback so an accidental call cannot raise effort.
            Self::None | Self::Minimal => "low",
        }
    }
}

impl FromStr for ThinkingEffort {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            "max" => Ok(Self::Max),
            other => Err(format!(
                "BUZZ_AGENT_THINKING_EFFORT={other} is invalid; accepted values: none|minimal|low|medium|high|xhigh|max"
            )),
        }
    }
}

/// Anthropic thinking mechanism selected for a recognized model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicThinking {
    /// Legacy `thinking: {type: enabled, budgetTokens: ...}` mechanism.
    ManualBudget {
        /// Budget before any caller-specific output-token cap.
        budget_tokens: u32,
    },
    /// Modern `thinking: {type: adaptive}` plus an effort level.
    Adaptive {
        /// Model-supported effort after family-specific clamping.
        effort: ThinkingEffort,
    },
}

/// Resolve Anthropic's thinking mechanism for a model and requested effort.
///
/// Unknown/unverified models return `None`: callers must omit thinking fields
/// rather than guessing a request shape. Catalog prefixes are stripped by
/// locating the first `claude-` token, matching the native runner's historical
/// behavior for Databricks and Goose model IDs. A trailing context-window
/// annotation such as `[1m]` is metadata, not part of the model family, and is
/// removed before classification.
pub fn resolve_anthropic_thinking(
    effective_model: &str,
    effort: ThinkingEffort,
) -> Option<AnthropicThinking> {
    let model = normalize_claude_model_id(effective_model);
    if is_manual_budget_model(model) {
        return Some(AnthropicThinking::ManualBudget {
            budget_tokens: effort.anthropic_budget_tokens(),
        });
    }
    if is_adaptive_thinking_model(model) {
        return Some(AnthropicThinking::Adaptive {
            effort: clamp_adaptive_effort(model, effort),
        });
    }
    None
}

/// Whether a recognized Anthropic model uses a manual token budget.
pub fn is_manual_budget_model(effective_model: &str) -> bool {
    let model = normalize_claude_model_id(effective_model);
    model.starts_with("claude-3") || model == "claude-opus-4-5"
}

/// Whether a recognized Anthropic model uses adaptive thinking.
pub fn is_adaptive_thinking_model(effective_model: &str) -> bool {
    let model = normalize_claude_model_id(effective_model);
    model.starts_with("claude-opus-4-6")
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
        || model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-sonnet-4-6")
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
        || model.starts_with("claude-mythos-preview")
}

/// Whether an adaptive Anthropic model supports `xhigh`.
pub fn anthropic_model_supports_xhigh(effective_model: &str) -> bool {
    let model = normalize_claude_model_id(effective_model);
    model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
        || model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
}

/// Clamp adaptive effort to the model's documented capability.
pub fn clamp_adaptive_effort(effective_model: &str, effort: ThinkingEffort) -> ThinkingEffort {
    if effort == ThinkingEffort::XHigh && !anthropic_model_supports_xhigh(effective_model) {
        ThinkingEffort::High
    } else {
        effort
    }
}

fn strip_claude_catalog_prefix(model: &str) -> &str {
    let lower = model.to_ascii_lowercase();
    lower.find("claude-").map_or(model, |index| &model[index..])
}

fn strip_context_window_suffix(model: &str) -> &str {
    if !model.ends_with(']') {
        return model;
    }
    model.rfind('[').map_or(model, |index| &model[..index])
}

fn normalize_claude_model_id(model: &str) -> &str {
    strip_context_window_suffix(strip_claude_catalog_prefix(model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_model_is_omitted() {
        assert_eq!(
            resolve_anthropic_thinking("claude-future-9", ThinkingEffort::Max),
            None
        );
    }

    #[test]
    fn adaptive_high_xhigh_and_max_remain_distinct_when_supported() {
        let model = "claude-opus-4-8";
        for effort in [
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            assert_eq!(
                resolve_anthropic_thinking(model, effort),
                Some(AnthropicThinking::Adaptive { effort })
            );
        }
    }

    #[test]
    fn legacy_model_uses_budget() {
        assert_eq!(
            resolve_anthropic_thinking("claude-opus-4-5", ThinkingEffort::Medium),
            Some(AnthropicThinking::ManualBudget {
                budget_tokens: 8_192
            })
        );
    }

    #[test]
    fn context_window_suffix_is_removed_before_family_classification() {
        for model in [
            "claude-fable-5[1m]",
            "claude-opus-5[1m]",
            "goose-claude-fable-5[1m]",
            "claude-opus-5[200k]",
            "claude-opus-5[]",
        ] {
            assert_eq!(
                resolve_anthropic_thinking(model, ThinkingEffort::Medium),
                Some(AnthropicThinking::Adaptive {
                    effort: ThinkingEffort::Medium
                }),
                "model: {model}"
            );
        }
    }

    #[test]
    fn malformed_context_window_suffix_does_not_break_classification() {
        assert_eq!(
            resolve_anthropic_thinking("claude-opus-5[1m", ThinkingEffort::Medium),
            Some(AnthropicThinking::Adaptive {
                effort: ThinkingEffort::Medium
            })
        );
        assert_eq!(
            resolve_anthropic_thinking("opus[1m", ThinkingEffort::Medium),
            None
        );
    }
}
