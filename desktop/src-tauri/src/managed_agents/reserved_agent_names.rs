//! Community bot display names that local managed agents / personas must not
//! reuse. Matching is case-insensitive on the trimmed exact name so "Mo Local"
//! stays allowed while "mo" / "MO" are blocked.

/// Exact display names reserved for Hula community bots.
pub(crate) const RESERVED_AGENT_NAMES: &[&str] = &["Captain", "Mo", "Stitch", "Quasar", "Korg"];

/// Returns `true` when `name` (after trim) exactly matches a reserved
/// community-bot name, ignoring ASCII case.
pub(crate) fn is_reserved_agent_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    RESERVED_AGENT_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(trimmed))
}

/// Error when a local agent or persona tries to use a reserved community name.
pub(crate) fn reserved_agent_name_error(name: &str) -> String {
    format!(
        "\"{}\" is reserved for a Hula community bot. Pick a different local agent name (for example \"{} Local\").",
        name.trim(),
        name.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_exact_and_case_variants() {
        for name in ["Mo", "mo", "MO", " Captain ", "korg", "Quasar", "Stitch"] {
            assert!(is_reserved_agent_name(name), "{name}");
        }
    }

    #[test]
    fn allows_qualified_and_unrelated_names() {
        for name in ["Mo Local", "Captain Hook", "Mosaic", "Ada", ""] {
            assert!(!is_reserved_agent_name(name), "{name}");
        }
    }
}
