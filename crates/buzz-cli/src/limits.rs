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
