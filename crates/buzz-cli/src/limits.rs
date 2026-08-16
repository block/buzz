//! Shared `--limit` bookkeeping for the read commands.
//!
//! Every list-shaped read has a default limit and a hard cap, and a response
//! that comes back at that bound looks exactly like a complete one on stdout.
//! An agent rebuilding context from such a read reconstructs a prefix of the
//! truth and acts on it. These helpers resolve the bound that actually applied
//! and phrase the note the commands print to stderr.

/// What a command can actually do about a read that came back at its cap.
///
/// The recovery instruction has to name flags the command really exposes: an
/// agent told to retry with `--before` on a command that has no `--before`
/// burns a call on a parse error, and one told to "page through the rest" when
/// there is no older page believes the data is reachable when it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paging {
    /// Both `--since` and `--before`: the window can be moved either way.
    TimestampWindow,
    /// `--before` plus `--before-id`, the composite cursor `social notes`
    /// uses to walk backwards through a pubkey's notes.
    BeforeCursor,
    /// `--since` only. The window can be narrowed to *newer* results, which
    /// does not help when the missing ones are older.
    SinceOnly,
    /// No windowing flags at all — the cap is the end of the road.
    None,
}

/// A read command's `--limit` contract: its default, its hard cap, and how (or
/// whether) a caller can reach results beyond the cap.
#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    pub default: u32,
    pub max: u32,
    pub paging: Paging,
}

impl ReadLimits {
    /// Resolve a `--limit` against this command's default and cap.
    pub fn effective(&self, requested: Option<u32>) -> u32 {
        requested.unwrap_or(self.default).min(self.max)
    }
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
/// Below the cap the advice is always "ask for more"; at the cap it depends on
/// what the command exposes, which is what [`Paging`] carries.
///
/// Returns `None` for a short read, which is the only case that is provably
/// complete.
pub fn truncation_notice(
    returned: usize,
    requested: Option<u32>,
    limits: ReadLimits,
) -> Option<String> {
    let limit = limits.effective(requested);
    if returned < limit as usize {
        return None;
    }
    let max = limits.max;
    let bound = match requested {
        None => format!("the default limit of {}", limits.default),
        Some(r) if r > max => format!("--limit {r}, capped at {max}"),
        Some(r) => format!("--limit {r}"),
    };
    let advice = if limit < max {
        format!("pass a larger --limit (max {max})")
    } else {
        match limits.paging {
            Paging::TimestampWindow => {
                "narrow the window with --since / --before to page through the rest".to_string()
            }
            Paging::BeforeCursor => {
                "page backwards with --before / --before-id, taking both from the oldest note shown"
                    .to_string()
            }
            Paging::SinceOnly => format!(
                "--limit {max} is the hard cap, and --since only narrows to newer results — this command cannot request an older page"
            ),
            Paging::None => format!(
                "--limit {max} is the hard cap, and this command has no windowing flags — it cannot request a larger page"
            ),
        }
    };
    Some(format!(
        "showing {returned} results — {bound} was reached, so more may exist; {advice}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{truncation_notice, Paging, ReadLimits};

    const WINDOWED: ReadLimits = ReadLimits {
        default: 20,
        max: 50,
        paging: Paging::TimestampWindow,
    };

    #[test]
    fn effective_limit_applies_the_default_then_the_cap() {
        let limits = ReadLimits {
            default: 50,
            max: 200,
            paging: Paging::None,
        };
        assert_eq!(limits.effective(None), 50);
        assert_eq!(limits.effective(Some(10)), 10);
        assert_eq!(limits.effective(Some(1_000)), 200);
    }

    #[test]
    fn a_short_read_is_silent() {
        // The only provably complete case.
        assert_eq!(truncation_notice(19, None, WINDOWED), None);
        assert_eq!(truncation_notice(0, Some(10), WINDOWED), None);
    }

    #[test]
    fn a_full_default_read_names_the_default() {
        let notice = truncation_notice(20, None, WINDOWED).expect("full read must warn");
        assert!(notice.contains("the default limit of 20"), "{notice}");
        assert!(notice.contains("max 50"), "{notice}");
    }

    #[test]
    fn a_clamped_limit_says_it_was_clamped() {
        let notice = truncation_notice(50, Some(500), WINDOWED).expect("full read must warn");
        assert!(notice.contains("--limit 500, capped at 50"), "{notice}");
        // At the cap there is no larger limit to suggest.
        assert!(notice.contains("--since / --before"), "{notice}");
    }

    #[test]
    fn a_read_at_the_requested_limit_names_that_limit() {
        let notice = truncation_notice(30, Some(30), WINDOWED).expect("full read must warn");
        assert!(notice.contains("--limit 30"), "{notice}");
        assert!(!notice.contains("capped"), "{notice}");
    }

    #[test]
    fn an_overlong_read_still_warns() {
        // A relay that ignores the limit and returns more must not read as
        // complete just because the count is above the bound.
        assert!(truncation_notice(60, None, WINDOWED).is_some());
    }

    #[test]
    fn below_the_cap_every_command_is_told_to_ask_for_more() {
        // Whatever the command can do about paging is irrelevant while there
        // is still headroom under the cap.
        for paging in [
            Paging::TimestampWindow,
            Paging::BeforeCursor,
            Paging::SinceOnly,
            Paging::None,
        ] {
            let limits = ReadLimits {
                default: 20,
                max: 50,
                paging,
            };
            let notice = truncation_notice(20, None, limits).expect("full read must warn");
            assert!(
                notice.contains("pass a larger --limit (max 50)"),
                "{notice}"
            );
        }
    }

    #[test]
    fn at_the_cap_the_advice_names_only_flags_the_command_has() {
        let at_cap = |paging| {
            let limits = ReadLimits {
                default: 20,
                max: 50,
                paging,
            };
            truncation_notice(50, Some(50), limits).expect("full read must warn")
        };

        let window = at_cap(Paging::TimestampWindow);
        assert!(window.contains("--since / --before"), "{window}");

        let cursor = at_cap(Paging::BeforeCursor);
        assert!(cursor.contains("--before / --before-id"), "{cursor}");
        assert!(!cursor.contains("--since"), "{cursor}");

        // The two dead ends must not send a caller after a flag that does not
        // exist, or one that cannot reach older results.
        let since_only = at_cap(Paging::SinceOnly);
        assert!(
            since_only.contains("cannot request an older page"),
            "{since_only}"
        );
        assert!(!since_only.contains("--before"), "{since_only}");

        let none = at_cap(Paging::None);
        assert!(none.contains("cannot request a larger page"), "{none}");
        assert!(!none.contains("--since"), "{none}");
        assert!(!none.contains("--before"), "{none}");
    }
}
