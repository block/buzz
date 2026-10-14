//! Socket refusal -> production unread fetch/filter -> batch classifier.
use super::*;
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn refused_unread_history_recovers_unique_events_with_original_filters() {
    let keys = Keys::generate();
    let owner = keys.public_key().to_hex();
    let author = Keys::generate();
    let root = "a".repeat(64);
    let make = |content: &str, signer: &Keys, at, tags: Vec<Tag>| {
        EventBuilder::new(Kind::Custom(9), content)
            .custom_created_at(Timestamp::from(at))
            .tags(tags)
            .sign_with_keys(signer)
            .unwrap()
    };
    let participation = make(
        "participation",
        &keys,
        42,
        vec![
            Tag::parse(["h", "stream"]).unwrap(),
            Tag::parse(["e", &root, "", "reply"]).unwrap(),
        ],
    );
    let external = make(
        "reply",
        &author,
        43,
        vec![
            Tag::parse(["h", "stream"]).unwrap(),
            Tag::parse(["e", &root, "", "reply"]).unwrap(),
        ],
    );
    let dm = make("dm", &author, 101, vec![Tag::parse(["h", "dm"]).unwrap()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let expected = [participation.clone(), external.clone(), dm.clone()];
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ws.send(Message::Text(
            serde_json::json!(["AUTH", "unread-test"])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let mut originals = std::collections::HashMap::new();
        let mut attempts = std::collections::HashMap::<String, usize>::new();
        let mut completed = 0;
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
            let id = frame[1].as_str().unwrap_or_default();
            match frame[0].as_str() {
                Some("AUTH") => {
                    ws.send(Message::Text(
                        serde_json::json!(["OK", frame[1]["id"], true, ""])
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                }
                Some("REQ") => {
                    let channel = frame[2]["#h"][0].as_str().unwrap();
                    let count = attempts.entry(channel.to_owned()).or_default();
                    *count += 1;
                    if channel == "denied" {
                        ws.send(Message::Text(
                            serde_json::json!(["CLOSED", id, "restricted: denied"])
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                        continue;
                    }
                    assert_eq!(frame[2]["limit"], 1000);
                    assert_eq!(frame[2]["since"], if channel == "dm" { 101 } else { 42 });
                    assert_eq!(
                        frame[2]["kinds"],
                        if channel == "dm" {
                            serde_json::json!([9, 40002, 45001, 45003, KIND_HUDDLE_STARTED])
                        } else {
                            serde_json::json!([9, 40002, 45001, 45003])
                        }
                    );
                    if *count == 1 {
                        originals.insert(
                            channel.to_owned(),
                            (frame.clone(), std::time::Instant::now()),
                        );
                        if channel == "stream" {
                            ws.send(Message::Text(
                                serde_json::json!(["EVENT", id, participation])
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                        }
                        ws.send(Message::Text(
                            serde_json::json!([
                                "CLOSED",
                                id,
                                "rate-limited: quota exceeded; retry in 1s"
                            ])
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    } else {
                        assert_eq!(*count, 2);
                        let (original, refused) = &originals[channel];
                        assert_eq!(&frame, original, "retry must keep exact filter and id");
                        assert!(refused.elapsed() >= Duration::from_secs(1));
                        let events = if channel == "dm" {
                            vec![&dm]
                        } else {
                            vec![&participation, &external, &external]
                        };
                        for event in events {
                            ws.send(Message::Text(
                                serde_json::json!(["EVENT", id, event]).to_string().into(),
                            ))
                            .await
                            .unwrap();
                        }
                        ws.send(Message::Text(
                            serde_json::json!(["EOSE", id]).to_string().into(),
                        ))
                        .await
                        .unwrap();
                        completed += 1;
                    }
                }
                Some("CLOSE") if completed == 2 => break,
                _ => {}
            }
        }
        assert_eq!(completed, 2);
        assert_eq!(attempts["denied"], 1);
    });
    let (session, _archive) = crate::native_relay_client::start(url, keys, None).await;
    let request = UnreadCatchUpRequest {
        self_pubkey: owner,
        muted_channel_ids: HashSet::new(),
        channels: vec![
            ("stream", "stream", 41),
            ("dm", "dm", 100),
            ("denied", "stream", 41),
        ]
        .into_iter()
        .map(|(id, channel_type, read_at)| CatchUpChannel {
            id: id.into(),
            channel_type: channel_type.into(),
            name: id.into(),
            read_at: Some(read_at),
        })
        .collect(),
    };
    let (fetched, failures) = fetch_channels(std::sync::Arc::clone(&session), &request)
        .await
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert!(
        matches!(&failures[0], ChannelResult::Error {channel_id, error} if channel_id == "denied" && error.contains("restricted:"))
    );
    assert_eq!(fetched.iter().map(|f| f.events.len()).sum::<usize>(), 3);
    let result = classify_batch(&request, fetched, &std::collections::HashMap::new());
    let ChannelResult::Success {
        observed_events,
        max_trigger,
        discovered,
        ..
    } = &result[0]
    else {
        panic!("stream success")
    };
    assert_eq!(observed_events.len(), 1);
    assert_eq!(observed_events[0].id, expected[1].id.to_hex());
    assert!(observed_events[0].high_priority);
    assert_eq!(*max_trigger, 43);
    assert_eq!(discovered.participated, [root]);
    let ChannelResult::Success {
        observed_events, ..
    } = &result[1]
    else {
        panic!("dm success")
    };
    assert_eq!(observed_events.len(), 1);
    assert_eq!(observed_events[0].id, expected[2].id.to_hex());
    assert!(observed_events[0].counts_toward_app_badge);
    server.await.unwrap();
    session.shutdown();
}
