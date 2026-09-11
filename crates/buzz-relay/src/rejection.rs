//! How a rejected client frame is addressed back to the client.
//!
//! NIP-01 gives every request type its own acknowledgement channel, and a
//! rejection is only actionable if it travels on the same one: a REQ or COUNT
//! refusal settles on `CLOSED`, an EVENT on `OK`. Rejecting an EVENT with a bare
//! `NOTICE` leaves a client that tracks pending publishes by event id with
//! nothing to key on, so the send cannot fail — it can only time out.

use crate::admission::AdmissionError;
use crate::connection::{AuthState, ConnectionState};
use crate::protocol::{ClientMessage, RelayMessage};
use crate::state::AppState;
use buzz_auth::LimitType;

/// What a rejected client frame is correlated back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectionTarget<'a> {
    /// A REQ or COUNT names the query it opened.
    Subscription(&'a str),
    /// An EVENT names the event it submitted.
    Event(nostr::EventId),
    /// No per-request correlation exists — connection-scoped notice.
    Connection,
}

/// Picks the acknowledgement channel a rejection of `msg` must travel on.
pub(crate) fn rejection_target_for(msg: &ClientMessage) -> RejectionTarget<'_> {
    match msg {
        ClientMessage::Req { sub_id, .. } | ClientMessage::Count { sub_id, .. } => {
            RejectionTarget::Subscription(sub_id.as_str())
        }
        ClientMessage::Event(event) => RejectionTarget::Event(event.id),
        _ => RejectionTarget::Connection,
    }
}

/// Renders `reason` as the rejection frame `target`'s acknowledgement channel
/// expects.
pub(crate) fn request_rejection_message(target: RejectionTarget<'_>, reason: &str) -> String {
    match target {
        RejectionTarget::Subscription(sub_id) => RelayMessage::closed(sub_id, reason),
        RejectionTarget::Event(event_id) => RelayMessage::ok(&event_id.to_hex(), false, reason),
        RejectionTarget::Connection => RelayMessage::notice(reason),
    }
}

/// Applies the WebSocket admission quotas to `msg`, returning whether it may be
/// handled. A rejection is addressed to the frame's own acknowledgement channel.
pub(crate) async fn enforce_ws_admission(
    msg: &ClientMessage,
    conn: &ConnectionState,
    state: &AppState,
) -> bool {
    let is_event = matches!(msg, ClientMessage::Event(_));
    if !is_event && !matches!(msg, ClientMessage::Req { .. } | ClientMessage::Count { .. }) {
        return true;
    }

    let (pubkey, is_agent) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Authenticated(ctx) => (ctx.pubkey, ctx.agent_owner_pubkey.is_some()),
            _ => return true,
        }
    };

    let limits = &state.auth.config().rate_limits;
    let (ws_window_secs, ws_limit) =
        crate::admission::ws_admission_budget(limits.human_ws_events_per_sec);
    let ws_result = crate::admission::check_principal(
        state.admission_rate_limiter.as_ref(),
        &conn.tenant,
        &pubkey,
        LimitType::WsEvents,
        ws_window_secs,
        ws_limit,
    )
    .await;
    if !send_admission_result(conn, ws_result, msg) {
        return false;
    }

    if is_event {
        let message_limit = if is_agent {
            limits.agent_standard_messages_per_min
        } else {
            limits.human_messages_per_min
        };
        let message_result = crate::admission::check_principal(
            state.admission_rate_limiter.as_ref(),
            &conn.tenant,
            &pubkey,
            LimitType::Messages,
            60,
            message_limit,
        )
        .await;
        // The per-minute message quota only applies to EVENTs, and its
        // rejection must be as correlatable as the burst quota's.
        if !send_admission_result(conn, message_result, msg) {
            return false;
        }
    }

    true
}

/// Forwards an admission verdict to the client, returning whether the frame was
/// admitted.
///
/// The rejection target is derived from `msg` here rather than supplied by the
/// caller: every quota check in this module must address its rejection to the
/// rejected frame's own acknowledgement channel, so there is deliberately no way
/// for a call site to name a different one.
fn send_admission_result(
    conn: &ConnectionState,
    result: Result<(), AdmissionError>,
    msg: &ClientMessage,
) -> bool {
    let target = rejection_target_for(msg);
    match result {
        Ok(()) => true,
        Err(AdmissionError::Exceeded { reset_in_secs }) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "websocket", "reason" => "quota").increment(1);
            conn.send(request_rejection_message(
                target,
                &format!("rate-limited: quota exceeded; retry in {reset_in_secs}s"),
            ));
            false
        }
        Err(AdmissionError::Unavailable) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "websocket", "reason" => "unavailable").increment(1);
            conn.send(request_rejection_message(
                target,
                "rate-limited: shared admission unavailable",
            ));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    //! A rejected frame must be answerable on the acknowledgement channel the
    //! client is actually waiting on.
    //!
    //! History: an over-quota EVENT used to be rejected with a bare
    //! `["NOTICE", reason]`. A NOTICE carries no event id, and desktop/mobile
    //! settle pending publishes only from an `OK` keyed by event id, so the
    //! rejection was unaddressable: the send could not fail, it could only time
    //! out (25s in Desktop, `PUBLISH_TIMEOUT_MS`) and surface as a message stuck
    //! on "Sending…". Startup quota exhaustion made it routine in the first
    //! seconds after launch.
    //!
    //! These tests drive the production rejection path — a real parsed
    //! `ClientMessage` through `enforce_ws_admission` and
    //! `send_admission_result` — and assert on the frame that reaches the
    //! connection's outbound channel.

    use std::sync::Arc;

    use axum::extract::ws::Message as WsMessage;
    use nostr::{EventBuilder, Keys, Kind};
    use tokio::sync::mpsc;

    use crate::connection::tests::{authenticated_state, read_frame, test_conn_with_auth};
    use crate::connection::AuthState;

    use super::*;

    fn sent_frame(rx: &mut mpsc::Receiver<WsMessage>) -> serde_json::Value {
        read_frame(rx)
    }

    fn test_conn() -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
        test_conn_with_auth(AuthState::Failed)
    }

    /// Parses a real EVENT frame exactly as the recv loop does, so the test is
    /// coupled to production parsing and not to a hand-built target.
    fn parsed_event_message() -> (ClientMessage, String) {
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&Keys::generate())
            .expect("sign event");
        let event_id = event.id.to_hex();
        let frame = serde_json::json!(["EVENT", event]).to_string();
        (ClientMessage::parse(&frame).expect("parse EVENT"), event_id)
    }

    /// The regression: an over-quota EVENT must be rejected with
    /// `OK(event_id, false, reason)` so the client can settle the exact pending
    /// publish it belongs to. A NOTICE here reintroduces the 25s send stall.
    #[test]
    fn over_quota_event_is_rejected_with_a_correlated_ok() {
        let (conn, mut rx) = test_conn();
        let (msg, event_id) = parsed_event_message();

        let admitted = send_admission_result(
            &conn,
            Err(AdmissionError::Exceeded { reset_in_secs: 7 }),
            &msg,
        );

        assert!(!admitted, "an over-quota frame is not admitted");
        let frame = sent_frame(&mut rx);
        assert_eq!(
            frame[0], "OK",
            "an EVENT rejection must travel on the OK channel — a NOTICE cannot \
             be correlated to a pending publish, so the send hangs until the \
             client's publish timeout instead of failing"
        );
        assert_eq!(
            frame[1], event_id,
            "the OK must name the rejected event id, which is what the client's \
             pending-publish map is keyed by"
        );
        assert_eq!(frame[2], false, "and must be an explicit rejection");
        assert_eq!(
            frame[3], "rate-limited: quota exceeded; retry in 7s",
            "the retry hint must survive so the client can arm its gate"
        );
    }

    /// The same correlation is required when admission is unavailable rather
    /// than exceeded — both branches strand a send if they emit a NOTICE.
    #[test]
    fn event_rejected_for_unavailable_admission_is_also_correlated() {
        let (conn, mut rx) = test_conn();
        let (msg, event_id) = parsed_event_message();

        send_admission_result(&conn, Err(AdmissionError::Unavailable), &msg);

        let frame = sent_frame(&mut rx);
        assert_eq!(frame[0], "OK");
        assert_eq!(frame[1], event_id);
        assert_eq!(frame[2], false);
    }

    /// A REQ still settles on CLOSED, which carries the subscription id. This
    /// pins the pre-existing behavior the fix must not disturb.
    #[test]
    fn over_quota_req_still_closes_the_subscription() {
        let (conn, mut rx) = test_conn();
        let raw = serde_json::json!(["REQ", "history-abc", {"kinds": [1]}]).to_string();
        let msg = ClientMessage::parse(&raw).expect("parse REQ");

        send_admission_result(
            &conn,
            Err(AdmissionError::Exceeded { reset_in_secs: 7 }),
            &msg,
        );

        let frame = sent_frame(&mut rx);
        assert_eq!(frame[0], "CLOSED");
        assert_eq!(
            frame[1], "history-abc",
            "a REQ rejection must name the subscription it rejected"
        );
        assert_eq!(frame[2], "rate-limited: quota exceeded; retry in 7s");
    }

    /// NIP-45 uses `CLOSED(query_id, reason)` when a relay refuses a COUNT.
    #[test]
    fn over_quota_count_closes_the_query() {
        let (conn, mut rx) = test_conn();
        let raw = serde_json::json!(["COUNT", "count-abc", {"kinds": [1]}]).to_string();
        let msg = ClientMessage::parse(&raw).expect("parse COUNT");

        send_admission_result(
            &conn,
            Err(AdmissionError::Exceeded { reset_in_secs: 7 }),
            &msg,
        );

        let frame = sent_frame(&mut rx);
        assert_eq!(frame[0], "CLOSED");
        assert_eq!(frame[1], "count-abc");
        assert_eq!(frame[2], "rate-limited: quota exceeded; retry in 7s");
    }

    /// Drives the real entry point `handle_text_message` calls, so the wiring
    /// between `enforce_ws_admission` and the target choice is under test and
    /// not just the leaf renderer.
    ///
    /// The state's Redis is deliberately unreachable, which makes admission
    /// return `Unavailable` — a production rejection path that needs no live
    /// quota burst to reach.
    async fn enforce_against_unreachable_admission(raw: &str) -> serde_json::Value {
        let state = crate::state::tests::test_state().await;
        let (conn, mut rx) = test_conn_with_auth(authenticated_state());
        let msg = ClientMessage::parse(raw).expect("parse client frame");

        let admitted = enforce_ws_admission(&msg, &conn, &state).await;
        assert!(!admitted, "an unadmitted frame must not be handled");
        sent_frame(&mut rx)
    }

    #[tokio::test]
    async fn enforce_ws_admission_rejects_an_event_on_the_ok_channel() {
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&Keys::generate())
            .expect("sign event");
        let event_id = event.id.to_hex();
        let raw = serde_json::json!(["EVENT", event]).to_string();

        let frame = enforce_against_unreachable_admission(&raw).await;

        assert_eq!(
            frame[0], "OK",
            "the admission gate must reject an EVENT on the channel the client's \
             pending publish is keyed by, or the send can only time out"
        );
        assert_eq!(frame[1], event_id);
        assert_eq!(frame[2], false);
    }

    #[tokio::test]
    async fn enforce_ws_admission_rejects_a_count_on_the_closed_channel() {
        let raw = serde_json::json!(["COUNT", "count-abc", {"kinds": [1]}]).to_string();

        let frame = enforce_against_unreachable_admission(&raw).await;

        assert_eq!(frame[0], "CLOSED");
        assert_eq!(frame[1], "count-abc");
    }

    #[tokio::test]
    async fn enforce_ws_admission_rejects_a_req_on_the_closed_channel() {
        let raw = serde_json::json!(["REQ", "history-abc", {"kinds": [1]}]).to_string();

        let frame = enforce_against_unreachable_admission(&raw).await;

        assert_eq!(frame[0], "CLOSED");
        assert_eq!(frame[1], "history-abc");
    }
}
