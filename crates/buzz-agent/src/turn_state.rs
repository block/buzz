//! The conversation for one turn, held in memory.
//!
//! buzz-agent used to round-trip every message through goose's
//! `SessionManager` — a sqlite database — once per round. That was never
//! required: none of the goose APIs buzz calls reads the store.
//!
//! * `Provider::stream` takes `&[Message]`, not a session.
//! * `Agent::dispatch_tool_call` takes `&Session` but reads only
//!   `working_dir` (for the subdirectory-hint tracker) and `id` (for the
//!   `PreToolUse` hook context).
//! * `context_mgmt::{check_if_compaction_needed, compact_messages}` take a
//!   `&Session` / `session_id`, but the id is only carried into a tokio
//!   task-local that becomes an `agent-session-id` HTTP header for provider
//!   request attribution (`goose/src/session_context.rs`), and the session is
//!   read for `usage.total_tokens` and `model_config` — both plain fields.
//!
//! So the turn owns a [`Session`] value directly. goose still gets the
//! `&Session` its signatures ask for; it just isn't backed by a database.
//!
//! # Why this matters beyond dependency hygiene
//!
//! A `SessionManager` per agent process meant every buzz agent grew a sqlite
//! file it never read back. Worse, before the store was isolated they all
//! shared the user's own goose CLI history. An agent turn is ephemeral — the
//! durable record of a Buzz conversation is the relay, not a local database.

use goose::conversation::Conversation;
use goose::session::session_manager::Session;
use goose_provider_types::conversation::message::Message;

/// One turn's conversation and the session metadata goose needs alongside it.
pub struct TurnState {
    session: Session,
}

impl TurnState {
    /// Start a turn from the session's identity and working directory.
    ///
    /// `model_config` is carried because `check_if_compaction_needed` reads it
    /// from the session to resolve the context limit; without it goose falls
    /// back to a default limit and compaction fires at the wrong point.
    pub fn new(
        id: String,
        working_dir: std::path::PathBuf,
        model_config: Option<goose_providers::model::ModelConfig>,
    ) -> Self {
        let mut session = Session {
            id,
            working_dir,
            model_config,
            ..Session::default()
        };
        session.conversation = Some(Conversation::new_unvalidated(Vec::new()));
        Self { session }
    }

    /// The session value goose's APIs take.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// This turn's conversation.
    pub fn conversation(&self) -> Conversation {
        self.session.conversation.clone().unwrap_or_default()
    }

    /// Append a message to the turn.
    pub fn push(&mut self, message: Message) {
        let mut messages = self
            .session
            .conversation
            .take()
            .map(|conversation| conversation.messages().to_vec())
            .unwrap_or_default();
        messages.push(message);
        self.session.message_count = messages.len();
        self.session.conversation = Some(Conversation::new_unvalidated(messages));
    }

    /// Replace the conversation wholesale, as compaction does.
    pub fn replace(&mut self, conversation: Conversation) {
        self.session.message_count = conversation.messages().len();
        self.session.conversation = Some(conversation);
    }

    /// Record the running token total.
    ///
    /// `check_if_compaction_needed` prefers `session.usage.total_tokens` over
    /// re-counting the conversation, so keeping this current is what makes
    /// compaction fire on the provider's numbers rather than an estimate.
    pub fn set_total_tokens(&mut self, total: Option<i32>) {
        self.session.usage.total_tokens = total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TurnState {
        TurnState::new("s1".to_string(), std::path::PathBuf::from("/tmp"), None)
    }

    #[test]
    fn a_new_turn_starts_empty_but_has_a_conversation() {
        // `Some(empty)` rather than `None`: goose's compaction check bails on
        // a session with no conversation at all.
        let state = state();
        assert!(state.session().conversation.is_some());
        assert!(state.conversation().messages().is_empty());
    }

    #[test]
    fn pushed_messages_are_visible_to_goose() {
        let mut state = state();
        state.push(Message::user().with_text("hello"));
        state.push(Message::assistant().with_text("hi"));

        let conversation = state.conversation();
        assert_eq!(conversation.messages().len(), 2);
        // goose reads the conversation through the session, so the two views
        // must not drift.
        assert_eq!(
            state
                .session()
                .conversation
                .as_ref()
                .map(|c| c.messages().len()),
            Some(2)
        );
        assert_eq!(state.session().message_count, 2);
    }

    #[test]
    fn replace_swaps_the_whole_conversation() {
        let mut state = state();
        state.push(Message::user().with_text("one"));
        state.push(Message::user().with_text("two"));

        state.replace(Conversation::new_unvalidated(vec![
            Message::user().with_text("summary")
        ]));

        assert_eq!(state.conversation().messages().len(), 1);
        assert_eq!(state.session().message_count, 1);
    }

    #[test]
    fn total_tokens_reach_the_session_goose_reads() {
        let mut state = state();
        state.set_total_tokens(Some(1234));
        assert_eq!(state.session().usage.total_tokens, Some(1234));
    }

    #[test]
    fn the_working_dir_and_id_goose_reads_are_preserved() {
        // These two fields are the entirety of what `dispatch_tool_call` uses
        // the session for; getting them wrong breaks subdirectory hints and
        // the PreToolUse hook context.
        let state = TurnState::new("abc".to_string(), std::path::PathBuf::from("/work"), None);
        assert_eq!(state.session().id, "abc");
        assert_eq!(
            state.session().working_dir,
            std::path::PathBuf::from("/work")
        );
    }
}
