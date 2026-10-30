//! NIP-TP community thread policy.
//!
//! The policy is a read-time presentation rule: stored events and
//! `thread_metadata` keep the parent each reply was sent to, and
//! [`ThreadPolicy::Flat`] only changes the derived thread structure a relay
//! returns. The methods below are the NIP-TP flattening rule, applied by every
//! derived thread read.

/// How a community presents replies inside a thread (NIP-TP).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThreadPolicy {
    /// Replies keep the parent they were sent to and render as a tree.
    #[default]
    Nested,
    /// Every reply is presented as a direct reply to its thread root.
    Flat,
}

impl ThreadPolicy {
    /// Parse the wire value (`"nested"` or `"flat"`). Any other value is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "nested" => Some(Self::Nested),
            "flat" => Some(Self::Flat),
            _ => None,
        }
    }

    /// The wire value, as stored in `communities.thread_policy` and served in
    /// the NIP-11 `thread_policy` field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nested => "nested",
            Self::Flat => "flat",
        }
    }

    /// The depth limit a thread read applies for a `requested` limit.
    ///
    /// Under `flat` every reply has effective depth 1, so any limit of 1 or
    /// more includes the whole thread (`None` = no stored-depth filter).
    pub const fn depth_limit(self, requested: Option<u32>) -> Option<u32> {
        match (self, requested) {
            (Self::Flat, Some(depth)) if depth >= 1 => None,
            _ => requested,
        }
    }

    /// The effective `(parent, depth)` of a thread row stored with `parent`,
    /// `root` and `depth`. Under `flat` a reply (depth > 0 with a known root)
    /// becomes a direct reply to its root.
    pub fn position<T>(self, parent: Option<T>, root: Option<T>, depth: i32) -> (Option<T>, i32) {
        match (self, root) {
            (Self::Flat, Some(root)) if depth > 0 => (Some(root), 1),
            _ => (parent, depth),
        }
    }

    /// The effective `(reply_count, descendant_count)` of a thread row at
    /// stored `depth`. Under `flat` a root's direct replies are all of its
    /// descendants, and a non-root reply has no replies of its own.
    pub const fn counts(self, depth: i32, reply_count: i32, descendant_count: i32) -> (i32, i32) {
        match self {
            Self::Nested => (reply_count, descendant_count),
            Self::Flat if depth == 0 => (descendant_count, descendant_count),
            Self::Flat => (0, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadPolicy::{self, Flat, Nested};

    #[test]
    fn parse_accepts_only_the_two_wire_values() {
        for policy in [Nested, Flat] {
            assert_eq!(ThreadPolicy::parse(policy.as_str()), Some(policy));
        }
        for invalid in ["", "Flat", "FLAT", " flat", "tree", "nested "] {
            assert_eq!(ThreadPolicy::parse(invalid), None, "{invalid:?}");
        }
        assert_eq!(ThreadPolicy::default(), Nested);
    }

    #[test]
    fn flattening_rule_table() {
        // Depth limits (rule 4): flat lifts any limit >= 1; 0 and absent stay.
        for (policy, requested, expected) in [
            (Nested, Some(1), Some(1)),
            (Nested, Some(3), Some(3)),
            (Nested, None, None),
            (Flat, Some(1), None),
            (Flat, Some(100), None),
            (Flat, Some(0), Some(0)),
            (Flat, None, None),
        ] {
            assert_eq!(
                policy.depth_limit(requested),
                expected,
                "{policy:?} {requested:?}"
            );
        }

        // Position (rule 1): flat re-parents replies to the root at depth 1.
        for (policy, parent, root, depth, expected) in [
            (Nested, Some("p"), Some("r"), 3, (Some("p"), 3)),
            (Flat, Some("p"), Some("r"), 3, (Some("r"), 1)),
            (Flat, Some("r"), Some("r"), 1, (Some("r"), 1)),
            // A root row (or one without a known root) is unchanged.
            (Flat, None, None, 0, (None, 0)),
            (Flat, Some("p"), None, 2, (Some("p"), 2)),
        ] {
            assert_eq!(policy.position(parent, root, depth), expected, "{policy:?}");
        }

        // Counts (rules 2 and 3).
        for (policy, depth, reply, descendant, expected) in [
            (Nested, 0, 2, 5, (2, 5)),
            (Nested, 1, 3, 0, (3, 0)),
            (Flat, 0, 2, 5, (5, 5)),
            (Flat, 1, 3, 0, (0, 0)),
            (Flat, 4, 1, 0, (0, 0)),
        ] {
            assert_eq!(
                policy.counts(depth, reply, descendant),
                expected,
                "{policy:?} {depth}"
            );
        }
    }
}
