//! Detect turns that finished `Ok` without publishing a channel reply (#2459).
//!
//! Agents reply out-of-band via `buzz messages send`. When that write fails,
//! ACP still reports `end_turn` / `Ok` and the human sees silence. The harness
//! counts agent-authored stream messages observed on the wire during the turn
//! (self-events that `ignore_self` would otherwise drop) and posts a failure
//! notice when a mention-triggered turn ends with zero publishes.

use buzz_core::kind::{KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2};

use crate::pool::PromptOutcome;
use crate::queue::FlushBatch;

/// True when this batch was admitted as an `@mention` of a stream message —
/// the path that is expected to produce a visible channel reply.
pub(crate) fn batch_expects_channel_reply(batch: &FlushBatch) -> bool {
    batch.events.iter().any(|be| {
        be.prompt_tag == "@mention"
            && matches!(
                be.event.kind.as_u16() as u32,
                KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2
            )
    })
}

/// Kind filter used when counting self-authored publishes during a turn.
pub(crate) fn is_agent_reply_kind(kind: u32) -> bool {
    matches!(kind, KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2)
}

/// Return a user-visible notice when an Ok turn looks like a silently lost reply.
pub(crate) fn silent_reply_loss_notice(
    outcome: &PromptOutcome,
    batch: Option<&FlushBatch>,
    agent_message_publishes: u64,
) -> Option<String> {
    if !matches!(outcome, PromptOutcome::Ok(_)) {
        return None;
    }
    let batch = batch?;
    if !batch_expects_channel_reply(batch) {
        return None;
    }
    if agent_message_publishes > 0 {
        return None;
    }
    Some(
        "⚠️ I finished the turn but couldn't publish a reply to the channel. Please re-send if it's still needed."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::{AcpError, StopReason};
    use crate::queue::BatchEvent;
    use nostr::{EventBuilder, Keys, Kind};
    use std::time::Instant;
    use uuid::Uuid;

    fn mention_batch() -> FlushBatch {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "hi @agent")
            .tag(nostr::Tag::parse(["h", &Uuid::new_v4().to_string()]).unwrap())
            .sign_with_keys(&keys)
            .unwrap();
        FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![BatchEvent {
                event,
                prompt_tag: "@mention".into(),
                received_at: Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        }
    }

    fn all_mode_batch() -> FlushBatch {
        let mut batch = mention_batch();
        batch.events[0].prompt_tag = "all".into();
        batch
    }

    #[test]
    fn notice_fires_for_mention_ok_with_zero_publishes() {
        let batch = mention_batch();
        let notice =
            silent_reply_loss_notice(&PromptOutcome::Ok(StopReason::EndTurn), Some(&batch), 0);
        assert!(notice.unwrap().contains("couldn't publish"));
    }

    #[test]
    fn notice_skips_when_agent_published() {
        let batch = mention_batch();
        assert!(
            silent_reply_loss_notice(&PromptOutcome::Ok(StopReason::EndTurn), Some(&batch), 1,)
                .is_none()
        );
    }

    #[test]
    fn notice_skips_non_mention_batches() {
        let batch = all_mode_batch();
        assert!(
            silent_reply_loss_notice(&PromptOutcome::Ok(StopReason::EndTurn), Some(&batch), 0,)
                .is_none()
        );
    }

    #[test]
    fn notice_skips_errors_and_missing_batch() {
        assert!(silent_reply_loss_notice(
            &PromptOutcome::Error(AcpError::Protocol("x".into())),
            Some(&mention_batch()),
            0,
        )
        .is_none());
        assert!(
            silent_reply_loss_notice(&PromptOutcome::Ok(StopReason::EndTurn), None, 0,).is_none()
        );
    }

    #[test]
    fn reply_kind_filter_excludes_reactions() {
        assert!(is_agent_reply_kind(KIND_STREAM_MESSAGE));
        assert!(is_agent_reply_kind(KIND_STREAM_MESSAGE_V2));
        assert!(!is_agent_reply_kind(7));
    }
}
