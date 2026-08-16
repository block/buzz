//! Twilio outbound SMS sink.
//!
//! Mirrors [`crate::api::sms`]'s tag conventions in reverse: when a reply
//! event lands in the configured SMS-inbox channel and its `e` tag resolves
//! to an event carrying an `sms_from` tag (an inbound SMS this relay itself
//! synthesized), the reply's content is sent back to that phone number via
//! Twilio's REST API.
//!
//! Called from the post-persist side of the event pipeline
//! ([`crate::handlers::event::dispatch_persistent_event_inner`]) — same seam
//! [`crate::workflow_sink`] hooks into for its own post-persist side effects.
//! Fail-open throughout: every "can't tell" or "send failed" case is logged
//! and returned as [`SendOutcome`], never propagated as an error the caller
//! must handle. A Twilio outage or misconfiguration must never disrupt the
//! relay's own event-dispatch pipeline.

use std::sync::Arc;

use buzz_core::event::StoredEvent;
use buzz_core::kind::KIND_STREAM_MESSAGE;
use buzz_core::tenant::TenantContext;
use tracing::{info, warn};

use crate::state::AppState;

/// Outcome of considering `stored_event` for outbound send. Not an error
/// type — every variant is a normal, expected result of this fail-open sink.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SendOutcome {
    /// Not a reply in the SMS-inbox channel resolving to an inbound-SMS
    /// source event — nothing to send. The overwhelmingly common case, since
    /// this runs for every persisted event relay-wide.
    NotApplicable,
    /// Sent successfully.
    Sent,
    /// Resolved a real outbound target but the send itself failed (Twilio
    /// account misconfigured, Twilio rejected the request, or the request
    /// itself failed).
    Failed(String),
}

fn tag_value<'a>(event: &'a nostr::Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|t| t.as_slice().first().map(String::as_str) == Some(name))
        .and_then(|t| t.as_slice().get(1))
        .map(String::as_str)
}

/// The first `e`-tagged event id, in tag order — matches `buzz-sdk`'s
/// `reply_to_event_id`, which emits a single `["e", <id>, "", "reply"]` tag
/// for a top-level reply. That is the shape this sink is built for: the
/// sms-operator persona replies directly to the inbound event, so the first
/// `e` tag *is* the inbound event.
///
/// Deliberately shallow — it resolves one hop only, and does not walk a
/// thread. A reply nested deeper than one level below the inbound SMS will
/// not resolve to a phone number and simply won't be sent. Widening this to
/// walk ancestors would need care: it would let any reply anywhere in a
/// thread rooted at an inbound SMS trigger an outbound text.
fn first_e_tag_event_id(event: &nostr::Event) -> Option<&str> {
    tag_value(event, "e")
}

/// Base URL for Twilio's Messages API — `Some` only when explicitly
/// overridden (tests point this at a local mock server); `None` means "use
/// the real API," resolved by the caller rather than baked in here so this
/// function stays a pure string builder.
fn twilio_messages_url(base_url: &str, account_sid: &str) -> String {
    format!("{base_url}/2010-04-01/Accounts/{account_sid}/Messages.json")
}

/// Whether an event is even worth a database lookup, decided from the event
/// alone. Split out from [`maybe_send_outbound_sms`] so the cases that matter
/// are testable without an `AppState` or a live Postgres.
///
/// The kind check is load-bearing, not defensive tidiness. Every other
/// condition here is satisfied by the agent harness's own bookkeeping events:
/// it reacts to a message it is picking up (kind:7 `👀`) and again while it
/// works (kind:7 `💬`), and those reactions carry an `e` tag pointing at the
/// inbound SMS. Without a kind filter each one resolved to the sender's phone
/// number and went out as a text, so a single inbound message produced two
/// emoji-only SMS before the real reply. The harness deletes those reactions
/// a moment later (kind:5), but a delivered SMS cannot be recalled.
fn is_outbound_candidate(
    event: &nostr::Event,
    channel_id: Option<uuid::Uuid>,
    inbox_channel: uuid::Uuid,
) -> bool {
    if channel_id != Some(inbox_channel) {
        return false;
    }
    if event.kind.as_u16() as u32 != KIND_STREAM_MESSAGE {
        return false;
    }
    // The inbound event itself already carries sms_from — it is not a reply
    // to anything, and must never be echoed back to Twilio as if it were.
    tag_value(event, "sms_from").is_none()
}

/// If `stored_event` is a reply inside the configured SMS-inbox channel
/// whose `e` tag resolves to an inbound-SMS event, send its content back to
/// that sender via Twilio.
pub(crate) async fn maybe_send_outbound_sms(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    stored_event: &StoredEvent,
) -> SendOutcome {
    let Some(inbox_channel) = state.config.twilio_sms_inbox_channel else {
        return SendOutcome::NotApplicable;
    };
    if !is_outbound_candidate(&stored_event.event, stored_event.channel_id, inbox_channel) {
        return SendOutcome::NotApplicable;
    }

    let Some(target_id) = first_e_tag_event_id(&stored_event.event) else {
        return SendOutcome::NotApplicable;
    };
    let Ok(target_id_bytes) = nostr::EventId::from_hex(target_id).map(|id| id.as_bytes().to_vec())
    else {
        return SendOutcome::NotApplicable;
    };

    let source_event = match state
        .db
        .get_event_by_id(tenant.community(), &target_id_bytes)
        .await
    {
        Ok(Some(e)) => e,
        Ok(None) => return SendOutcome::NotApplicable,
        Err(e) => {
            warn!(error = %e, "sms_sink: failed to look up e-tagged source event");
            return SendOutcome::NotApplicable;
        }
    };
    let Some(phone_number) = tag_value(&source_event.event, "sms_from") else {
        return SendOutcome::NotApplicable;
    };

    let (Some(account_sid), Some(auth_token), Some(from_number)) = (
        state.config.twilio_account_sid.as_deref(),
        state.config.twilio_auth_token.as_deref(),
        state.config.twilio_from_number.as_deref(),
    ) else {
        warn!(
            "sms_sink: reply resolved to an outbound SMS target but Twilio account config \
             is incomplete (need TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN, TWILIO_FROM_NUMBER) \
             — dropping the send"
        );
        return SendOutcome::Failed("twilio account not configured".into());
    };
    let base_url = state
        .config
        .twilio_api_base_url
        .as_deref()
        .unwrap_or("https://api.twilio.com");

    send_via_twilio(
        base_url,
        account_sid,
        auth_token,
        from_number,
        phone_number,
        &stored_event.event.content,
    )
    .await
}

async fn send_via_twilio(
    base_url: &str,
    account_sid: &str,
    auth_token: &str,
    from_number: &str,
    to_number: &str,
    body: &str,
) -> SendOutcome {
    let url = twilio_messages_url(base_url, account_sid);
    // Built by hand rather than via reqwest's `.form()`: the workspace pins
    // reqwest with default features off, so the `urlencoded` feature that
    // provides `.form()` isn't compiled in.
    let form_body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("To", to_number)
        .append_pair("From", from_number)
        .append_pair("Body", body)
        .finish();
    let client = reqwest::Client::new();
    let result = client
        .post(&url)
        .basic_auth(account_sid, Some(auth_token))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            info!(to = %to_number, "sms_sink: sent outbound SMS reply");
            SendOutcome::Sent
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(to = %to_number, %status, %body, "sms_sink: Twilio rejected the outbound send");
            SendOutcome::Failed(format!("twilio returned {status}"))
        }
        Err(e) => {
            warn!(to = %to_number, error = %e, "sms_sink: outbound SMS request failed");
            SendOutcome::Failed(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn event_with_tags(tags: Vec<Vec<&str>>, content: &str) -> nostr::Event {
        event_of_kind(9, tags, content)
    }

    fn event_of_kind(kind: u16, tags: Vec<Vec<&str>>, content: &str) -> nostr::Event {
        let keys = nostr::Keys::generate();
        let tags: Vec<nostr::Tag> = tags
            .into_iter()
            .map(|t| nostr::Tag::parse(t).expect("valid tag"))
            .collect();
        nostr::EventBuilder::new(nostr::Kind::from(kind), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign test event")
    }

    // ── is_outbound_candidate ───────────────────────────────────────────────

    /// A reply the sms-operator persona posts: kind:9, in the inbox channel,
    /// threading back to the inbound SMS. The one shape that should send.
    #[test]
    fn outbound_candidate_accepts_a_message_reply_in_the_inbox_channel() {
        let inbox = uuid::Uuid::new_v4();
        let event = event_with_tags(vec![vec!["e", "abc123", "", "reply"]], "Which project?");
        assert!(is_outbound_candidate(&event, Some(inbox), inbox));
    }

    /// Regression: the agent harness reacts to an inbound message with 👀 and
    /// 💬 (kind:7), each carrying an `e` tag back to it. Those satisfied every
    /// other condition and were texted to the sender as two junk messages
    /// ahead of the real reply. See the doc comment on is_outbound_candidate.
    #[test]
    fn outbound_candidate_rejects_agent_reactions() {
        let inbox = uuid::Uuid::new_v4();
        for emoji in ["👀", "💬", "+"] {
            let reaction = event_of_kind(7, vec![vec!["e", "abc123"]], emoji);
            assert!(
                !is_outbound_candidate(&reaction, Some(inbox), inbox),
                "kind:7 reaction {emoji} must never be sent as an SMS"
            );
        }
    }

    /// The harness deletes its own reactions once it has replied. A deletion
    /// also carries `e` tags, so it must be filtered on kind too.
    #[test]
    fn outbound_candidate_rejects_deletions() {
        let inbox = uuid::Uuid::new_v4();
        let deletion = event_of_kind(5, vec![vec!["e", "abc123"]], "");
        assert!(!is_outbound_candidate(&deletion, Some(inbox), inbox));
    }

    #[test]
    fn outbound_candidate_rejects_the_inbound_event_itself() {
        let inbox = uuid::Uuid::new_v4();
        // Synthesized inbound SMS: kind:9 in the inbox channel, but tagged
        // sms_from. Echoing it would text the sender their own message back.
        let inbound = event_with_tags(
            vec![vec!["sms_from", "+15551234567"], vec!["e", "abc123"]],
            "Hi",
        );
        assert!(!is_outbound_candidate(&inbound, Some(inbox), inbox));
    }

    #[test]
    fn outbound_candidate_rejects_events_in_other_channels() {
        let inbox = uuid::Uuid::new_v4();
        let elsewhere = uuid::Uuid::new_v4();
        let event = event_with_tags(vec![vec!["e", "abc123", "", "reply"]], "unrelated reply");
        assert!(!is_outbound_candidate(&event, Some(elsewhere), inbox));
        assert!(!is_outbound_candidate(&event, None, inbox));
    }

    // ── tag_value / first_e_tag_event_id ────────────────────────────────────

    #[test]
    fn tag_value_finds_named_tag() {
        let event = event_with_tags(vec![vec!["sms_from", "+15551234567"]], "hi");
        assert_eq!(tag_value(&event, "sms_from"), Some("+15551234567"));
    }

    #[test]
    fn tag_value_none_when_absent() {
        let event = event_with_tags(vec![], "hi");
        assert_eq!(tag_value(&event, "sms_from"), None);
    }

    #[test]
    fn first_e_tag_event_id_reads_reply_marker_tag() {
        // Matches buzz-sdk's reply_to_event_id shape: ["e", <id>, "", "reply"].
        let event = event_with_tags(vec![vec!["e", "abc123", "", "reply"]], "reply text");
        assert_eq!(first_e_tag_event_id(&event), Some("abc123"));
    }

    #[test]
    fn first_e_tag_event_id_none_without_e_tag() {
        let event = event_with_tags(vec![vec!["h", "some-channel"]], "no reply");
        assert_eq!(first_e_tag_event_id(&event), None);
    }

    // ── twilio_messages_url ──────────────────────────────────────────────────

    #[test]
    fn twilio_messages_url_includes_account_sid() {
        assert_eq!(
            twilio_messages_url("https://api.twilio.com", "ACxxxx"),
            "https://api.twilio.com/2010-04-01/Accounts/ACxxxx/Messages.json"
        );
    }

    // ── send_via_twilio (real HTTP against a local mock listener) ───────────

    /// Minimal single-shot HTTP mock: accepts one connection, reads the
    /// request until the blank line ending headers plus any `Content-Length`
    /// body, replies with `response`, and yields the raw request text.
    ///
    /// Returns the base URL immediately plus a handle the caller awaits
    /// *after* issuing the request — awaiting it here would deadlock, since
    /// the server is waiting for a connection the caller can't make yet.
    async fn mock_http_once(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];

            // Read until the header/body boundary is seen, then read exactly
            // as many more bytes as Content-Length declares — a plain
            // `read()` can return before the whole request has arrived.
            let header_end = loop {
                let n = socket.read(&mut chunk).await.expect("read headers");
                assert_ne!(n, 0, "connection closed before headers completed");
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    break pos;
                }
            };
            let content_length: usize = String::from_utf8_lossy(&buf[..header_end])
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            while buf.len() - body_start < content_length {
                let n = socket.read(&mut chunk).await.expect("read body");
                assert_ne!(n, 0, "connection closed before body completed");
                buf.extend_from_slice(&chunk[..n]);
            }

            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            String::from_utf8_lossy(&buf).into_owned()
        });
        (format!("http://{addr}"), handle)
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    #[tokio::test]
    async fn send_via_twilio_posts_form_body_and_reports_sent_on_2xx() {
        let (base_url, server) = mock_http_once(
            "HTTP/1.1 201 Created\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .await;

        let outcome = send_via_twilio(
            &base_url,
            "ACtest",
            "authtoken",
            "+15550001111",
            "+15559998888",
            "hello from buzz",
        )
        .await;
        let request_text = server.await.expect("mock task");

        assert_eq!(outcome, SendOutcome::Sent);
        assert!(request_text.starts_with("POST /2010-04-01/Accounts/ACtest/Messages.json"));
        // hyper emits header names lowercased on the wire.
        let lowered = request_text.to_lowercase();
        assert!(
            lowered.contains("authorization: basic"),
            "expected basic auth header, got: {request_text}"
        );
        assert!(lowered.contains("content-type: application/x-www-form-urlencoded"));
        assert!(request_text.contains("To=%2B15559998888"));
        assert!(request_text.contains("From=%2B15550001111"));
        assert!(request_text.contains("Body=hello+from+buzz"));
    }

    #[tokio::test]
    async fn send_via_twilio_reports_failed_on_non_2xx() {
        let (base_url, server) = mock_http_once(
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{\"code\":21211}",
        )
        .await;

        let outcome = send_via_twilio(
            &base_url,
            "ACtest",
            "authtoken",
            "+15550001111",
            "+15559998888",
            "hello",
        )
        .await;
        let _ = server.await;

        assert!(matches!(outcome, SendOutcome::Failed(_)));
    }

    #[tokio::test]
    async fn send_via_twilio_reports_failed_when_unreachable() {
        // Port 1 is reserved/unassigned — connection should fail immediately
        // rather than hang, without needing a real timeout.
        let outcome = send_via_twilio(
            "http://127.0.0.1:1",
            "ACtest",
            "authtoken",
            "+15550001111",
            "+15559998888",
            "hello",
        )
        .await;

        assert!(matches!(outcome, SendOutcome::Failed(_)));
    }
}
