//! Selector eligibility and shared directed-reason rules. Aggregate SQL applies
//! the same eligibility before grouping, covered by the PostgreSQL parity test.
use super::model::{Reason, ELIGIBLE_KINDS};

pub(super) fn eligible(
    kind: i32,
    own: bool,
    deleted: bool,
    created_ms: i64,
    cutoff_ms: i64,
) -> bool {
    ELIGIBLE_KINDS.contains(&kind) && !own && !deleted && created_ms >= cutoff_ms
}

/// Why a message is directed, before conversation membership is known.
pub(super) fn reason(channel_type: &str, actor_hex: &str, tags: &[Vec<String>]) -> Option<Reason> {
    let tagged = |name: &str, matches: &dyn Fn(&str) -> bool| {
        tags.iter()
            .any(|tag| tag.len() >= 2 && tag[0] == name && matches(&tag[1]))
    };
    if channel_type == "dm" {
        Some(Reason::Direct)
    } else if tagged("p", &|value| value.eq_ignore_ascii_case(actor_hex)) {
        Some(Reason::Mention)
    } else if tagged("broadcast", &|value| value == "1") {
        Some(Reason::Broadcast)
    } else {
        None
    }
}
