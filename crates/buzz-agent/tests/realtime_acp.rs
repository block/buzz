//! Real ACP process -> standard Realtime WS -> existing permission/MCP seam.
mod common;

use base64::{engine::general_purpose::STANDARD, Engine};
use common::{approve_permission, Harness};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;
async fn send(ws: &mut Ws, event: Value) {
    ws.send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}
async fn recv(ws: &mut Ws) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(msg.to_text().unwrap()).unwrap()
}
async fn modality(ws: &mut Ws) {
    let update = recv(ws).await;
    assert_eq!(update["type"], "session.update");
    send(
        ws,
        json!({"type":"session.updated", "session":update["session"]}),
    )
    .await;
}
async fn created(ws: &mut Ws, id: &str) {
    send(ws, json!({"type":"response.created", "response":{"id":id}})).await;
}
fn call(id: &str) -> Value {
    json!({"id":format!("item_{id}"), "type":"function_call", "status":"completed", "call_id":id, "name":"fake__shell", "arguments":"{\"command\":\"echo realtime\"}"})
}
async fn done(ws: &mut Ws, id: &str, output: Vec<Value>) {
    send(
        ws,
        json!({"type":"response.done", "response":{"id":id,"status":"completed", "output":output}}),
    )
    .await;
}
async fn text(ws: &mut Ws, id: &str) {
    created(ws, id).await;
    done(ws, id, vec![json!({"type":"message", "id":format!("item_{id}"), "role":"assistant", "status":"completed", "content":[{"type":"output_text", "text":"done"}]})]).await;
}
async fn connect(listener: TcpListener) -> Ws {
    let (socket, _) = listener.accept().await.unwrap();
    let mut ws = accept_async(socket).await.unwrap();
    send(
        &mut ws,
        json!({"type":"session.created", "session":{"id":"s1"}}),
    )
    .await;
    let update = recv(&mut ws).await;
    assert_eq!(update["type"], "session.update");
    assert_eq!(update["session"]["output_modalities"], json!(["text"]));
    assert!(update["session"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "fake__shell"));
    send(
        &mut ws,
        json!({"type":"session.updated", "session":update["session"]}),
    )
    .await;
    modality(&mut ws).await;
    assert_eq!(recv(&mut ws).await["type"], "conversation.item.create");
    assert_eq!(recv(&mut ws).await["type"], "response.create");
    ws
}
async fn init(h: &mut Harness, dir: &std::path::Path) -> String {
    let i = h
        .send(
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        )
        .await;
    let init = h.recv_until(|v| v["id"] == i).await;
    assert_eq!(
        init["result"]["agentCapabilities"]["promptCapabilities"]["audio"],
        true
    );
    let i = h.send("session/new", json!({"cwd":dir, "mcpServers":[{"name":"fake", "command":env!("CARGO_BIN_EXE_fake-mcp"), "args":[], "env":[{"name":"FAKE_MCP_SHELL_TOOL","value":"1"},{"name":"FAKE_MCP_CALL_LOG","value":dir.join("calls.log")}]}]})).await;
    let result = h.recv_until(|v| v["id"] == i).await;
    result["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("{result}: {}", h.stderr_text()))
        .to_owned()
}
async fn prompt(h: &mut Harness, sid: &str) -> i64 {
    h.send(
        "session/prompt",
        json!({"sessionId":sid,"prompt":[{"type":"text","text":"use shell"}]}),
    )
    .await
}
fn options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("OPENAI_COMPAT_API", "realtime"),
        ("MCP_HOOK_SERVERS", ""),
        ("BUZZ_AGENT_REQUIRE_REPLY", "0"),
    ]
}

#[tokio::test]
async fn realtime_acp_permission_and_persistent_conversation() {
    for allowed in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut ws = connect(listener).await;
            created(&mut ws, "r1").await;
            done(&mut ws, "r1", vec![call("c1")]).await;
            let output = recv(&mut ws).await;
            assert_eq!(output["type"], "conversation.item.create");
            assert_eq!(output["item"]["type"], "function_call_output");
            assert_eq!(output["item"]["call_id"], "c1");
            if !allowed {
                assert!(output["item"]["output"]
                    .as_str()
                    .unwrap()
                    .contains("permission"));
            }
            assert_eq!(recv(&mut ws).await["type"], "response.create");
            text(&mut ws, "r2").await;
            // A second ACP prompt must use the SAME socket, not a new session.
            modality(&mut ws).await;
            assert_eq!(recv(&mut ws).await["type"], "conversation.item.create");
            assert_eq!(recv(&mut ws).await["type"], "response.create");
            text(&mut ws, "r3").await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let mut h = Harness::spawn_with_env(&url, &options()).await;
        let sid = init(&mut h, dir.path()).await;
        let p = prompt(&mut h, &sid).await;
        let ask = h
            .recv_until(|v| v["method"] == "session/request_permission" || v["id"] == p)
            .await;
        assert_eq!(ask["method"], "session/request_permission", "{ask}");
        assert!(
            !dir.path().join("calls.log").exists(),
            "tool ran before approval"
        );
        if allowed {
            h.write(approve_permission(&ask)).await;
        } else {
            h.write(json!({"jsonrpc":"2.0", "id":ask["id"], "result":{"outcome":{"outcome":"cancelled"}}})).await;
        }
        let result = h.recv_until(|v| v["id"] == p).await;
        assert_eq!(result["result"]["stopReason"], "end_turn", "{result}");
        let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap_or_default();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            if allowed { vec!["shell"] } else { vec![] }
        );
        let p = prompt(&mut h, &sid).await;
        let result = h.recv_until(|v| v["id"] == p).await;
        assert_eq!(result["result"]["stopReason"], "end_turn", "{result}");
        server.await.unwrap();
    }
}

#[tokio::test]
async fn realtime_acp_duplicate_calls_fail_before_permission_or_effects() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut ws = connect(listener).await;
        created(&mut ws, "r1").await;
        done(&mut ws, "r1", vec![call("c1"), call("c1")]).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let mut h = Harness::spawn_with_env(&url, &options()).await;
    let sid = init(&mut h, dir.path()).await;
    let p = prompt(&mut h, &sid).await;
    loop {
        let event = h.recv().await;
        assert_ne!(event["method"], "session/request_permission");
        if event["id"] == p {
            assert!(event.get("error").is_some(), "{event}");
            break;
        }
    }
    assert!(!dir.path().join("calls.log").exists());
    let p = prompt(&mut h, &sid).await;
    let result = h.recv_until(|v| v["id"] == p).await;
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("session ended"));
    server.await.unwrap();
}

#[tokio::test]
async fn realtime_acp_cancel_reaches_provider_while_read_pending() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut ws = connect(listener).await;
        created(&mut ws, "r1").await;
        ready_tx.send(()).unwrap();
        let cancel = recv(&mut ws).await;
        assert_eq!(cancel["type"], "response.cancel");
        assert_eq!(cancel["response_id"], "r1");
    });
    let mut h = Harness::spawn_with_env(&url, &options()).await;
    let sid = init(&mut h, dir.path()).await;
    let p = prompt(&mut h, &sid).await;
    ready_rx.await.unwrap();
    // Allow the created frame to reach the driver before cancelling.
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.notify("session/cancel", json!({"sessionId":sid})).await;
    let result = h.recv_until(|v| v["id"] == p).await;
    assert_eq!(result["result"]["stopReason"], "cancelled", "{result}");
    server.await.unwrap();
    assert!(!dir.path().join("calls.log").exists());
}

#[tokio::test]
async fn realtime_acp_audio_content_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let pcm = vec![0u8; 4800];
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&4836u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&[1, 0, 1, 0]);
    wav.extend_from_slice(&24000u32.to_le_bytes());
    wav.extend_from_slice(&48000u32.to_le_bytes());
    wav.extend_from_slice(&[2, 0, 16, 0]);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&4800u32.to_le_bytes());
    wav.extend_from_slice(&pcm);
    let expected_wav = wav.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        send(
            &mut ws,
            json!({"type":"session.created", "session":{"id":"s1"}}),
        )
        .await;
        let update = recv(&mut ws).await;
        assert_eq!(update["session"]["output_modalities"], json!(["audio"]));
        send(
            &mut ws,
            json!({"type":"session.updated","session":update["session"]}),
        )
        .await;
        modality(&mut ws).await;
        let append = recv(&mut ws).await;
        assert_eq!(append["type"], "input_audio_buffer.append");
        assert_eq!(
            STANDARD.decode(append["audio"].as_str().unwrap()).unwrap(),
            pcm
        );
        assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.commit");
        assert_eq!(recv(&mut ws).await["type"], "response.create");
        created(&mut ws, "r1").await;
        send(&mut ws,json!({"type":"response.output_audio.delta","response_id":"r1","item_id":"a1","content_index":0,"delta":STANDARD.encode(pcm)})).await;
        done(&mut ws,"r1",vec![json!({"id":"a1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_audio","transcript":"hello"}]})]).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let mut h = Harness::spawn_with_env(&url, &options()).await;
    let sid = init(&mut h, dir.path()).await;
    let p = h.send("session/prompt",json!({"sessionId":sid,"prompt":[{"type":"audio","mimeType":"audio/wav","data":STANDARD.encode(wav)}]})).await;
    let mut audio = false;
    loop {
        let event = h.recv().await;
        let content = &event["params"]["update"]["content"];
        if content["type"] == "audio" {
            assert_eq!(content["mimeType"], "audio/wav");
            assert_eq!(
                STANDARD.decode(content["data"].as_str().unwrap()).unwrap(),
                expected_wav
            );
            audio = true;
        }
        if event["id"] == p {
            assert_eq!(event["result"]["stopReason"], "end_turn", "{event}");
            break;
        }
    }
    assert!(audio);
    server.await.unwrap();
}

#[tokio::test]
async fn realtime_disconnect_revokes_pending_permission_without_executing() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut ws = connect(listener).await;
        created(&mut ws, "r1").await;
        done(&mut ws, "r1", vec![call("c1")]).await;
        disconnect_rx.await.unwrap();
        ws.close(None).await.unwrap();
    });
    let mut h = Harness::spawn_with_env(&url, &options()).await;
    let sid = init(&mut h, dir.path()).await;
    let p = prompt(&mut h, &sid).await;
    let ask = h
        .recv_until(|v| v["method"] == "session/request_permission" || v["id"] == p)
        .await;
    assert_eq!(ask["method"], "session/request_permission");
    disconnect_tx.send(()).unwrap();
    let result = h.recv_until(|v| v["id"] == p).await;
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("disconnected"),
        "{result}"
    );
    // An approval delivered after disconnection must not revive the call.
    h.write(approve_permission(&ask)).await;
    let p = prompt(&mut h, &sid).await;
    let result = h.recv_until(|v| v["id"] == p).await;
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("session ended"));
    assert!(!dir.path().join("calls.log").exists());
    server.await.unwrap();
}
