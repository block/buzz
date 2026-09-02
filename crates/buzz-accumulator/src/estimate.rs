//! Zero-spend input-size estimates and context-window fit.
//!
//! Estimates never call a model — by construction, not by discipline: this
//! module has no runner access. Windows are curated rather than discovered;
//! an unknown model's fit deliberately remains `None` instead of becoming
//! false precision. Deliberately no dollar figures: a curated USD table that
//! ignores output tokens goes silently stale — a UI can price tokens × rate
//! client-side if it wants money on screen.

use serde::Serialize;

/// Documented context window for the Claude aliases below.
const CLAUDE_WINDOW: u64 = 200_000;

/// Curated input-context capacities by model alias.
const MODEL_WINDOWS: &[(&str, u64)] = &[
    ("haiku", CLAUDE_WINDOW),
    ("sonnet", CLAUDE_WINDOW),
    ("opus", CLAUDE_WINDOW),
];

/// Whether an input estimate fits a model's curated window.
///
/// `headroom_tokens` is intentionally negative for an overfull known window so
/// the amount of overflow is observable; both fields are `None` for a model
/// whose window is not curated.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowFit {
    /// Curated window size, if known.
    pub model_window: Option<u64>,
    /// The estimate being fitted.
    pub est_input_tokens: u64,
    /// Whether it fits; `None` when the window is unknown.
    pub fits: Option<bool>,
    /// `window - estimate`; negative when overfull, `None` when unknown.
    pub headroom_tokens: Option<i64>,
}

/// An input-only estimate for one fold run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Estimate {
    /// chars/4 heuristic, clearly labeled as such.
    pub est_input_tokens: u64,
    /// Window fit for the model.
    pub window_fit: WindowFit,
}

/// The clearly labeled chars/4 input-token heuristic.
pub fn estimate_tokens(chars: usize) -> u64 {
    chars.div_ceil(4) as u64
}

/// Output tokens reserved out of the model window before sizing the input.
pub const RESERVED_OUTPUT_TOKENS: u64 = 16_384;

/// Safety margin reserved for tokenizer drift (chars/4 is a heuristic) and
/// the runner's own system prompt.
pub const SAFETY_MARGIN_TOKENS: u64 = 8_192;

/// Conservative input budget when the model's window is not curated — the
/// pre-model-aware planning ceiling, kept as the honest fallback.
pub const FALLBACK_INPUT_BUDGET_CHARS: usize = 120_000;

/// Emergency hard cap on one run's input, whatever the window says. A guard,
/// not a target: it only binds if a future curated window would push a single
/// input past a megabyte of text.
pub const EMERGENCY_MAX_INPUT_CHARS: usize = 1_000_000;

/// How much input one run may plan for a model, and where that number came
/// from. Derived from the curated window minus explicit reservations —
/// preflight surfaces every term so a boundary is always explainable.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextBudget {
    /// Curated window size, if known.
    pub model_window: Option<u64>,
    /// Tokens reserved for the model's output.
    pub reserved_output_tokens: u64,
    /// Tokens reserved for estimate drift + runner system prompt.
    pub safety_margin_tokens: u64,
    /// Tokens the planned input may use (window − reservations, or the
    /// fallback for an unknown model).
    pub input_budget_tokens: u64,
    /// The same budget in chars (tokens × 4), the unit planning works in.
    pub input_budget_chars: usize,
}

/// Compute the planning budget for `model`.
///
/// Known window: `window − reserved output − safety margin`, in chars, capped
/// by [`EMERGENCY_MAX_INPUT_CHARS`]. Unknown window: the honest conservative
/// fallback rather than a guess.
pub fn context_budget(model: &str) -> ContextBudget {
    match MODEL_WINDOWS.iter().find(|(m, _)| *m == model) {
        Some((_, window)) => {
            let tokens = window.saturating_sub(RESERVED_OUTPUT_TOKENS + SAFETY_MARGIN_TOKENS);
            let chars = ((tokens as usize) * 4).min(EMERGENCY_MAX_INPUT_CHARS);
            ContextBudget {
                model_window: Some(*window),
                reserved_output_tokens: RESERVED_OUTPUT_TOKENS,
                safety_margin_tokens: SAFETY_MARGIN_TOKENS,
                input_budget_tokens: (chars / 4) as u64,
                input_budget_chars: chars,
            }
        }
        None => ContextBudget {
            model_window: None,
            reserved_output_tokens: RESERVED_OUTPUT_TOKENS,
            safety_margin_tokens: SAFETY_MARGIN_TOKENS,
            input_budget_tokens: (FALLBACK_INPUT_BUDGET_CHARS / 4) as u64,
            input_budget_chars: FALLBACK_INPUT_BUDGET_CHARS,
        },
    }
}

/// Report whether an input-token estimate fits `model`'s curated window.
pub fn window_fit(model: &str, est_input_tokens: u64) -> WindowFit {
    match MODEL_WINDOWS.iter().find(|(m, _)| *m == model) {
        None => WindowFit {
            model_window: None,
            est_input_tokens,
            fits: None,
            headroom_tokens: None,
        },
        Some((_, window)) => {
            let headroom = *window as i64 - est_input_tokens as i64;
            WindowFit {
                model_window: Some(*window),
                est_input_tokens,
                fits: Some(headroom >= 0),
                headroom_tokens: Some(headroom),
            }
        }
    }
}

/// Estimate one run's input size and window fit.
pub fn estimate(model: &str, chars: usize) -> Estimate {
    let tokens = estimate_tokens(chars);
    let fit = window_fit(model, tokens);
    Estimate {
        est_input_tokens: tokens,
        window_fit: fit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_up() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(1), 1);
        assert_eq!(estimate_tokens(4), 1);
        assert_eq!(estimate_tokens(5), 2);
    }

    #[test]
    fn known_model_reports_overflow() {
        let e = estimate("haiku", 4_000_000);
        assert_eq!(e.est_input_tokens, 1_000_000);
        assert_eq!(e.window_fit.fits, Some(false));
        assert_eq!(e.window_fit.headroom_tokens, Some(200_000 - 1_000_000));
    }

    #[test]
    fn unknown_model_is_honest_none() {
        let e = estimate("mystery-model", 400);
        assert_eq!(e.est_input_tokens, 100);
        assert_eq!(e.window_fit.fits, None);
        assert_eq!(e.window_fit.model_window, None);
    }

    #[test]
    fn fitting_input_has_positive_headroom() {
        let f = window_fit("sonnet", 10);
        assert_eq!(f.fits, Some(true));
        assert_eq!(f.headroom_tokens, Some(199_990));
    }

    #[test]
    fn known_model_budget_reserves_output_and_safety() {
        let b = context_budget("haiku");
        assert_eq!(b.model_window, Some(200_000));
        assert_eq!(
            b.input_budget_tokens,
            200_000 - RESERVED_OUTPUT_TOKENS - SAFETY_MARGIN_TOKENS
        );
        assert_eq!(b.input_budget_chars, b.input_budget_tokens as usize * 4);
        assert!(
            b.input_budget_chars > 4 * FALLBACK_INPUT_BUDGET_CHARS,
            "a 200k window must not be capped near the 30k-token fallback"
        );
        assert!(b.input_budget_chars <= EMERGENCY_MAX_INPUT_CHARS);
    }

    #[test]
    fn unknown_model_budget_falls_back_conservatively() {
        let b = context_budget("mystery-model");
        assert_eq!(b.model_window, None);
        assert_eq!(b.input_budget_chars, FALLBACK_INPUT_BUDGET_CHARS);
        assert_eq!(b.input_budget_tokens, 30_000);
    }
}
