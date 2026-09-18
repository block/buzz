//! Coalescing policy for streaming an agent's reply into a channel.
//!
//! ACP delivers an answer as many `agent_message_chunk` updates. Today those
//! are logged and dropped, so a channel sees nothing until the agent posts a
//! finished message — the reply appears to arrive all at once, long after the
//! agent started talking.
//!
//! Relaying every chunk is not the fix: a token-rate edit stream would spend
//! the connection's event budget on typography. This module decides *when* an
//! accumulated buffer is worth publishing. It is pure — no clock, no I/O; the
//! caller supplies `now_ms` and performs the post/edit.
//!
//! Three properties the caller depends on:
//!
//! - **The first publish is a post, every later one is an edit.** Edits target
//!   the posted event id, so a stream is one message that grows, not a wall of
//!   fragments. Edits are excluded from unread triggers upstream, so this does
//!   not turn one answer into hundreds of notifications.
//! - **A finished turn always flushes**, even if the throttle would say wait —
//!   otherwise the last few tokens of every answer are silently dropped.
//! - **Nothing is published for an empty turn.** A turn that produced no text
//!   must not leave an empty message behind.

/// Hard ceiling on a single message body. `build_edit` rejects content over
/// 64 KiB, and a rejected edit would strand the message mid-answer, so the
/// policy truncates before the builder can refuse.
pub const MAX_STREAM_BYTES: usize = 64 * 1024;

/// Appended when the buffer is truncated, so a reader can tell a clipped
/// answer from a complete one rather than silently seeing less.
pub const TRUNCATION_MARKER: &str = "\n\n… (streamed reply truncated)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFlushConfig {
    /// Minimum gap between publishes. Zero disables throttling.
    pub throttle_ms: u64,
    /// Minimum newly-buffered characters before a mid-turn publish is worth it.
    pub min_delta_chars: usize,
    /// When false the policy never publishes — the pre-existing behaviour.
    pub enabled: bool,
}

impl Default for StreamFlushConfig {
    fn default() -> Self {
        // Off by default: streaming changes what every channel sees, so it is
        // opted into rather than inherited by existing deployments.
        Self {
            throttle_ms: 1_000,
            min_delta_chars: 24,
            enabled: false,
        }
    }
}

/// Publication state for one turn. `posted` flips once the first message exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamFlushState {
    /// Everything received this turn.
    pub buffer: String,
    /// Whether a message has been posted for this turn yet.
    pub posted: bool,
    /// Byte length of the buffer at the last publish.
    pub published_len: usize,
    /// `now_ms` of the last publish.
    pub last_publish_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFlushDecision {
    /// Accumulate; nothing to publish yet.
    Buffer,
    /// Create the turn's message with this body.
    Post { body: String },
    /// Replace the turn's message body.
    Edit { body: String },
}

/// Whether this call is a mid-turn chunk or the end of the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent<'a> {
    Chunk(&'a str),
    /// The turn ended — flush whatever is buffered.
    TurnEnd,
}

fn clamp_body(buffer: &str) -> String {
    if buffer.len() <= MAX_STREAM_BYTES {
        return buffer.to_string();
    }
    // Truncate on a char boundary: slicing mid-codepoint would panic.
    let budget = MAX_STREAM_BYTES - TRUNCATION_MARKER.len();
    let mut end = budget.min(buffer.len());
    while end > 0 && !buffer.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &buffer[..end], TRUNCATION_MARKER)
}

/// Fold one stream event into the state and decide whether to publish.
///
/// The state is advanced in place so the caller only records the outcome of a
/// publish it actually performed — a failed post must not leave the policy
/// believing a message exists to edit.
pub fn decide_stream_flush(
    state: &mut StreamFlushState,
    event: StreamEvent<'_>,
    now_ms: u64,
    config: StreamFlushConfig,
) -> StreamFlushDecision {
    if let StreamEvent::Chunk(text) = event {
        state.buffer.push_str(text);
    }

    if !config.enabled {
        return StreamFlushDecision::Buffer;
    }

    let trimmed_is_empty = state.buffer.trim().is_empty();
    let is_end = matches!(event, StreamEvent::TurnEnd);

    // An empty turn publishes nothing — and if it somehow posted earlier, an
    // end-of-turn edit to empty would blank a real message, so hold.
    if trimmed_is_empty {
        return StreamFlushDecision::Buffer;
    }

    let delta = state.buffer.len().saturating_sub(state.published_len);
    if !is_end {
        if delta < config.min_delta_chars {
            return StreamFlushDecision::Buffer;
        }
        let elapsed = now_ms.saturating_sub(state.last_publish_ms);
        // `posted == false` is the first publish: never delay the first sign of
        // life behind the throttle, or the channel stays silent exactly when a
        // reader most needs to know the agent is working.
        if state.posted && elapsed < config.throttle_ms {
            return StreamFlushDecision::Buffer;
        }
    } else if delta == 0 && state.posted {
        // Nothing new since the last publish and the message already reflects
        // it — no redundant final edit.
        return StreamFlushDecision::Buffer;
    }

    let body = clamp_body(&state.buffer);
    state.published_len = state.buffer.len();
    state.last_publish_ms = now_ms;

    if state.posted {
        StreamFlushDecision::Edit { body }
    } else {
        state.posted = true;
        StreamFlushDecision::Post { body }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> StreamFlushConfig {
        StreamFlushConfig {
            throttle_ms: 1_000,
            min_delta_chars: 10,
            enabled: true,
        }
    }

    #[test]
    fn disabled_never_publishes_but_still_accumulates() {
        let cfg = StreamFlushConfig {
            enabled: false,
            ..on()
        };
        let mut st = StreamFlushState::default();
        assert_eq!(
            decide_stream_flush(&mut st, StreamEvent::Chunk("hello world"), 0, cfg),
            StreamFlushDecision::Buffer
        );
        assert_eq!(
            decide_stream_flush(&mut st, StreamEvent::TurnEnd, 5_000, cfg),
            StreamFlushDecision::Buffer
        );
        assert_eq!(st.buffer, "hello world");
        assert!(!st.posted);
    }

    #[test]
    fn first_publish_is_a_post_and_ignores_the_throttle() {
        let mut st = StreamFlushState::default();
        // now_ms = 0 and last_publish_ms = 0: a throttle check would block this.
        let d = decide_stream_flush(&mut st, StreamEvent::Chunk("a first sentence"), 0, on());
        assert!(matches!(d, StreamFlushDecision::Post { .. }));
        assert!(st.posted);
    }

    #[test]
    fn later_publishes_are_edits() {
        let mut st = StreamFlushState::default();
        decide_stream_flush(&mut st, StreamEvent::Chunk("a first sentence"), 0, on());
        let d = decide_stream_flush(
            &mut st,
            StreamEvent::Chunk(" and a second one"),
            2_000,
            on(),
        );
        assert!(matches!(d, StreamFlushDecision::Edit { .. }));
    }

    #[test]
    fn small_deltas_buffer_until_they_are_worth_a_round_trip() {
        let mut st = StreamFlushState::default();
        decide_stream_flush(&mut st, StreamEvent::Chunk("a first sentence"), 0, on());
        assert_eq!(
            decide_stream_flush(&mut st, StreamEvent::Chunk("hi"), 9_000, on()),
            StreamFlushDecision::Buffer
        );
    }

    #[test]
    fn throttle_holds_edits_that_are_too_close_together() {
        let mut st = StreamFlushState::default();
        decide_stream_flush(&mut st, StreamEvent::Chunk("a first sentence"), 0, on());
        // Big enough delta, but only 100ms later.
        assert_eq!(
            decide_stream_flush(
                &mut st,
                StreamEvent::Chunk("plenty of new text here"),
                100,
                on()
            ),
            StreamFlushDecision::Buffer
        );
    }

    #[test]
    fn turn_end_flushes_through_the_throttle() {
        let mut st = StreamFlushState::default();
        decide_stream_flush(&mut st, StreamEvent::Chunk("a first sentence"), 0, on());
        decide_stream_flush(&mut st, StreamEvent::Chunk(" tail"), 10, on()); // buffered
        let d = decide_stream_flush(&mut st, StreamEvent::TurnEnd, 20, on());
        match d {
            StreamFlushDecision::Edit { body } => assert!(body.ends_with(" tail")),
            other => panic!("expected a final edit, got {other:?}"),
        }
    }

    // The whole point of streaming is that the reader sees the finished answer.
    // Dropping the tail because the throttle was mid-window would be worse than
    // not streaming at all.
    #[test]
    fn no_tokens_are_lost_between_the_last_edit_and_turn_end() {
        let mut st = StreamFlushState::default();
        decide_stream_flush(&mut st, StreamEvent::Chunk("opening statement"), 0, on());
        decide_stream_flush(
            &mut st,
            StreamEvent::Chunk(" middle part here"),
            5_000,
            on(),
        );
        decide_stream_flush(&mut st, StreamEvent::Chunk(" final words"), 5_050, on());
        let d = decide_stream_flush(&mut st, StreamEvent::TurnEnd, 5_060, on());
        match d {
            StreamFlushDecision::Edit { body } => {
                assert_eq!(body, "opening statement middle part here final words")
            }
            other => panic!("expected a final edit, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_turn_publishes_nothing() {
        let mut st = StreamFlushState::default();
        assert_eq!(
            decide_stream_flush(&mut st, StreamEvent::Chunk("   \n"), 0, on()),
            StreamFlushDecision::Buffer
        );
        assert_eq!(
            decide_stream_flush(&mut st, StreamEvent::TurnEnd, 1_000, on()),
            StreamFlushDecision::Buffer
        );
        assert!(!st.posted);
    }

    #[test]
    fn turn_end_does_not_re_edit_when_nothing_changed() {
        let mut st = StreamFlushState::default();
        decide_stream_flush(&mut st, StreamEvent::Chunk("a complete answer"), 0, on());
        assert_eq!(
            decide_stream_flush(&mut st, StreamEvent::TurnEnd, 9_000, on()),
            StreamFlushDecision::Buffer
        );
    }

    #[test]
    fn a_turn_that_only_ends_still_posts_what_it_buffered() {
        // Chunks arrived while disabled-by-throttle; the turn ends without any
        // publish having happened yet. It must still post once.
        let mut st = StreamFlushState::default();
        st.buffer.push_str("buffered without publishing");
        let d = decide_stream_flush(&mut st, StreamEvent::TurnEnd, 1, on());
        assert!(matches!(d, StreamFlushDecision::Post { .. }));
    }

    #[test]
    fn oversize_buffers_are_truncated_below_the_builder_limit() {
        let mut st = StreamFlushState::default();
        let huge = "x".repeat(MAX_STREAM_BYTES * 2);
        let d = decide_stream_flush(&mut st, StreamEvent::Chunk(&huge), 0, on());
        match d {
            StreamFlushDecision::Post { body } => {
                assert!(
                    body.len() <= MAX_STREAM_BYTES,
                    "body {} exceeds cap",
                    body.len()
                );
                assert!(body.ends_with(TRUNCATION_MARKER));
            }
            other => panic!("expected a post, got {other:?}"),
        }
    }

    // Truncation slices bytes; a multi-byte codepoint straddling the cap would
    // panic on a naive slice.
    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let mut st = StreamFlushState::default();
        let huge = "é".repeat(MAX_STREAM_BYTES); // 2 bytes each
        let d = decide_stream_flush(&mut st, StreamEvent::Chunk(&huge), 0, on());
        match d {
            StreamFlushDecision::Post { body } => assert!(body.len() <= MAX_STREAM_BYTES),
            other => panic!("expected a post, got {other:?}"),
        }
    }

    #[test]
    fn zero_throttle_publishes_on_every_sufficient_delta() {
        let cfg = StreamFlushConfig {
            throttle_ms: 0,
            min_delta_chars: 1,
            enabled: true,
        };
        let mut st = StreamFlushState::default();
        assert!(matches!(
            decide_stream_flush(&mut st, StreamEvent::Chunk("one"), 0, cfg),
            StreamFlushDecision::Post { .. }
        ));
        assert!(matches!(
            decide_stream_flush(&mut st, StreamEvent::Chunk("two"), 0, cfg),
            StreamFlushDecision::Edit { .. }
        ));
    }
}

/// Drive one turn's streamed reply.
///
/// Owns the buffer and publishes through `sink`, which is the seam a test
/// injects a fake at — the policy above is pure, but "post once then edit that
/// same event" is a stateful contract worth exercising end to end.
pub trait StreamPublisher {
    /// Create the turn's message; returns the event id later edits target.
    fn post(&mut self, body: String) -> impl std::future::Future<Output = Option<String>> + Send;
    /// Replace the body of a previously posted message.
    fn edit(
        &mut self,
        event_id: &str,
        body: String,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Consume chunks until the sender is dropped, publishing per the policy, then
/// flush whatever remains.
///
/// Dropping the sender IS the end-of-turn signal: every turn exit path in the
/// driver — success, idle timeout, agent exit, cancellation — drops it, so no
/// path can silently skip the final flush and strand a half-written answer.
pub async fn run_stream<P: StreamPublisher>(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    mut publisher: P,
    config: StreamFlushConfig,
    now_ms: impl Fn() -> u64 + Send,
) -> StreamFlushState {
    let mut state = StreamFlushState::default();
    let mut event_id: Option<String> = None;

    while let Some(chunk) = rx.recv().await {
        let decision =
            decide_stream_flush(&mut state, StreamEvent::Chunk(&chunk), now_ms(), config);
        apply(&mut publisher, &mut state, &mut event_id, decision).await;
    }

    let decision = decide_stream_flush(&mut state, StreamEvent::TurnEnd, now_ms(), config);
    apply(&mut publisher, &mut state, &mut event_id, decision).await;
    state
}

async fn apply<P: StreamPublisher>(
    publisher: &mut P,
    state: &mut StreamFlushState,
    event_id: &mut Option<String>,
    decision: StreamFlushDecision,
) {
    match decision {
        StreamFlushDecision::Buffer => {}
        StreamFlushDecision::Post { body } => match publisher.post(body).await {
            Some(id) => *event_id = Some(id),
            None => {
                // The post failed, so there is nothing to edit. Roll the state
                // back so the next decision retries a post rather than issuing
                // edits against an event that was never created.
                state.posted = false;
                state.published_len = 0;
            }
        },
        StreamFlushDecision::Edit { body } => {
            if let Some(id) = event_id.as_deref() {
                publisher.edit(id, body).await;
            }
        }
    }
}

#[cfg(test)]
mod run_stream_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct FakePublisher {
        posts: Arc<Mutex<Vec<String>>>,
        edits: Arc<Mutex<Vec<(String, String)>>>,
        fail_post: bool,
    }

    impl StreamPublisher for FakePublisher {
        async fn post(&mut self, body: String) -> Option<String> {
            if self.fail_post {
                return None;
            }
            self.posts.lock().unwrap().push(body);
            Some("evt-1".to_string())
        }
        async fn edit(&mut self, event_id: &str, body: String) {
            self.edits
                .lock()
                .unwrap()
                .push((event_id.to_string(), body));
        }
    }

    fn cfg() -> StreamFlushConfig {
        StreamFlushConfig {
            throttle_ms: 0,
            min_delta_chars: 1,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn posts_once_then_edits_the_same_event() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pubr = FakePublisher::default();
        let (posts, edits) = (pubr.posts.clone(), pubr.edits.clone());

        tx.send("first".to_string()).unwrap();
        tx.send(" second".to_string()).unwrap();
        drop(tx); // end of turn

        run_stream(rx, pubr, cfg(), || 0).await;

        assert_eq!(posts.lock().unwrap().len(), 1, "exactly one post per turn");
        let edits = edits.lock().unwrap();
        assert!(!edits.is_empty(), "later chunks must edit");
        assert!(
            edits.iter().all(|(id, _)| id == "evt-1"),
            "every edit targets the posted event"
        );
        assert_eq!(
            edits.last().unwrap().1,
            "first second",
            "final edit carries the whole reply"
        );
    }

    // Dropping the sender is the only end-of-turn signal, so a turn that ends
    // before the throttle would have fired must still publish.
    #[tokio::test]
    async fn a_dropped_sender_flushes_the_tail() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pubr = FakePublisher::default();
        let posts = pubr.posts.clone();

        tx.send("only chunk".to_string()).unwrap();
        drop(tx);

        let state = run_stream(
            rx,
            pubr,
            StreamFlushConfig {
                throttle_ms: 60_000,
                ..cfg()
            },
            || 0,
        )
        .await;
        assert_eq!(
            posts.lock().unwrap().as_slice(),
            &["only chunk".to_string()]
        );
        assert!(state.posted);
    }

    #[tokio::test]
    async fn an_empty_turn_publishes_nothing() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pubr = FakePublisher::default();
        let (posts, edits) = (pubr.posts.clone(), pubr.edits.clone());
        drop(tx);

        run_stream(rx, pubr, cfg(), || 0).await;
        assert!(posts.lock().unwrap().is_empty());
        assert!(edits.lock().unwrap().is_empty());
    }

    // A failed post must not leave the policy believing a message exists — the
    // edits that followed would target nothing and the reply would vanish.
    #[tokio::test]
    async fn a_failed_post_never_produces_orphan_edits() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pubr = FakePublisher {
            fail_post: true,
            ..Default::default()
        };
        let edits = pubr.edits.clone();

        tx.send("first".to_string()).unwrap();
        tx.send(" second".to_string()).unwrap();
        drop(tx);

        run_stream(rx, pubr, cfg(), || 0).await;
        assert!(
            edits.lock().unwrap().is_empty(),
            "no edits without a posted event"
        );
    }

    #[tokio::test]
    async fn disabled_streaming_publishes_nothing_at_all() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let pubr = FakePublisher::default();
        let (posts, edits) = (pubr.posts.clone(), pubr.edits.clone());

        tx.send("text that would otherwise publish".to_string())
            .unwrap();
        drop(tx);

        run_stream(
            rx,
            pubr,
            StreamFlushConfig {
                enabled: false,
                ..cfg()
            },
            || 0,
        )
        .await;
        assert!(posts.lock().unwrap().is_empty());
        assert!(edits.lock().unwrap().is_empty());
    }
}
