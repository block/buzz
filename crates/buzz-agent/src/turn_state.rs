//! The conversation and small amount of runtime metadata for one turn.
//!
//! Buzz persists conversation history on the relay. The Goose GDK loop and
//! provider APIs only need an in-memory [`Conversation`], so carrying Goose's
//! application-level `Session` (and its sqlite-oriented fields) here couples
//! the embedder to state it does not use.

use goose_provider_types::conversation::message::Message;
use goose_provider_types::conversation::Conversation;

/// Runtime metadata passed through Goose's generic state machine.
#[derive(Default)]
pub struct TurnSession {
    /// ACP/Goose attribution id for this turn.
    pub id: String,
    /// Working directory used by prompt discovery and tool hints.
    pub working_dir: std::path::PathBuf,
    /// Latest provider-reported occupancy for the current conversation.
    pub total_tokens: Option<i32>,
    conversation: Conversation,
}

impl goose_agent::machine::MachineSession for TurnSession {
    fn id(&self) -> &str {
        &self.id
    }

    fn conversation(&self) -> Option<&Conversation> {
        Some(&self.conversation)
    }
}

/// Bound the bytes a session's carried-forward history may occupy
/// (`BUZZ_AGENT_MAX_HISTORY_BYTES`).
///
/// goose's compaction frees *model context* by marking superseded messages
/// agent-invisible — it deletes nothing, so without this a long-lived
/// session's in-memory history grows without bound. Once the stored history
/// exceeds `max_bytes`, evict the **oldest agent-invisible** messages: they
/// were already summarized away, the model will never see them again, and
/// Buzz's durable record is the relay rather than this buffer. Messages the
/// model can still see are never evicted — their footprint is what
/// token-based compaction itself bounds — so this can leave the history over
/// budget when everything left is visible, and that is the correct outcome.
pub fn evict_hidden_history(messages: &mut Vec<Message>, max_bytes: usize) {
    let size = |message: &Message| {
        serde_json::to_string(message)
            .map(|serialized| serialized.len())
            .unwrap_or(0)
    };
    let mut total: usize = messages.iter().map(size).sum();
    if total <= max_bytes {
        return;
    }
    let original_len = messages.len();
    // Retain in order, dropping invisible messages front-first until under
    // budget. `retain` visits in order, so the oldest go first.
    messages.retain(|message| {
        if total <= max_bytes || message.is_agent_visible() {
            return true;
        }
        total = total.saturating_sub(size(message));
        false
    });
    if messages.len() < original_len {
        tracing::info!(
            evicted = original_len - messages.len(),
            remaining_bytes = total,
            "history over byte budget; evicted oldest compacted-away messages"
        );
    }
}

/// One turn's in-memory conversation and runtime metadata.
pub struct TurnState {
    session: TurnSession,
}

impl TurnState {
    pub fn new(id: String, working_dir: std::path::PathBuf) -> Self {
        Self {
            session: TurnSession {
                id,
                working_dir,
                total_tokens: None,
                conversation: Conversation::new_unvalidated(Vec::new()),
            },
        }
    }

    pub fn session(&self) -> &TurnSession {
        &self.session
    }

    pub fn conversation(&self) -> Conversation {
        self.session.conversation.clone()
    }

    pub fn push(&mut self, message: Message) {
        let mut messages = self.session.conversation.messages().to_vec();
        messages.push(message);
        self.session.conversation = Conversation::new_unvalidated(messages);
    }

    pub fn replace(&mut self, conversation: Conversation) {
        self.session.conversation = conversation;
    }

    pub fn set_total_tokens(&mut self, total: Option<i32>) {
        self.session.total_tokens = total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TurnState {
        TurnState::new("s1".to_string(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn a_new_turn_starts_empty() {
        assert!(state().conversation().messages().is_empty());
    }

    #[test]
    fn pushed_messages_are_visible() {
        let mut state = state();
        state.push(Message::user().with_text("hello"));
        state.push(Message::assistant().with_text("hi"));
        assert_eq!(state.conversation().messages().len(), 2);
    }

    fn hidden(text: &str) -> Message {
        let message = Message::user().with_text(text);
        let metadata = message.metadata.clone().with_agent_invisible();
        message.with_metadata(metadata)
    }

    #[test]
    fn eviction_is_a_noop_under_budget() {
        let mut messages = vec![hidden("old"), Message::user().with_text("new")];
        evict_hidden_history(&mut messages, usize::MAX);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn eviction_drops_oldest_hidden_messages_first() {
        let filler = "x".repeat(2000);
        let mut messages = vec![
            hidden(&format!("first {filler}")),
            hidden(&format!("second {filler}")),
            Message::user().with_text(format!("visible {filler}")),
        ];
        // Budget fits roughly one hidden message plus the visible one: the
        // oldest hidden message must go, the newer hidden one may stay.
        let budget = serde_json::to_string(&messages[1]).unwrap().len()
            + serde_json::to_string(&messages[2]).unwrap().len()
            + 64;
        evict_hidden_history(&mut messages, budget);
        let texts: Vec<String> = messages
            .iter()
            .map(|m| m.as_concat_text().chars().take(7).collect())
            .collect();
        assert!(
            !texts.iter().any(|t| t.starts_with("first")),
            "oldest hidden message must be evicted, got {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("visible")),
            "visible message must survive, got {texts:?}"
        );
    }

    #[test]
    fn eviction_never_touches_agent_visible_messages() {
        let filler = "y".repeat(4000);
        let mut messages = vec![
            Message::user().with_text(format!("one {filler}")),
            Message::assistant().with_text(format!("two {filler}")),
        ];
        // Over budget but everything is visible: nothing may be evicted.
        evict_hidden_history(&mut messages, 1024 * 1024);
        assert_eq!(messages.len(), 2);
        let mut big = vec![
            Message::user().with_text("z".repeat(2 * 1024 * 1024)),
            Message::assistant().with_text("w".repeat(2 * 1024 * 1024)),
        ];
        evict_hidden_history(&mut big, 1024 * 1024);
        assert_eq!(big.len(), 2, "visible messages must never be evicted");
    }

    #[test]
    fn replace_swaps_the_whole_conversation() {
        let mut state = state();
        state.push(Message::user().with_text("one"));
        state.replace(Conversation::new_unvalidated(vec![
            Message::user().with_text("summary")
        ]));
        assert_eq!(state.conversation().messages().len(), 1);
    }

    #[test]
    fn runtime_metadata_is_preserved() {
        let mut state = TurnState::new("abc".to_string(), std::path::PathBuf::from("/work"));
        state.set_total_tokens(Some(1234));
        assert_eq!(state.session().id, "abc");
        assert_eq!(
            state.session().working_dir,
            std::path::PathBuf::from("/work")
        );
        assert_eq!(state.session().total_tokens, Some(1234));
    }
}
