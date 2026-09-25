use std::{process::Stdio, time::Duration};

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    net::{TcpListener, TcpStream},
    process::{ChildStdin, ChildStdout, Command},
    time::timeout,
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

async fn wire(ws: &mut WebSocketStream<TcpStream>) -> Value {
    let frame = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("websocket deadline")
        .expect("frame")
        .expect("valid frame");
    serde_json::from_str(frame.to_text().expect("text")).expect("json")
}

async fn auth(listener: &TcpListener, human: &Keys) -> WebSocketStream<TcpStream> {
    let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("connect deadline")
        .expect("connection");
    let mut ws = accept_async(stream).await.expect("upgrade");
    ws.send(Message::Text(
        json!(["AUTH", "challenge"]).to_string().into(),
    ))
    .await
    .expect("challenge");
    let value = wire(&mut ws).await;
    assert_eq!(value[0], "AUTH");
    let event: Event = serde_json::from_value(value[1].clone()).expect("auth event");
    event.verify().expect("auth signature");
    assert_eq!(event.pubkey, human.public_key());
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["challenge", "challenge"]));
    ws.send(Message::Text(
        json!(["OK", event.id, true, ""]).to_string().into(),
    ))
    .await
    .expect("auth OK");
    ws
}

async fn response(lines: &mut Lines<BufReader<ChildStdout>>, field: &str, value: &str) -> Value {
    timeout(Duration::from_secs(8), async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("stdout")
                .expect("host alive");
            let message: Value = serde_json::from_str(&line).expect("host JSONL");
            if message[field] == value {
                return message;
            }
        }
    })
    .await
    .expect("host response deadline")
}

async fn request(stdin: &mut ChildStdin, id: &str, method: &str, params: Value) {
    stdin
        .write_all(
            format!(
                "{}\n",
                json!({"id": id, "method": method, "params": params})
            )
            .as_bytes(),
        )
        .await
        .expect("request");
}

#[tokio::test]
async fn stdio_auth_live_history_reconnect_and_idempotent_send() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
    let human = Keys::generate();
    let agent = Keys::generate();
    let relay = Keys::generate();
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-terminal-host"))
        .env("BUZZ_PRIVATE_KEY", human.secret_key().to_secret_hex())
        .env(
            "BUZZ_RELAY_URL",
            format!("ws://{}", listener.local_addr().expect("address")),
        )
        .env("BUZZ_RELAY_PUBKEY", relay.public_key().to_hex())
        .env_remove("BUZZ_AUTH_TAG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("host");
    let mut input = child.stdin.take().expect("stdin");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let ready = response(&mut lines, "type", "ready").await;
    assert_eq!(ready["pubkey"], human.public_key().to_hex());
    assert!(ready.get("privateKey").is_none());
    let mut ws = auth(&listener, &human).await;
    response(&mut lines, "status", "connected").await;
    let channel = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let root = "1".repeat(64);
    let subscription = format!("{channel}:{root}");
    let filters = json!([{"kinds":[9],"#h":[channel],"limit":200}]);
    request(
        &mut input,
        "sub",
        "subscribe",
        json!({"subscriptionId":subscription,"filters":filters}),
    )
    .await;
    let req = wire(&mut ws).await;
    assert_eq!(req, json!(["REQ", subscription, filters[0]]));
    response(&mut lines, "id", "sub").await;
    let history = EventBuilder::new(Kind::Custom(9), "history")
        .tags([Tag::parse(["h", channel]).expect("tag")])
        .sign_with_keys(&agent)
        .expect("event");
    let mut forged = serde_json::to_value(&history).expect("json");
    forged["content"] = json!("forged");
    ws.send(Message::Text(
        json!(["EVENT", subscription, forged]).to_string().into(),
    ))
    .await
    .expect("invalid event");
    ws.send(Message::Text(
        json!(["EVENT", subscription, history]).to_string().into(),
    ))
    .await
    .expect("history");
    assert_eq!(
        response(&mut lines, "type", "event").await["event"]["content"],
        "history"
    );
    ws.send(Message::Text(
        json!(["EOSE", subscription]).to_string().into(),
    ))
    .await
    .expect("eose");
    response(&mut lines, "type", "eose").await;
    let payload = json!({"requestKey":"stable-send","channelId":channel,"content":"hello Atlas","recipientPubkeys":[agent.public_key().to_hex()],"rootEventId":root});
    request(&mut input, "send", "sendMessage", payload.clone()).await;
    let published = wire(&mut ws).await;
    let signed: Event = serde_json::from_value(published[1].clone()).expect("signed message");
    signed.verify().expect("valid signature");
    assert_eq!(signed.pubkey, human.public_key());
    assert!(signed
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["h", channel]));
    assert!(signed
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["p", &agent.public_key().to_hex()]));
    assert!(signed
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["e", &root, "", "reply"]));
    ws.close(None).await.expect("disconnect before ack");
    let failed = response(&mut lines, "id", "send").await;
    assert_eq!(failed["error"]["code"], "uncertain");
    assert_eq!(failed["error"]["eventId"], signed.id.to_hex());
    let mut ws = auth(&listener, &human).await;
    assert_eq!(wire(&mut ws).await, req);
    response(&mut lines, "status", "connected").await;
    let mut wrong = payload.clone();
    wrong["channelId"] = json!("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    request(&mut input, "wrong", "sendMessage", wrong).await;
    assert_eq!(
        response(&mut lines, "id", "wrong").await["error"]["code"],
        "request_key_conflict"
    );
    request(&mut input, "retry", "sendMessage", payload.clone()).await;
    assert_eq!(wire(&mut ws).await, published);
    ws.send(Message::Text(
        json!(["OK", signed.id, true, ""]).to_string().into(),
    ))
    .await
    .expect("ack retry");
    assert_eq!(
        response(&mut lines, "id", "retry").await["result"]["accepted"],
        true
    );
    request(&mut input, "accepted-again", "sendMessage", payload).await;
    assert_eq!(
        response(&mut lines, "id", "accepted-again").await["result"]["event"]["id"],
        signed.id.to_hex()
    );
    for (method, kind, extra_tag) in [
        ("createChannel", 9007, json!(["visibility", "private"])),
        ("addMember", 9000, json!(["p", agent.public_key().to_hex()])),
        ("joinChannel", 9021, json!(["h", channel])),
    ] {
        let params =
            json!({"requestKey":method,"channelId":channel,"pubkey":agent.public_key().to_hex()});
        request(&mut input, method, method, params.clone()).await;
        let wire_event = wire(&mut ws).await;
        assert_eq!(wire_event[0], "EVENT");
        let event: Event = serde_json::from_value(wire_event[1].clone()).expect("channel event");
        event.verify().expect("channel signature");
        assert_eq!(event.pubkey, human.public_key());
        assert_eq!(event.kind.as_u16(), kind);
        assert!(wire_event[1]["tags"]
            .as_array()
            .expect("tags")
            .contains(&extra_tag));
        assert!(wire_event[1]["tags"]
            .as_array()
            .expect("tags")
            .contains(&json!(["h", channel])));
        ws.send(Message::Text(
            json!(["OK", event.id, true, ""]).to_string().into(),
        ))
        .await
        .expect("channel ack");
        assert_eq!(
            response(&mut lines, "id", method).await["result"]["accepted"],
            true
        );
        request(&mut input, "cached-channel", method, params).await;
        assert_eq!(
            response(&mut lines, "id", "cached-channel").await["result"]["event"]["id"],
            event.id.to_hex()
        );
    }
    request(
        &mut input,
        "close",
        "unsubscribe",
        json!({"subscriptionId":subscription}),
    )
    .await;
    assert_eq!(wire(&mut ws).await, json!(["CLOSE", subscription]));
    response(&mut lines, "id", "close").await;
    drop(input);
    assert!(timeout(Duration::from_secs(3), child.wait())
        .await
        .expect("EOF shuts down host")
        .expect("exit")
        .success());
}
