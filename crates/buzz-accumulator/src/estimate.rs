//! Zero-spend price estimates and context-window fit.
//!
//! Estimates never call a model — by construction, not by discipline: this
//! module has no runner access. Prices and context windows are curated rather
//! than discovered; an unknown model's cost deliberately remains `None`
//! instead of becoming false precision.

use serde::Serialize;

/// USD per million *input* tokens (curated; update deliberately with a source).
const PRICING: &[(&str, f64)] = &[("haiku", 0.80), ("sonnet", 3.00), ("opus", 15.00)];

/// Documented context window for the Claude aliases above.
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
    /// `None` for a model with no curated rate — an honest unknown.
    pub est_cost_usd: Option<f64>,
    /// Window fit for the model.
    pub window_fit: WindowFit,
}

/// The clearly labeled chars/4 input-token heuristic.
pub fn estimate_tokens(chars: usize) -> u64 {
    chars.div_ceil(4) as u64
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

/// Estimate one run's input cost; never fabricates a dollar cost for an
/// unknown model.
pub fn estimate(model: &str, chars: usize) -> Estimate {
    let tokens = estimate_tokens(chars);
    let fit = window_fit(model, tokens);
    let cost = PRICING
        .iter()
        .find(|(m, _)| *m == model)
        .map(|(_, rate)| (tokens as f64 * rate / 1_000_000.0 * 1e8).round() / 1e8);
    Estimate {
        est_input_tokens: tokens,
        est_cost_usd: cost,
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
    fn known_model_prices_exactly() {
        let e = estimate("haiku", 4_000_000);
        assert_eq!(e.est_input_tokens, 1_000_000);
        assert_eq!(e.est_cost_usd, Some(0.80));
        assert_eq!(e.window_fit.fits, Some(false));
        assert_eq!(e.window_fit.headroom_tokens, Some(200_000 - 1_000_000));
    }

    #[test]
    fn unknown_model_is_honest_none() {
        let e = estimate("mystery-model", 400);
        assert_eq!(e.est_input_tokens, 100);
        assert_eq!(e.est_cost_usd, None);
        assert_eq!(e.window_fit.fits, None);
        assert_eq!(e.window_fit.model_window, None);
    }

    #[test]
    fn fitting_input_has_positive_headroom() {
        let f = window_fit("sonnet", 10);
        assert_eq!(f.fits, Some(true));
        assert_eq!(f.headroom_tokens, Some(199_990));
    }
}
