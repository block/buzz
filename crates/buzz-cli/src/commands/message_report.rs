//! `buzz messages report` — NIP-56 report (kind 1984) to community moderators.
//!
//! The relay queues a report for moderators only: it is never stored as an
//! ordinary event or fanned out, so the command prints the write envelope and
//! nothing else. A message report carries the author's `p` tag and an `e` tag
//! whose third element is the report type, exactly as desktop sends it.

use buzz_sdk::ReportTarget;

use super::messages::{channel_id_from_event, fetch_event};
use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Parsed `buzz messages report` arguments.
pub struct ReportParams<'a> {
    /// Channel UUID the message must belong to, when given.
    pub channel: Option<&'a str>,
    /// Hex id of the message to report.
    pub event: Option<&'a str>,
    /// Hex pubkey of a user to report without a message.
    pub pubkey: Option<&'a str>,
    /// NIP-56 report type.
    pub reason: &'a str,
    /// Optional free-text note for moderators.
    pub note: Option<&'a str>,
}

/// The relay rejects a report whose target it cannot read exactly like a
/// missing one, so both surface as not-found rather than a relay failure.
fn interpret_submit_error(e: CliError) -> CliError {
    match e {
        CliError::Relay { status: 400, body } if body.contains("report target") => {
            CliError::NotFound(body)
        }
        other => other,
    }
}

/// Build, sign, and submit a report; print the write envelope.
pub async fn cmd_report_message(
    client: &BuzzClient,
    params: ReportParams<'_>,
) -> Result<(), CliError> {
    let builder = match (params.event, params.pubkey) {
        (Some(event_id), None) => {
            validate_hex64(event_id)?;
            let event = fetch_event(client, event_id).await?;
            if let Some(channel) = params.channel {
                let actual = channel_id_from_event(event_id, &event)?;
                if !actual.to_string().eq_ignore_ascii_case(channel.trim()) {
                    return Err(CliError::Usage(format!(
                        "event {event_id} is not in channel {channel}"
                    )));
                }
            }
            let author = event
                .get("pubkey")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CliError::Other(format!("event {event_id} has no pubkey")))?;
            buzz_sdk::build_report(
                ReportTarget::Event {
                    author_pubkey: author,
                    event_id,
                },
                params.reason,
                params.note,
            )
        }
        (None, Some(pubkey)) => {
            buzz_sdk::build_report(ReportTarget::Pubkey(pubkey), params.reason, params.note)
        }
        _ => {
            return Err(CliError::Usage(
                "report needs exactly one of --event or --pubkey".into(),
            ))
        }
    }
    .map_err(|e| CliError::Usage(format!("invalid report: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client
        .submit_event(event)
        .await
        .map_err(interpret_submit_error)?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{extract::State, http::StatusCode, routing::post, Router};
    use nostr::Keys;
    use tokio::net::TcpListener;

    use super::*;
    use crate::error::exit_code;
    use crate::{Cli, Cmd, MessagesCmd};
    use clap::Parser;

    const CHANNEL: &str = "123e4567-e89b-12d3-a456-426614174000";
    const EVENT: &str = "cd00000000000000000000000000000000000000000000000000000000000000";
    const AUTHOR: &str = "ab00000000000000000000000000000000000000000000000000000000000000";

    #[derive(Clone)]
    struct Relay {
        query: String,
        events_status: StatusCode,
        events_body: String,
        captured: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    async fn fake_relay(
        query: serde_json::Value,
        events_status: StatusCode,
        events_body: &str,
    ) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let state = Relay {
            query: query.to_string(),
            events_status,
            events_body: events_body.to_string(),
            captured: captured.clone(),
        };
        let app = Router::new()
            .route(
                "/query",
                post(|State(s): State<Relay>| async move {
                    (
                        StatusCode::OK,
                        [("content-type", "application/json")],
                        s.query,
                    )
                }),
            )
            .route(
                "/events",
                post(|State(s): State<Relay>, body: String| async move {
                    s.captured
                        .lock()
                        .unwrap()
                        .push(serde_json::from_str(&body).unwrap());
                    (
                        s.events_status,
                        [("content-type", "application/json")],
                        s.events_body,
                    )
                }),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), captured)
    }

    fn message_in_channel() -> serde_json::Value {
        serde_json::json!([{
            "id": EVENT, "pubkey": AUTHOR, "kind": 9, "content": "hi",
            "created_at": 1, "tags": [["h", CHANNEL]], "sig": "00"
        }])
    }

    fn client(url: String) -> BuzzClient {
        BuzzClient::new(url, Keys::generate(), None, None).unwrap()
    }

    fn parse(args: &[&str]) -> Result<Cmd, clap::Error> {
        let mut argv = vec!["buzz", "messages", "report"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).map(|cli| cli.command)
    }

    fn tags(event: &serde_json::Value) -> Vec<Vec<String>> {
        serde_json::from_value(event["tags"].clone()).unwrap()
    }

    #[test]
    fn parse_requires_known_reason_and_one_target() {
        assert!(parse(&["--event", EVENT]).is_err(), "reason is required");
        assert!(
            parse(&["--event", EVENT, "--reason", "harassment"]).is_err(),
            "unknown reason"
        );
        assert!(parse(&["--reason", "spam"]).is_err(), "target is required");
        assert!(
            parse(&["--event", EVENT, "--pubkey", AUTHOR, "--reason", "spam"]).is_err(),
            "event and pubkey conflict"
        );
        assert!(
            parse(&["--channel", CHANNEL, "--pubkey", AUTHOR, "--reason", "spam"]).is_err(),
            "channel needs an event"
        );
        for reason in buzz_sdk::REPORT_TYPES {
            assert!(
                parse(&["--event", EVENT, "--reason", reason]).is_ok(),
                "{reason}"
            );
        }
        let Cmd::Messages(MessagesCmd::Report { reason, note, .. }) =
            parse(&["--pubkey", AUTHOR, "--reason", "spam", "--note", "x"]).unwrap()
        else {
            panic!("expected messages report");
        };
        assert_eq!((reason.as_str(), note.as_deref()), ("spam", Some("x")));
    }

    #[tokio::test]
    async fn reports_a_message_with_desktop_tag_shape() {
        let (url, captured) = fake_relay(
            message_in_channel(),
            StatusCode::OK,
            r#"{"event_id":"e1","accepted":true,"message":""}"#,
        )
        .await;
        cmd_report_message(
            &client(url),
            ReportParams {
                channel: Some(CHANNEL),
                event: Some(EVENT),
                pubkey: None,
                reason: "spam",
                note: Some(" buy now "),
            },
        )
        .await
        .unwrap();
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["kind"], 1984);
        assert_eq!(sent[0]["content"], "buy now");
        assert_eq!(
            tags(&sent[0]),
            vec![
                vec!["p".to_string(), AUTHOR.to_string()],
                vec!["e".to_string(), EVENT.to_string(), "spam".to_string()],
            ]
        );
    }

    #[tokio::test]
    async fn reports_a_user_without_a_message() {
        let (url, captured) = fake_relay(
            serde_json::json!([]),
            StatusCode::OK,
            r#"{"event_id":"e1","accepted":true,"message":""}"#,
        )
        .await;
        cmd_report_message(
            &client(url),
            ReportParams {
                channel: None,
                event: None,
                pubkey: Some(AUTHOR),
                reason: "impersonation",
                note: None,
            },
        )
        .await
        .unwrap();
        let sent = captured.lock().unwrap();
        assert_eq!(
            tags(&sent[0]),
            vec![vec![
                "p".to_string(),
                AUTHOR.to_string(),
                "impersonation".to_string()
            ]]
        );
    }

    #[tokio::test]
    async fn channel_mismatch_is_a_usage_error_and_sends_nothing() {
        let (url, captured) = fake_relay(message_in_channel(), StatusCode::OK, "{}").await;
        let err = cmd_report_message(
            &client(url),
            ReportParams {
                channel: Some("00000000-0000-0000-0000-000000000000"),
                event: Some(EVENT),
                pubkey: None,
                reason: "spam",
                note: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err}");
        assert_eq!(exit_code(&err), 1);
        assert!(captured.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn malformed_ids_are_usage_errors_before_any_network_call() {
        let dead = client("http://127.0.0.1:1".into());
        for (event, pubkey) in [(Some("abc"), None), (None, Some("abc"))] {
            let err = cmd_report_message(
                &dead,
                ReportParams {
                    channel: None,
                    event,
                    pubkey,
                    reason: "spam",
                    note: None,
                },
            )
            .await
            .unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{err}");
        }
    }

    #[tokio::test]
    async fn missing_event_is_not_found_and_sends_nothing() {
        let (url, captured) = fake_relay(serde_json::json!([]), StatusCode::OK, "{}").await;
        let err = cmd_report_message(
            &client(url),
            ReportParams {
                channel: None,
                event: Some(EVENT),
                pubkey: None,
                reason: "spam",
                note: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(exit_code(&err), 1);
        assert!(captured.lock().unwrap().is_empty());
    }

    async fn rejected_exit_code(status: StatusCode, body: &str) -> (CliError, i32) {
        let (url, _) = fake_relay(message_in_channel(), status, body).await;
        let err = cmd_report_message(
            &client(url),
            ReportParams {
                channel: None,
                event: Some(EVENT),
                pubkey: None,
                reason: "spam",
                note: None,
            },
        )
        .await
        .unwrap_err();
        let code = exit_code(&err);
        (err, code)
    }

    #[tokio::test]
    async fn relay_rejections_map_to_exit_codes() {
        let (err, code) = rejected_exit_code(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid: report target event not found"}"#,
        )
        .await;
        assert!(matches!(err, CliError::NotFound(_)), "{err}");
        assert_eq!(code, 1);

        let (_, code) = rejected_exit_code(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid: unsupported report type: x"}"#,
        )
        .await;
        assert_eq!(code, 2);

        let (_, code) = rejected_exit_code(
            StatusCode::FORBIDDEN,
            r#"{"error":"restricted: community writes are fenced"}"#,
        )
        .await;
        assert_eq!(code, 3);
    }
}
