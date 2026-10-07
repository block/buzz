//! Directed-reason rules for explicit selectors. The sidebar's SQL applies the
//! same rules, covered by the PostgreSQL parity test.
use super::model::Reason;

/// Why a message is directed, before conversation membership is known.
/// `broadcast_reply` is a depth-1 reply broadcast to the channel timeline.
pub(super) fn reason(
    channel_type: &str,
    actor_hex: &str,
    tags: &[Vec<String>],
    broadcast_reply: bool,
) -> Option<Reason> {
    let tagged = |name: &str, matches: &dyn Fn(&str) -> bool| {
        tags.iter()
            .any(|tag| tag.len() >= 2 && tag[0] == name && matches(&tag[1]))
    };
    if channel_type == "dm" {
        Some(Reason::Direct)
    } else if tagged("p", &|value| value.eq_ignore_ascii_case(actor_hex)) {
        Some(Reason::Mention)
    } else if broadcast_reply {
        Some(Reason::Broadcast)
    } else {
        None
    }
}
