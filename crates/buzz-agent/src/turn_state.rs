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
