//! `buzz raw`: catch-all escape hatch for any Nostr read or write the
//! purpose-built commands do not cover. Both subcommands go through the same
//! client paths as every other command, so credentials, the NIP-OA auth tag
//! and the NIP-FI assertion are handled identically.

use std::io::Write;

use buzz_core::kind::{KIND_GIFT_WRAP, KIND_NIP43_LEAVE_REQUEST};
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde::Deserialize;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::read_or_stdin;

/// An unsigned event template accepted by `raw publish --event-json`.
///
/// Unknown fields (`id`, `pubkey`, `sig`, typos) are rejected rather than
/// silently dropped: the CLI always signs with its own identity.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventTemplate {
    kind: u16,
    #[serde(default)]
    content: String,
    #[serde(default)]
    tags: Vec<Vec<String>>,
    #[serde(default)]
    created_at: Option<u64>,
}

/// Parse one filter object or a non-empty array of filter objects.
fn parse_filters(input: &str) -> Result<Vec<serde_json::Value>, CliError> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| CliError::Usage(format!("invalid filter JSON: {e}")))?;
    let filters = match value {
        serde_json::Value::Array(filters) => filters,
        object @ serde_json::Value::Object(_) => vec![object],
        _ => {
            return Err(CliError::Usage(
                "filter must be a JSON object or an array of objects".into(),
            ))
        }
    };
    if filters.is_empty() {
        return Err(CliError::Usage("filter array must not be empty".into()));
    }
    if let Some(bad) = filters.iter().find(|f| !f.is_object()) {
        return Err(CliError::Usage(format!(
            "every filter must be a JSON object, got: {bad}"
        )));
    }
    Ok(filters)
}

/// Parse one `--tag` value: a JSON array of strings, e.g. `["p","<hex>"]`.
fn parse_tag(input: &str) -> Result<Vec<String>, CliError> {
    let parts: Vec<String> = serde_json::from_str(input).map_err(|e| {
        CliError::Usage(format!(
            "invalid --tag {input:?}: expected a JSON array of strings, e.g. '[\"p\",\"<hex>\"]' ({e})"
        ))
    })?;
    if parts.is_empty() {
        return Err(CliError::Usage("--tag must not be an empty array".into()));
    }
    Ok(parts)
}

fn build_tags(raw: &[Vec<String>]) -> Result<Vec<Tag>, CliError> {
    raw.iter()
        .map(|parts| {
            if parts.first().map(String::as_str) == Some("auth") {
                return Err(CliError::Usage(
                    "auth tags are attached from the CLI's own credentials; remove the \"auth\" tag"
                        .into(),
                ));
            }
            Tag::parse(parts.iter().map(String::as_str))
                .map_err(|e| CliError::Usage(format!("invalid tag {parts:?}: {e}")))
        })
        .collect()
}

pub async fn cmd_query(
    client: &BuzzClient,
    filter: &str,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let filters = parse_filters(&read_or_stdin(filter)?)?;
    let resp = client.query_multi(&filters).await?;
    // Print the relay's JSON in its own shape: most filters return an event
    // array, but some (read-state snapshots, incremental artifacts) return an
    // object.
    let value: serde_json::Value = serde_json::from_str(&resp)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    writeln!(out, "{value}").map_err(|e| CliError::Other(format!("write failed: {e}")))
}

pub async fn cmd_publish(
    client: &BuzzClient,
    kind: Option<u16>,
    tags: &[String],
    content: Option<&str>,
    event_json: Option<&str>,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let template = match (event_json, kind) {
        (Some(json), _) => {
            let json = read_or_stdin(json)?;
            serde_json::from_str::<EventTemplate>(&json)
                .map_err(|e| CliError::Usage(format!("invalid --event-json: {e}")))?
        }
        (None, Some(kind)) => EventTemplate {
            kind,
            content: match content {
                Some(content) => read_or_stdin(content)?,
                None => String::new(),
            },
            tags: tags
                .iter()
                .map(|t| parse_tag(t))
                .collect::<Result<_, _>>()?,
            created_at: None,
        },
        (None, None) => {
            return Err(CliError::Usage(
                "provide --kind (with optional --tag and --content) or --event-json".into(),
            ))
        }
    };

    if u32::from(template.kind) == KIND_GIFT_WRAP {
        return Err(CliError::Usage(
            "raw publish cannot send gift wraps (kind 1059): they need NIP-59 signing with \
             a throwaway key, and the relay refuses them over HTTP"
                .into(),
        ));
    }

    let mut builder = EventBuilder::new(Kind::Custom(template.kind), template.content)
        .tags(build_tags(&template.tags)?)
        // Keep a `p` tag naming the signer: nostr drops them by default, and
        // `raw publish` must send the tags it was given.
        .allow_self_tagging();
    if let Some(created_at) = template.created_at {
        builder = builder.custom_created_at(Timestamp::from(created_at));
    }
    let event = client.sign_event(builder).await?;

    // The relay takes only typing (kind 20002) of the ephemeral range over
    // HTTP; the rest of 20000-29999 must go over the WebSocket. The NIP-43
    // leave request is the exception: the relay revokes the submitting socket
    // as it removes the membership, so over the WebSocket a successful leave
    // reads as a failure.
    let ephemeral = (20000..30000).contains(&template.kind);
    let resp = if ephemeral && u32::from(template.kind) != KIND_NIP43_LEAVE_REQUEST {
        client.publish_ephemeral_event(event).await?
    } else {
        client.submit_event(event).await?
    };
    // The relay refuses some events over HTTP with a 2xx and an explicit
    // `accepted:false` (a duplicate reaction, for example). Fail like the
    // WebSocket path does, keeping the event id and message in the error.
    let refused = serde_json::from_str::<serde_json::Value>(&resp)
        .ok()
        .and_then(|v| v.get("accepted").and_then(serde_json::Value::as_bool))
        == Some(false);
    let resp = normalize_write_response(&resp);
    if refused {
        return Err(CliError::Relay {
            status: 400,
            body: resp,
        });
    }
    writeln!(out, "{resp}").map_err(|e| CliError::Other(format!("write failed: {e}")))
}

pub async fn dispatch(cmd: crate::RawCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::RawCmd;
    match cmd {
        RawCmd::Query { filter } => cmd_query(client, &filter, &mut std::io::stdout()).await,
        RawCmd::Publish {
            kind,
            tag,
            content,
            event_json,
        } => {
            cmd_publish(
                client,
                kind,
                &tag,
                content.as_deref(),
                event_json.as_deref(),
                &mut std::io::stdout(),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Cmd, RawCmd};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use clap::Parser;
    use nostr::Keys;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[derive(Clone, Default)]
    struct Captured {
        queries: Arc<Mutex<Vec<serde_json::Value>>>,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
        auth_tags: Arc<Mutex<Vec<Option<String>>>>,
    }

    /// A local relay bridge: `/query` records its body and returns `reply`;
    /// `/events` records the signed event and accepts it.
    async fn fake_relay(reply: &'static str) -> (String, Captured) {
        fake_relay_acking(reply, true).await
    }

    /// Like [`fake_relay`], but `/events` answers HTTP 200 with the given
    /// `accepted` flag, as the relay does for a duplicate reaction.
    async fn fake_relay_acking(reply: &'static str, accepted: bool) -> (String, Captured) {
        let captured = Captured::default();
        let app = Router::new()
            .route(
                "/query",
                post(
                    move |State(c): State<Captured>, headers: HeaderMap, body: String| async move {
                        c.queries
                            .lock()
                            .unwrap()
                            .push(serde_json::from_str(&body).unwrap());
                        c.auth_tags.lock().unwrap().push(
                            headers
                                .get("x-auth-tag")
                                .map(|v| v.to_str().unwrap().to_string()),
                        );
                        (
                            StatusCode::OK,
                            [("content-type", "application/json")],
                            reply,
                        )
                    },
                ),
            )
            .route(
                "/events",
                post(
                    move |State(c): State<Captured>, headers: HeaderMap, body: String| async move {
                        let event: serde_json::Value = serde_json::from_str(&body).unwrap();
                        let id = event["id"].as_str().unwrap().to_string();
                        c.events.lock().unwrap().push(event);
                        c.auth_tags.lock().unwrap().push(
                            headers
                                .get("x-auth-tag")
                                .map(|v| v.to_str().unwrap().to_string()),
                        );
                        (
                            StatusCode::OK,
                            [("content-type", "application/json")],
                            if accepted {
                                format!(r#"{{"event_id":"{id}","accepted":true,"message":""}}"#)
                            } else {
                                format!(
                                    r#"{{"event_id":"{id}","accepted":false,"message":"duplicate: reaction already exists"}}"#
                                )
                            },
                        )
                    },
                ),
            )
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), captured)
    }

    fn client(url: &str) -> BuzzClient {
        BuzzClient::new(
            url.to_string(),
            crate::credentials::Credentials::from_keys(Keys::generate()),
        )
        .unwrap()
    }

    fn client_with_auth_tag(url: &str) -> (BuzzClient, String) {
        let parts = vec![
            "auth".to_string(),
            "a".repeat(64),
            "conditions".to_string(),
            "b".repeat(128),
        ];
        let json = serde_json::to_string(&parts).unwrap();
        let tag = Tag::parse(parts).unwrap();
        let client = BuzzClient::new(
            url.to_string(),
            crate::credentials::Credentials::from_keys(Keys::generate())
                .with_auth_tag(tag, json.clone()),
        )
        .unwrap();
        (client, json)
    }

    fn tags_of(event: &serde_json::Value) -> Vec<Vec<String>> {
        serde_json::from_value(event["tags"].clone()).unwrap()
    }

    #[test]
    fn parse_accepts_query_and_publish_forms() {
        let Cmd::Raw(RawCmd::Query { filter }) =
            Cli::try_parse_from(["buzz", "raw", "query", r#"{"kinds":[1]}"#])
                .unwrap()
                .command
        else {
            panic!("expected raw query");
        };
        assert_eq!(filter, r#"{"kinds":[1]}"#);

        let Cmd::Raw(RawCmd::Publish { kind, tag, .. }) = Cli::try_parse_from([
            "buzz",
            "raw",
            "publish",
            "--kind",
            "1",
            "--tag",
            r#"["t","a"]"#,
            "--tag",
            r#"["t","b"]"#,
        ])
        .unwrap()
        .command
        else {
            panic!("expected raw publish");
        };
        assert_eq!((kind, tag.len()), (Some(1), 2));

        assert!(
            Cli::try_parse_from([
                "buzz",
                "raw",
                "publish",
                "--kind",
                "1",
                "--event-json",
                "{}"
            ])
            .is_err(),
            "--event-json conflicts with --kind"
        );
    }

    #[test]
    fn filters_parse_as_object_or_nonempty_array_of_objects() {
        assert_eq!(parse_filters(r#"{"kinds":[1]}"#).unwrap().len(), 1);
        assert_eq!(
            parse_filters(r#"[{"kinds":[1]},{"kinds":[2]}]"#)
                .unwrap()
                .len(),
            2
        );
        for bad in ["not json", "[]", "7", r#"["x"]"#, r#"[{"kinds":[1]},3]"#] {
            assert!(
                matches!(parse_filters(bad), Err(CliError::Usage(_))),
                "{bad} must be a usage error"
            );
        }
    }

    #[test]
    fn tags_must_be_json_string_arrays_without_auth() {
        assert_eq!(parse_tag(r#"["p","x"]"#).unwrap(), vec!["p", "x"]);
        for bad in ["p,x", "[]", r#"[1]"#, r#"{"p":1}"#] {
            assert!(
                matches!(parse_tag(bad), Err(CliError::Usage(_))),
                "{bad} must be a usage error"
            );
        }
        let err = build_tags(&[vec!["auth".into(), HEX.into(), "c".into(), "d".into()]]);
        assert!(matches!(err, Err(CliError::Usage(msg)) if msg.contains("auth")));
    }

    #[tokio::test]
    async fn query_posts_filters_with_credentials_and_prints_events() {
        let (url, captured) = fake_relay(r#"[{"id":"e1","kind":39002,"sig":"00"}]"#).await;
        let (client, auth_json) = client_with_auth_tag(&url);

        let filter = format!(r##"[{{"kinds":[39002],"#p":["{HEX}"]}},{{"kinds":[0],"limit":1}}]"##);
        let mut out = Vec::new();
        cmd_query(&client, &filter, &mut out).await.unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[{\"id\":\"e1\",\"kind\":39002,\"sig\":\"00\"}]\n",
            "prints the relay's events, signatures included"
        );

        let queries = captured.queries.lock().unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(
            queries[0],
            serde_json::json!([{"kinds":[39002],"#p":[HEX]},{"kinds":[0],"limit":1}])
        );
        assert_eq!(
            captured.auth_tags.lock().unwrap()[0].as_deref(),
            Some(auth_json.as_str()),
            "the NIP-OA auth tag rides on the query"
        );
    }

    #[tokio::test]
    async fn query_wraps_a_single_filter_and_keeps_object_responses() {
        let (url, captured) = fake_relay("[]").await;
        cmd_query(&client(&url), r#"{"kinds":[1],"limit":5}"#, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(
            captured.queries.lock().unwrap()[0],
            serde_json::json!([{"kinds":[1],"limit":5}])
        );

        let (url, _) = fake_relay(r#"{"snapshot":true}"#).await;
        let mut out = Vec::new();
        cmd_query(&client(&url), r#"{"kinds":[1]}"#, &mut out)
            .await
            .expect("an object response is valid relay output");
        assert_eq!(String::from_utf8(out).unwrap(), "{\"snapshot\":true}\n");

        let (url, _) = fake_relay("not json").await;
        let err = cmd_query(&client(&url), r#"{"kinds":[1]}"#, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Other(msg) if msg.contains("parse query response")));
    }

    #[tokio::test]
    async fn publish_signs_and_submits_kind_tags_content_and_auth_tag() {
        let (url, captured) = fake_relay("[]").await;
        let (client, auth_json) = client_with_auth_tag(&url);

        cmd_publish(
            &client,
            Some(9),
            &[format!(r#"["p","{HEX}"]"#), r#"["h","chan"]"#.to_string()],
            Some("hello"),
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();

        let events = captured.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event["kind"], 9);
        assert_eq!(event["content"], "hello");
        assert_eq!(event["pubkey"], client.public_key().to_hex());
        let tags = tags_of(event);
        assert_eq!(tags[0], vec!["p", HEX]);
        assert_eq!(tags[1], vec!["h", "chan"]);
        assert_eq!(
            tags.iter().filter(|t| t[0] == "auth").count(),
            1,
            "exactly the credentials' auth tag is attached"
        );
        assert_eq!(
            captured.auth_tags.lock().unwrap()[0].as_deref(),
            Some(auth_json.as_str())
        );
    }

    #[tokio::test]
    async fn publish_keeps_a_p_tag_naming_the_signer() {
        let (url, captured) = fake_relay("[]").await;
        let client = client(&url);
        let me = client.public_key().to_hex();

        cmd_publish(
            &client,
            Some(1),
            &[format!(r#"["p","{me}"]"#)],
            None,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        let template = format!(r#"{{"kind":1,"tags":[["p","{me}"]]}}"#);
        cmd_publish(&client, None, &[], None, Some(&template), &mut Vec::new())
            .await
            .unwrap();

        let events = captured.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        for event in events.iter() {
            assert_eq!(tags_of(event), vec![vec!["p".to_string(), me.clone()]]);
        }
    }

    #[tokio::test]
    async fn publish_event_json_template_sets_created_at_and_rejects_unknown_fields() {
        let (url, captured) = fake_relay("[]").await;
        let client = client(&url);

        cmd_publish(
            &client,
            None,
            &[],
            None,
            Some(r#"{"kind":1,"content":"hi","tags":[["t","x"]],"created_at":1700000000}"#),
            &mut Vec::new(),
        )
        .await
        .unwrap();
        {
            let events = captured.events.lock().unwrap();
            assert_eq!(events[0]["created_at"], 1_700_000_000_u64);
            assert_eq!(events[0]["content"], "hi");
            assert_eq!(tags_of(&events[0]), vec![vec!["t", "x"]]);
        }

        for bad in [
            r#"{"kind":1,"sig":"00"}"#,
            r#"{"kind":1,"pubkey":"aa"}"#,
            r#"{"content":"no kind"}"#,
            "not json",
        ] {
            let err = cmd_publish(&client, None, &[], None, Some(bad), &mut Vec::new())
                .await
                .unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{bad}: {err:?}");
        }
        assert_eq!(
            captured.events.lock().unwrap().len(),
            1,
            "nothing else was sent"
        );
    }

    #[tokio::test]
    async fn publish_without_kind_or_template_is_a_usage_error_and_sends_nothing() {
        let (url, captured) = fake_relay("[]").await;
        let err = cmd_publish(&client(&url), None, &[], Some("x"), None, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));

        let err = cmd_publish(
            &client(&url),
            Some(1),
            &[format!(r#"["auth","{HEX}","c","d"]"#)],
            None,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));

        let err = cmd_publish(&client(&url), Some(1059), &[], None, None, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(msg) if msg.contains("gift wrap")));
        assert!(captured.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_sends_a_leave_request_over_http() {
        // Other ephemeral kinds go over the WebSocket, which this fake relay
        // does not serve; a leave must reach `/events`.
        let (url, captured) = fake_relay("[]").await;
        cmd_publish(&client(&url), Some(28936), &[], None, None, &mut Vec::new())
            .await
            .unwrap();
        let events = captured.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["kind"], 28936);
    }

    #[tokio::test]
    async fn publish_prints_the_relay_acknowledgement() {
        let (url, captured) = fake_relay("[]").await;
        let mut out = Vec::new();
        cmd_publish(&client(&url), Some(1), &[], Some("hi"), None, &mut out)
            .await
            .unwrap();
        let id = captured.events.lock().unwrap()[0]["id"].clone();
        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            printed,
            serde_json::json!({"event_id": id, "accepted": true, "message": ""})
        );
    }

    #[tokio::test]
    async fn publish_fails_when_the_relay_refuses_over_http() {
        let (url, captured) = fake_relay_acking("[]", false).await;
        let mut out = Vec::new();
        let err = cmd_publish(&client(&url), Some(7), &[], Some("+"), None, &mut out)
            .await
            .unwrap_err();
        let id = captured.events.lock().unwrap()[0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let CliError::Relay { status, body } = &err else {
            panic!("expected a relay error, got {err:?}");
        };
        assert_eq!(*status, 400);
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["event_id"], id, "the error keeps the event id");
        assert_eq!(body["accepted"], false);
        assert_eq!(body["message"], "duplicate: reaction already exists");
        assert!(out.is_empty(), "a refusal prints nothing to stdout");
        assert_eq!(crate::error::exit_code(&err), 2);
    }
}
