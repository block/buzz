use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

async fn exercise(tamper: bool) -> std::process::Output {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let author = Keys::generate();
    let channel = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(9), "worker complete")
        .tags([Tag::parse(["h", channel.as_str()]).unwrap()])
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&author)
        .unwrap();
    let live_event = EventBuilder::new(Kind::Custom(9), "live progress")
        .tags([Tag::parse(["h", channel.as_str()]).unwrap()])
        .custom_created_at(Timestamp::from(101))
        .sign_with_keys(&author)
        .unwrap();
    let mut event: Value = serde_json::from_str(&event.as_json()).unwrap();
    if tamper {
        event["content"] = json!("forged");
    }
    let expected_author = author.public_key().to_hex();
    let channel_for_server = channel.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ws.send(Message::Text(
            json!(["AUTH", "test-challenge"]).to_string().into(),
        ))
        .await
        .unwrap();
        let auth: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(auth[0], "AUTH");
        let signed: nostr::Event = serde_json::from_value(auth[1].clone()).unwrap();
        signed.verify().unwrap();
        ws.send(Message::Text(
            json!(["OK", auth[1]["id"], true, ""]).to_string().into(),
        ))
        .await
        .unwrap();
        let req: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(req[0], "REQ");
        assert_eq!(req[2]["#h"], json!([channel_for_server]));
        assert_eq!(req[2]["authors"], json!([expected_author]));
        assert_eq!(req[2]["since"], 100);
        for frame in [
            json!(["EVENT", req[1], event]),
            json!(["EOSE", req[1]]),
            json!(["EVENT", req[1], live_event]),
            json!(["CLOSED", req[1], "fixture finished"]),
        ] {
            if ws
                .send(Message::Text(frame.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_buzz"))
            .env(
                "BUZZ_PRIVATE_KEY",
                Keys::generate().secret_key().to_secret_hex(),
            )
            .args([
                "--relay",
                &format!("ws://{address}"),
                "messages",
                "watch",
                "--channel",
                &channel,
                "--author",
                &author.public_key().to_hex(),
                "--since",
                "100",
            ])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.await.unwrap();
    output
}

#[tokio::test]
async fn authenticated_watch_emits_history_and_ready_then_reports_closure() {
    let output = exercise(false).await;
    assert!(!output.status.success());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2]["event"]["content"], "live progress");
    assert_eq!(lines[0]["event"]["content"], "worker complete");
    assert!(lines[0]["event"]["sig"].is_string());
    assert_eq!(lines[1], json!({"type":"ready"}));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("fixture finished"));
}

#[tokio::test]
async fn forged_event_never_reaches_stdout() {
    let output = exercise(true).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("invalid signed event"));
}
