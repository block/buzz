//! Pure acknowledgement state. Persistence and exact-event transport belong to clients.
use nostr::EventId;
use serde::{Deserialize, Serialize};

/// Durable application knowledge for one signed command at one target relay.
///
/// A client must persist the event and `Unknown` before sending it, so a crash
/// cannot make a potentially committed command look like a fresh attempt. Keep
/// the pre-attempt state in memory to classify that attempt's acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    /// The signed command is persisted but no delivery has been attempted.
    Prepared,
    /// All attempts are proven rejected before application.
    Rejected,
    /// At least one attempt may have committed; only a positive exact-ID OK resolves it.
    Unknown,
    /// The target relay positively acknowledged this exact command's application.
    Committed,
}

impl CommandOutcome {
    /// State to persist before transport starts, including an exact-event retry.
    ///
    /// Known commit is terminal: later rejection or lost delivery cannot undo it.
    pub fn before_send(self) -> Self {
        if self == Self::Committed {
            self
        } else {
            Self::Unknown
        }
    }

    /// Classify an OK received from the authenticated target relay.
    ///
    /// Call on the pre-attempt state, not the crash-safe state persisted by
    /// `before_send`. A mismatched ID or generic negative acknowledgement proves
    /// nothing. Readback of current labels is deliberately not an input.
    pub fn acknowledge(
        self,
        expected: EventId,
        received: EventId,
        accepted: bool,
        message: &str,
    ) -> Self {
        if self == Self::Committed {
            return self;
        }
        if expected != received {
            return Self::Unknown;
        }
        if accepted {
            return Self::Committed;
        }
        if self != Self::Unknown && is_rejected(message) {
            Self::Rejected
        } else {
            Self::Unknown
        }
    }
}

fn is_rejected(message: &str) -> bool {
    [
        "invalid: nip-cl-rejected",
        "auth-required: nip-cl-rejected",
        "restricted: nip-cl-rejected",
        "duplicate: nip-cl-rejected",
        "rate-limited: nip-cl-rejected",
        "error: nip-cl-rejected",
    ]
    .iter()
    .any(|prefix| {
        message == *prefix
            || message
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with(' '))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_knowledge_is_monotonic_across_loss_and_rejection() {
        let id = EventId::from_byte_array([1; 32]);
        let wrong_id = EventId::from_byte_array([2; 32]);
        for previous in [
            CommandOutcome::Prepared,
            CommandOutcome::Rejected,
            CommandOutcome::Unknown,
            CommandOutcome::Committed,
        ] {
            assert_eq!(
                previous.before_send(),
                if previous == CommandOutcome::Committed {
                    previous
                } else {
                    CommandOutcome::Unknown
                }
            );
            for message in ["", "duplicate: nip-cl-committed", "future positive detail"] {
                assert_eq!(
                    previous.acknowledge(id, id, true, message),
                    CommandOutcome::Committed
                );
                assert_eq!(
                    previous.acknowledge(id, wrong_id, true, message),
                    if previous == CommandOutcome::Committed {
                        previous
                    } else {
                        CommandOutcome::Unknown
                    }
                );
            }
            for category in [
                "invalid",
                "auth-required",
                "restricted",
                "duplicate",
                "rate-limited",
                "error",
            ] {
                for suffix in ["", " reason"] {
                    let message = format!("{category}: nip-cl-rejected{suffix}");
                    let expected = match previous {
                        CommandOutcome::Prepared | CommandOutcome::Rejected => {
                            CommandOutcome::Rejected
                        }
                        other => other,
                    };
                    assert_eq!(previous.acknowledge(id, id, false, &message), expected);
                }
            }
            for message in [
                "invalid: stale timestamp",
                "error: nip-cl-unknown",
                "restricted: nip-cl-rejected-extra",
                "INVALID: nip-cl-rejected",
                "invalid: nip-cl-rejected\nreason",
                "duplicate: nip-cl-committed",
                "",
            ] {
                assert_eq!(
                    previous.acknowledge(id, id, false, message),
                    if previous == CommandOutcome::Committed {
                        previous
                    } else {
                        CommandOutcome::Unknown
                    },
                    "{previous:?}: {message}"
                );
            }
        }
    }

    #[test]
    fn a_crashed_attempt_cannot_be_reclassified_as_a_rejected_new_command() {
        let id = EventId::from_byte_array([1; 32]);
        let persisted = serde_json::to_string(&CommandOutcome::Prepared.before_send()).unwrap();
        let recovered: CommandOutcome = serde_json::from_str(&persisted).unwrap();
        assert_eq!(
            recovered.acknowledge(id, id, false, "restricted: nip-cl-rejected expired"),
            CommandOutcome::Unknown
        );
        let committed = recovered.acknowledge(id, id, true, "duplicate: nip-cl-committed");
        assert_eq!(
            committed.acknowledge(id, id, false, "error: nip-cl-unknown"),
            CommandOutcome::Committed
        );
    }
}
