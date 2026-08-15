//! Shared `--limit` bookkeeping for the read commands.
//!
//! Every list-shaped read has a default limit and a hard cap, and a response
//! that comes back at that bound looks exactly like a complete one on stdout.
//! An agent rebuilding context from such a read reconstructs a prefix of the
//! truth and acts on it. These helpers resolve the bound that actually applied
//! and phrase the note the commands print to stderr.

/// Resolve a `--limit` against a command's default and cap.
pub fn effective_limit(requested: Option<u32>, default: u32, max: u32) -> u32 {
    requested.unwrap_or(default).min(max)
}

/// Build the stderr note for a read that came back full.
///
/// A read that returns exactly its limit is indistinguishable from a complete
/// one on stdout, which is how an agent rebuilding context from `messages get`
/// silently reconstructs a prefix of the conversation as if it were the whole
/// thing. There is no total to report — the relay answers a filter, not a
/// count — so the note states what bound was hit and how to raise it, and says
/// "may" because a result set exactly the size of the limit is also possible.
///
/// Returns `None` for a short read, which is the only case that is provably
/// complete.
pub fn truncation_notice(
    returned: usize,
    requested: Option<u32>,
    default: u32,
    max: u32,
) -> Option<String> {
    let limit = effective_limit(requested, default, max);
    if returned < limit as usize {
        return None;
    }
    let bound = match requested {
        None => format!("the default limit of {default}"),
        Some(r) if r > max => format!("--limit {r}, capped at {max}"),
        Some(r) => format!("--limit {r}"),
    };
    let advice = if limit < max {
        format!("pass a larger --limit (max {max})")
    } else {
        "narrow the window with --since / --before to page through the rest".to_string()
    };
    Some(format!(
        "showing {returned} results — {bound} was reached, so more may exist; {advice}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{effective_limit, truncation_notice};

    #[test]
    fn effective_limit_applies_the_default_then_the_cap() {
        assert_eq!(effective_limit(None, 50, 200), 50);
        assert_eq!(effective_limit(Some(10), 50, 200), 10);
        assert_eq!(effective_limit(Some(1_000), 50, 200), 200);
    }

    #[test]
    fn a_short_read_is_silent() {
        // The only provably complete case.
        assert_eq!(truncation_notice(19, None, 20, 50), None);
        assert_eq!(truncation_notice(0, Some(10), 20, 50), None);
    }

    #[test]
    fn a_full_default_read_names_the_default() {
        let notice = truncation_notice(20, None, 20, 50).expect("full read must warn");
        assert!(notice.contains("the default limit of 20"), "{notice}");
        assert!(notice.contains("max 50"), "{notice}");
    }

    #[test]
    fn a_clamped_limit_says_it_was_clamped() {
        let notice = truncation_notice(50, Some(500), 20, 50).expect("full read must warn");
        assert!(notice.contains("--limit 500, capped at 50"), "{notice}");
        // At the cap there is no larger limit to suggest.
        assert!(notice.contains("--since"), "{notice}");
    }

    #[test]
    fn a_read_at_the_requested_limit_names_that_limit() {
        let notice = truncation_notice(30, Some(30), 20, 50).expect("full read must warn");
        assert!(notice.contains("--limit 30"), "{notice}");
        assert!(!notice.contains("capped"), "{notice}");
    }

    #[test]
    fn an_overlong_read_still_warns() {
        // A relay that ignores the limit and returns more must not read as
        // complete just because the count is above the bound.
        assert!(truncation_notice(60, None, 20, 50).is_some());
    }
}
