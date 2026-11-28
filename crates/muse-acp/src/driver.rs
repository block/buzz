//! The transport-neutral turn interface.
//!
//! [`Driver`] is what the ACP layer talks to. The production implementation
//! ([`crate::muse_web::MuseWebDriver`]) drives muse.ai through Chromium;
//! tests substitute [`FakeDriver`]. Keeping this trait narrow — prepare a
//! session, run one turn, stream text deltas — is what lets the ACP framing
//! be tested without a browser.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::BridgeError;

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// Muse's reply settled; the turn's text is fully streamed.
    Finished,
    /// `session/cancel` arrived mid-turn; the driver stopped the reply.
    Cancelled,
}

/// A backend that can carry one ACP session's conversation.
///
/// Sessions are independent: one Buzz channel session maps to one driver
/// session, and sessions never share in-progress turn state.
pub trait Driver: Send + Sync {
    /// Record the Buzz channel a session belongs to, when the ACP layer
    /// knows it (from the session title or a prompt's `Channel:` line).
    /// The default implementation ignores it; the Muse web driver uses
    /// it to route a tagged wake to that channel's Muse side chat.
    /// Calling it is always safe: a later turn's own prompt text can
    /// still override the stored channel.
    fn set_session_channel(&self, _session_id: &str, _channel: &str) {}

    /// Prepare a session for turns (launch the browser on first use,
    /// navigate, sign in if needed). Idempotent.
    fn ensure_session(
        &self,
        session_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), BridgeError>> + Send + '_>>;

    /// Run one turn: submit `message`, stream response text deltas into
    /// `chunks` in order, and return how the turn ended. Polls `cancel`
    /// between driver steps; when it is set, the driver stops the reply
    /// promptly and returns [`TurnOutcome::Cancelled`].
    ///
    /// A closed `chunks` receiver (the ACP layer went away) is also
    /// treated as cancellation.
    fn run_turn(
        &self,
        session_id: &str,
        message: &str,
        cancel: Arc<AtomicBool>,
        chunks: mpsc::Sender<String>,
    ) -> Pin<Box<dyn Future<Output = Result<TurnOutcome, BridgeError>> + Send + '_>>;
}

/// A scripted [`Driver`] for tests: emits its chunks with a small delay
/// between them, checking `cancel` at each step.
#[cfg(test)]
pub struct FakeDriver {
    chunks: Vec<String>,
    delay_ms: u64,
}

#[cfg(test)]
impl FakeDriver {
    pub fn new(chunks: Vec<String>, delay_ms: u64) -> Self {
        Self { chunks, delay_ms }
    }
}

#[cfg(test)]
impl Driver for FakeDriver {
    fn ensure_session(
        &self,
        _session_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), BridgeError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn run_turn(
        &self,
        _session_id: &str,
        _message: &str,
        cancel: Arc<AtomicBool>,
        chunks: mpsc::Sender<String>,
    ) -> Pin<Box<dyn Future<Output = Result<TurnOutcome, BridgeError>> + Send + '_>> {
        let scripted = self.chunks.clone();
        let delay_ms = self.delay_ms;
        Box::pin(async move {
            for chunk in scripted {
                if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    return Ok(TurnOutcome::Cancelled);
                }
                if chunks.send(chunk).await.is_err() {
                    return Ok(TurnOutcome::Cancelled);
                }
                if delay_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(TurnOutcome::Cancelled);
            }
            Ok(TurnOutcome::Finished)
        })
    }
}
