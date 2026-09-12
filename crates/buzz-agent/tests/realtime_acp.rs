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
async fn modality_ack(ws: &mut Ws) {
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
    modality_ack(&mut ws).await;
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
    assert_eq!(
        init["result"]["agentCapabilities"]["promptCapabilities"]["image"],
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
async fn realtime_thinking_effort_is_sent_and_must_be_acknowledged() {
    for acknowledged in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"s1"}}),
            )
            .await;
            let update = recv(&mut ws).await;
            assert_eq!(update["session"]["reasoning"]["effort"], "low");
            let mut session = update["session"].clone();
            if !acknowledged {
                session.as_object_mut().unwrap().remove("reasoning");
            }
            send(&mut ws, json!({"type":"session.updated","session":session})).await;
            if acknowledged {
                modality_ack(&mut ws).await;
                assert_eq!(recv(&mut ws).await["type"], "conversation.item.create");
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                text(&mut ws, "r1").await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let mut env = options();
        env.push(("BUZZ_AGENT_THINKING_EFFORT", "low"));
        let mut harness = Harness::spawn_with_env(&url, &env).await;
        let sid = init(&mut harness, dir.path()).await;
        let prompt_id = prompt(&mut harness, &sid).await;
        let result = harness.recv_until(|v| v["id"] == prompt_id).await;
        if acknowledged {
            assert_eq!(result["result"]["stopReason"], "end_turn", "{result}");
        } else {
            assert!(
                result["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("acknowledge"),
                "{result}"
            );
        }
        assert!(!dir.path().join("calls.log").exists());
        server.await.unwrap();
    }
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
            modality_ack(&mut ws).await;
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
        for direction in ["input", "output"] {
            assert_eq!(
                update["session"]["audio"][direction]["format"],
                json!({"type":"audio/pcm","rate":24000})
            );
        }
        send(
            &mut ws,
            json!({"type":"session.updated","session":update["session"]}),
        )
        .await;
        modality_ack(&mut ws).await;
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

#[tokio::test]
async fn realtime_session_acknowledgment_is_not_an_echo_contract() {
    for (modality, detection, accepted) in [
        ("text", None, true),
        ("text", Some(Value::Null), true),
        ("audio", None, false),
        ("text", Some(json!({"type":"server_vad"})), false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"s1"}}),
            )
            .await;
            assert_eq!(recv(&mut ws).await["type"], "session.update");
            let mut session =
                json!({"type":"realtime","output_modalities":[modality],"audio":{"input":{}}});
            if let Some(detection) = detection {
                session["audio"]["input"]["turn_detection"] = detection;
            }
            send(&mut ws, json!({"type":"session.updated","session":session})).await;
            if accepted {
                modality_ack(&mut ws).await;
                assert_eq!(recv(&mut ws).await["type"], "conversation.item.create");
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                text(&mut ws, "r1").await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let mut h = Harness::spawn_with_env(&url, &options()).await;
        let sid = init(&mut h, dir.path()).await;
        let p = prompt(&mut h, &sid).await;
        let result = h.recv_until(|v| v["id"] == p).await;
        if accepted {
            assert_eq!(result["result"]["stopReason"], "end_turn", "{result}");
        } else {
            assert!(
                result["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("acknowledge manual session"),
                "{result}"
            );
        }
        assert!(!dir.path().join("calls.log").exists());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn realtime_rejection_reports_provider_reason_without_tool_effects() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut ws = connect(listener).await;
        send(&mut ws, json!({"type":"error", "error":{"code":"invalid_value","message":"Unsupported output voice"}})).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let mut h = Harness::spawn_with_env(&url, &options()).await;
    let sid = init(&mut h, dir.path()).await;
    let p = prompt(&mut h, &sid).await;
    let result = h.recv_until(|v| v["id"] == p).await;
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unsupported output voice"),
        "{result}"
    );
    assert!(!dir.path().join("calls.log").exists());
    server.await.unwrap();
}

// Protocol fixtures only: the provider does not perform image/audio inference.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_acp_image_forwarding_and_explicit_output() {
    for modality in ["text", "audio"] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let provider = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"s"}}),
            )
            .await;
            let update = recv(&mut ws).await;
            assert_eq!(update["session"]["output_modalities"], json!([modality]));
            send(
                &mut ws,
                json!({"type":"session.updated","session":update["session"]}),
            )
            .await;
            modality_ack(&mut ws).await;
            let item = recv(&mut ws).await;
            assert_eq!(item["type"], "conversation.item.create");
            assert_eq!(
                item["item"]["content"],
                json!([
                    {"type":"input_image","image_url":"data:image/png;base64,AQID"},
                    {"type":"input_text","text":"what is pictured?"}
                ])
            );
            assert_eq!(recv(&mut ws).await["type"], "response.create");
            if modality == "audio" {
                created(&mut ws, "r").await;
                send(&mut ws, json!({"type":"response.output_audio.delta","response_id":"r","item_id":"answer","content_index":0,"delta":STANDARD.encode(vec![0u8;4800])})).await;
                done(&mut ws, "r", vec![json!({"type":"message","id":"answer","role":"assistant","status":"completed","content":[{"type":"output_audio","transcript":"fixture"}]})]).await;
            } else {
                text(&mut ws, "r").await;
            }
        });
        let mut vars = options();
        vars.push(("OPENAI_COMPAT_BASE_URL", &url));
        vars.push(("BUZZ_AGENT_REALTIME_OUTPUT", modality));
        let mut h = Harness::spawn_with_env(&url, &vars).await;
        let sid = init(&mut h, dir.path()).await;
        let p = h
            .send(
                "session/prompt",
                json!({"sessionId":sid,"prompt":[
                    {"type":"image","mimeType":"image/png","data":"AQID"},
                    {"type":"text","text":"what is pictured?"}
                ]}),
            )
            .await;
        let mut heard = false;
        loop {
            let event = h.recv().await;
            heard |= event["params"]["update"]["content"]["type"] == "audio";
            if event["id"] == p {
                assert_eq!(event["result"]["stopReason"], "end_turn", "{event}");
                break;
            }
        }
        assert_eq!(heard, modality == "audio");
        provider.await.unwrap();
        h.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_invalid_image_fails_before_connect_and_keeps_session_usable() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let mut vars = options();
    vars.push(("OPENAI_COMPAT_BASE_URL", &url));
    let mut h = Harness::spawn_with_env(&url, &vars).await;
    let sid = init(&mut h, dir.path()).await;
    for (mime, data) in [
        ("image/png", "!".to_string()),
        ("image/svg+xml", "AQID".into()),
        ("image/png", STANDARD.encode(vec![1u8; 512 * 1024 + 1])),
    ] {
        let p = h
            .send(
                "session/prompt",
                json!({"sessionId":sid,"prompt":[{"type":"image","mimeType":mime,"data":data}]}),
            )
            .await;
        assert_eq!(
            h.recv_until(|v| v["id"] == p).await["error"]["code"],
            -32602
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }
    let provider = tokio::spawn(async move {
        let mut ws = connect(listener).await;
        text(&mut ws, "r").await;
    });
    let p = prompt(&mut h, &sid).await;
    assert_eq!(
        h.recv_until(|v| v["id"] == p).await["result"]["stopReason"],
        "end_turn"
    );
    provider.await.unwrap();
    h.shutdown().await;
}

#[tokio::test]
async fn realtime_acp_live_duplex_and_playback_fence() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        send(
            &mut ws,
            json!({"type":"session.created","session":{"id":"persistent"}}),
        )
        .await;
        let update = recv(&mut ws).await;
        assert_eq!(
            update["session"]["audio"]["input"]["turn_detection"],
            json!({"type":"server_vad","create_response":false,"interrupt_response":false})
        );
        send(
            &mut ws,
            json!({"type":"session.updated","session":update["session"]}),
        )
        .await;
        assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
        send(
            &mut ws,
            json!({"type":"input_audio_buffer.committed","item_id":"user1"}),
        )
        .await;
        assert_eq!(recv(&mut ws).await["type"], "response.create");
        created(&mut ws, "live1").await;
        send(&mut ws,json!({"type":"response.output_audio.delta","response_id":"live1","item_id":"audio1","content_index":0,"delta":STANDARD.encode(vec![0u8;4800])})).await;
        // The client can only send this after receiving PCM. The response is
        // deliberately still in flight: buffering until done deadlocks the test.
        assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
        send(&mut ws, json!({"type":"input_audio_buffer.speech_started"})).await;
        let cancel = recv(&mut ws).await;
        assert_eq!(cancel["type"], "response.cancel");
        assert_eq!(cancel["response_id"], "live1");
        send(&mut ws,json!({"type":"response.output_audio.delta","response_id":"live1","item_id":"audio1","content_index":0,"delta":STANDARD.encode(vec![1u8;4800])})).await;
        let truncate = recv(&mut ws).await;
        assert_eq!(truncate["type"], "conversation.item.truncate");
        assert_eq!(truncate["item_id"], "audio1");
        assert_eq!(truncate["audio_end_ms"], 25);
        send(
            &mut ws,
            json!({"type":"input_audio_buffer.committed","item_id":"user2"}),
        )
        .await;
        send(&mut ws,json!({"type":"response.done","response":{"id":"live1","status":"cancelled","output":[]}})).await;
        // No response may be created until the provider acknowledges truncation.
        assert!(tokio::time::timeout(Duration::from_millis(100), ws.next())
            .await
            .is_err());
        send(&mut ws,json!({"type":"conversation.item.truncated","item_id":"audio1","content_index":0,"audio_end_ms":25})).await;
        assert_eq!(recv(&mut ws).await["type"], "response.create");
        created(&mut ws, "live2").await;
        send(&mut ws,json!({"type":"response.output_audio.delta","response_id":"live2","item_id":"audio2","content_index":0,"delta":STANDARD.encode(vec![2u8;4800])})).await;
        done(&mut ws,"live2",vec![json!({"id":"audio2","type":"message","role":"assistant","status":"completed","content":[{"type":"output_audio","transcript":"second"}]})]).await;
        assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
        send(
            &mut ws,
            json!({"type":"input_audio_buffer.committed","item_id":"user3"}),
        )
        .await;
        assert_eq!(recv(&mut ws).await["type"], "response.create");
        created(&mut ws, "live3").await;
        // Ordering marker: the previous output has been retired, no new PCM yet.
        send(&mut ws, json!({"type":"input_audio_buffer.speech_stopped"})).await;
        assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
        send(&mut ws,json!({"type":"response.output_audio.delta","response_id":"live3","item_id":"audio3","content_index":0,"delta":STANDARD.encode(vec![2u8;4800])})).await;
        done(&mut ws,"live3",vec![json!({"id":"audio3","type":"message","role":"assistant","status":"completed","content":[{"type":"output_audio","transcript":"third"}]})]).await;
        // Keep the connection open until the ACP client explicitly closes it.
        let _ = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
    });
    let mut h = Harness::spawn_with_env(&url, &options()).await;
    let i = h.send("initialize",json!({"protocolVersion":1,"clientCapabilities":{"_meta":{"buzz":{"realtimeAudio":1}}}})).await;
    let initialized = h.recv_until(|v| v["id"] == i).await;
    assert_eq!(
        initialized["result"]["agentCapabilities"]["_meta"]["buzz"]["realtimeAudio"],
        1
    );
    let i = h
        .send("session/new", json!({"cwd":dir.path(),"mcpServers":[]}))
        .await;
    let sid = h.recv_until(|v| v["id"] == i).await["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let prompt = h
        .send(
            "session/prompt",
            json!({"sessionId":sid,"prompt":[],"_meta":{"buzz":{"realtimeAudio":1}}}),
        )
        .await;
    let ready = h
        .recv_until(|v| v["params"]["update"]["type"] == "ready" || v["id"] == prompt)
        .await;
    assert_eq!(ready["params"]["update"]["type"], "ready", "{ready}");
    let stream = ready["params"]["streamId"].as_str().unwrap();
    let append = json!({"sessionId":sid,"streamId":stream,"sequence":0,"data":STANDARD.encode(vec![0u8;960])});
    h.send("_buzz/unstable/realtime/append", append).await;
    let audio = h
        .recv_until(|v| v["params"]["update"]["type"] == "audio" || v["id"] == prompt)
        .await;
    assert_eq!(audio["params"]["update"]["responseId"], "live1");
    h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":1,"data":STANDARD.encode(vec![1u8;960])})).await;
    h.recv_until(|v| v["params"]["update"]["type"] == "clear")
        .await;
    // A position already in transit when clear arrives is not the stopped clock.
    // It must not close the truncation fence before the explicit worklet stop.
    let tick = h.send("_buzz/unstable/realtime/playback",json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":"live1","itemId":"audio1","contentIndex":0,"playedSamples":400}})).await;
    assert!(h
        .recv_until(|v| v["id"] == tick)
        .await
        .get("error")
        .is_none());
    h.send("_buzz/unstable/realtime/playback",json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":"live1","itemId":"audio1","contentIndex":0,"playedSamples":600,"stopped":true}})).await;
    let mut transcript = String::new();
    loop {
        let event = h.recv().await;
        if event["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
            transcript.push_str(
                event["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap(),
            );
        }
        assert_ne!(
            event["id"], prompt,
            "long-lived prompt ended before close: {event}"
        );
        if event["params"]["update"]["type"] == "audio" {
            assert_eq!(
                event["params"]["update"]["responseId"], "live2",
                "late interrupted PCM leaked"
            );
        }
        if event["params"]["update"]["type"] == "response_done"
            && event["params"]["update"]["responseId"] == "live2"
        {
            break;
        }
    }
    assert_eq!(
        transcript, "second",
        "completed live transcript was not forwarded"
    );
    let stale = h.send("_buzz/unstable/realtime/playback",json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":"live1","itemId":"audio1","contentIndex":0,"playedSamples":800}})).await;
    let rejected = h.recv_until(|v| v["id"] == stale).await;
    assert!(
        rejected.get("error").is_some(),
        "stale stopped playback was accepted: {rejected}"
    );
    let final_position = json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":"live2","itemId":"audio2","contentIndex":0,"playedSamples":2400}});
    let id = h
        .send("_buzz/unstable/realtime/playback", final_position.clone())
        .await;
    assert!(h.recv_until(|v| v["id"] == id).await.get("error").is_none());
    h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":2,"data":STANDARD.encode(vec![0u8;960])})).await;
    h.recv_until(|v| v["params"]["update"]["type"] == "speech_stopped")
        .await;
    let repeat = h
        .send("_buzz/unstable/realtime/playback", final_position.clone())
        .await;
    let ack = h.recv_until(|v| v["id"] == repeat).await;
    assert!(
        ack.get("error").is_none(),
        "final playback repeat rejected after retirement: {ack}"
    );
    let mut invalid = final_position.clone();
    invalid["playback"]["playedSamples"] = json!(2399);
    let id = h.send("_buzz/unstable/realtime/playback", invalid).await;
    assert!(h.recv_until(|v| v["id"] == id).await.get("error").is_some());
    h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":3,"data":STANDARD.encode(vec![0u8;960])})).await;
    h.recv_until(|v| {
        v["params"]["update"]["type"] == "response_done"
            && v["params"]["update"]["responseId"] == "live3"
    })
    .await;
    let repeat = h
        .send("_buzz/unstable/realtime/playback", final_position)
        .await;
    let ack = h.recv_until(|v| v["id"] == repeat).await;
    assert!(
        ack.get("error").is_none(),
        "final playback repeat rejected with newer PCM: {ack}"
    );
    let close = h
        .send(
            "_buzz/unstable/realtime/close",
            json!({"sessionId":sid,"streamId":stream}),
        )
        .await;
    h.recv_until(|v| v["id"] == close).await;
    let ended = h.recv_until(|v| v["id"] == prompt).await;
    assert_eq!(ended["result"]["stopReason"], "end_turn", "{ended}");
    server.await.unwrap();
}

#[tokio::test]
async fn realtime_live_tools_keep_capture_open_and_revoke_approval_on_speech() {
    for interrupt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let (speech_tx, speech_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"s"}}),
            )
            .await;
            let update = recv(&mut ws).await;
            send(
                &mut ws,
                json!({"type":"session.updated","session":update["session"]}),
            )
            .await;
            assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
            send(
                &mut ws,
                json!({"type":"input_audio_buffer.committed","item_id":"user1"}),
            )
            .await;
            assert_eq!(recv(&mut ws).await["type"], "response.create");
            created(&mut ws, "toolresponse").await;
            send(
                &mut ws,
                json!({"type":"response.output_audio.delta","response_id":"toolresponse",
                "item_id":"spoken","content_index":0,"delta":STANDARD.encode(vec![0;2400])}),
            )
            .await;
            done(&mut ws, "toolresponse", vec![
                json!({"id":"spoken","type":"message","role":"assistant","status":"completed","content":[]}),
                call("livecall")]).await;
            // Capture must reach the provider while approval is still pending.
            assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
            if interrupt {
                send(&mut ws, json!({"type":"input_audio_buffer.speech_started"})).await;
            }
            speech_tx.send(()).unwrap();
            let mut result = recv(&mut ws).await;
            if interrupt {
                let other = recv(&mut ws).await;
                let truncate = if result["type"] == "conversation.item.truncate" {
                    std::mem::replace(&mut result, other)
                } else {
                    other
                };
                assert_eq!(truncate["type"], "conversation.item.truncate");
                assert_eq!(truncate["item_id"], "spoken");
                assert_eq!(truncate["audio_end_ms"], 25);
                send(&mut ws, json!({"type":"conversation.item.truncated","item_id":"spoken","content_index":0,"audio_end_ms":25})).await;
            }
            assert_eq!(result["type"], "conversation.item.create");
            assert_eq!(result["item"]["call_id"], "livecall");
            if interrupt {
                assert!(result["item"]["output"]
                    .as_str()
                    .unwrap()
                    .contains("cancel"));
            } else {
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                created(&mut ws, "aftertool").await;
                done(&mut ws, "aftertool", vec![call("silentcall")]).await;
                let result = recv(&mut ws).await;
                assert_eq!(result["type"], "conversation.item.create");
                assert_eq!(result["item"]["call_id"], "silentcall");
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                created(&mut ws, "aftersilenttool").await;
                done(&mut ws, "aftersilenttool", vec![]).await;
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
        });
        let mut h = Harness::spawn_with_env(&url, &options()).await;
        let sid = init(&mut h, dir.path()).await;
        let i = h.send("initialize",json!({"protocolVersion":1,"clientCapabilities":{"_meta":{"buzz":{"realtimeAudio":1}}}})).await;
        h.recv_until(|v| v["id"] == i).await;
        let prompt = h
            .send(
                "session/prompt",
                json!({"sessionId":sid,"prompt":[],"_meta":{"buzz":{"realtimeAudio":1}}}),
            )
            .await;
        let ready = h
            .recv_until(|v| v["params"]["update"]["type"] == "ready" || v["id"] == prompt)
            .await;
        let stream = ready["params"]["streamId"].as_str().unwrap().to_owned();
        h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":0,"data":STANDARD.encode(vec![0;960])})).await;
        let ask = h
            .recv_until(|v| v["method"] == "session/request_permission" || v["id"] == prompt)
            .await;
        assert_eq!(ask["method"], "session/request_permission");
        assert!(!dir.path().join("calls.log").exists());
        let position = json!({"responseId":"toolresponse","itemId":"spoken","contentIndex":0,
            "playedSamples":if interrupt {240} else {1200}});
        let playback = h
            .send(
                "_buzz/unstable/realtime/playback",
                json!({"sessionId":sid,"streamId":stream,"playback":position}),
            )
            .await;
        let ack = h.recv_until(|v| v["id"] == playback).await;
        assert!(
            ack.get("error").is_none(),
            "playback during approval failed: {ack}"
        );
        h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":1,"data":STANDARD.encode(vec![0;960])})).await;
        speech_rx.await.unwrap();
        if interrupt {
            h.recv_until(|v| v["params"]["update"]["type"] == "speech_started")
                .await;
            h.send("_buzz/unstable/realtime/playback",json!({"sessionId":sid,"streamId":stream,
                "playback":{"responseId":"toolresponse","itemId":"spoken","contentIndex":0,"playedSamples":600,"stopped":true}})).await;
            h.recv_until(|v| {
                v["params"]["update"]["sessionUpdate"] == "tool_call_update"
                    && v["params"]["update"]["status"] == "failed"
            })
            .await;
        }
        h.write(approve_permission(&ask)).await;
        if !interrupt {
            let ask = h
                .recv_until(|v| v["method"] == "session/request_permission" || v["id"] == prompt)
                .await;
            assert_eq!(ask["method"], "session/request_permission");
            h.write(approve_permission(&ask)).await;
            h.recv_until(|v| {
                v["params"]["update"]["type"] == "response_done"
                    && v["params"]["update"]["responseId"] == "aftersilenttool"
            })
            .await;
            // An audio-less tool continuation must not erase the last speaker position.
            for (samples, valid) in [(1200, true), (1199, false), (1201, false)] {
                let id = h.send("_buzz/unstable/realtime/playback",json!({"sessionId":sid,"streamId":stream,
                    "playback":{"responseId":"toolresponse","itemId":"spoken","contentIndex":0,"playedSamples":samples}})).await;
                let ack = h.recv_until(|v| v["id"] == id).await;
                assert_eq!(
                    ack.get("error").is_none(),
                    valid,
                    "playback after silent tool continuation: {ack}"
                );
            }
        }
        let close = h
            .send(
                "_buzz/unstable/realtime/close",
                json!({"sessionId":sid,"streamId":stream}),
            )
            .await;
        h.recv_until(|v| v["id"] == close).await;
        h.recv_until(|v| v["id"] == prompt).await;
        let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap_or_default();
        assert_eq!(calls.lines().count(), if interrupt { 0 } else { 2 });
        server.await.unwrap();
    }
}

#[tokio::test]
async fn realtime_live_failure_preserves_bounded_provider_reason() {
    for failed_response in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"failure"}}),
            )
            .await;
            let update = recv(&mut ws).await;
            send(
                &mut ws,
                json!({"type":"session.updated","session":update["session"]}),
            )
            .await;
            assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
            let detail = json!({"code":"inference_error","message":format!("mouth\n priming failed {}TAIL", "x".repeat(600))});
            if failed_response {
                send(
                    &mut ws,
                    json!({"type":"input_audio_buffer.committed","item_id":"user"}),
                )
                .await;
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                created(&mut ws, "failed").await;
                send(&mut ws, json!({"type":"response.done","response":{"id":"failed","status":"failed","output":[],"status_details":{"type":"failed","error":detail}}})).await;
            } else {
                send(&mut ws, json!({"type":"error","error":detail})).await;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;
        });
        let mut h = Harness::spawn_with_env(&url, &options()).await;
        let i = h.send("initialize", json!({"protocolVersion":1,"clientCapabilities":{"_meta":{"buzz":{"realtimeAudio":1}}}})).await;
        h.recv_until(|v| v["id"] == i).await;
        let i = h
            .send("session/new", json!({"cwd":dir.path(),"mcpServers":[]}))
            .await;
        let sid = h.recv_until(|v| v["id"] == i).await["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        let prompt = h
            .send(
                "session/prompt",
                json!({"sessionId":sid,"prompt":[],"_meta":{"buzz":{"realtimeAudio":1}}}),
            )
            .await;
        let ready = h
            .recv_until(|v| v["params"]["update"]["type"] == "ready" || v["id"] == prompt)
            .await;
        assert_eq!(ready["params"]["update"]["type"], "ready", "{ready}");
        h.send("_buzz/unstable/realtime/append", json!({"sessionId":sid,"streamId":ready["params"]["streamId"],"sequence":0,"data":STANDARD.encode(vec![0u8;960])})).await;
        let result = h.recv_until(|v| v["id"] == prompt).await;
        let message = result["error"]["message"].as_str().unwrap();
        assert!(message.contains("mouth priming failed"), "{result}");
        assert!(
            !message.contains("TAIL") && !message.contains('\n') && message.len() < 650,
            "{result}"
        );
        server.await.unwrap();
        h.shutdown().await;
    }
}

#[tokio::test]
async fn realtime_acp_live_output_limits_preserve_session_without_tools() {
    for reason in ["max_output_tokens", "max_output_audio", "unknown_limit"] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let known = reason != "unknown_limit";
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"limits"}}),
            )
            .await;
            modality_ack(&mut ws).await;
            assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
            send(
                &mut ws,
                json!({"type":"input_audio_buffer.committed","item_id":"input1"}),
            )
            .await;
            assert_eq!(recv(&mut ws).await["type"], "response.create");
            created(&mut ws, "limited").await;
            send(&mut ws,json!({"type":"response.output_audio.delta","response_id":"limited","item_id":"audio_limit","content_index":0,"delta":STANDARD.encode(vec![0u8;4800])})).await;
            // Even a syntactically complete call in an incomplete response must not run.
            send(&mut ws,json!({"type":"response.done","response":{"id":"limited","status":"incomplete","status_details":{"type":"incomplete","reason":reason},"output":[call("discarded")]}})).await;
            if known {
                assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
                send(
                    &mut ws,
                    json!({"type":"input_audio_buffer.committed","item_id":"input2"}),
                )
                .await;
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                text(&mut ws, "recovered").await;
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
        });
        let mut h = Harness::spawn_with_env(&url, &options()).await;
        let i=h.send("initialize",json!({"protocolVersion":1,"clientCapabilities":{"_meta":{"buzz":{"realtimeAudio":1}}}})).await;
        h.recv_until(|v| v["id"] == i).await;
        let i = h
            .send("session/new", json!({"cwd":dir.path(),"mcpServers":[]}))
            .await;
        let sid = h.recv_until(|v| v["id"] == i).await["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        let prompt = h
            .send(
                "session/prompt",
                json!({"sessionId":sid,"prompt":[],"_meta":{"buzz":{"realtimeAudio":1}}}),
            )
            .await;
        let ready = h
            .recv_until(|v| v["params"]["update"]["type"] == "ready" || v["id"] == prompt)
            .await;
        let stream = ready["params"]["streamId"].as_str().unwrap();
        h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":0,"data":STANDARD.encode(vec![0u8;960])})).await;
        let mut notice = false;
        loop {
            let event = h.recv().await;
            assert_ne!(
                event["method"], "session/request_permission",
                "truncated tool reached permissions"
            );
            assert_ne!(event["params"]["update"]["sessionUpdate"], "tool_call");
            if event["params"]["update"]["type"] == "response_limited" {
                notice = true;
                assert_eq!(event["params"]["update"]["reason"], reason);
            }
            if event["id"] == prompt {
                assert!(!known, "bounded response killed live session: {event}");
                assert!(event.get("error").is_some());
                break;
            }
            if event["params"]["update"]["type"] == "response_done" {
                assert!(known && notice);
                break;
            }
        }
        if known {
            let id=h.send("_buzz/unstable/realtime/playback",json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":"limited","itemId":"audio_limit","contentIndex":0,"playedSamples":2400}})).await;
            assert!(h.recv_until(|v| v["id"] == id).await.get("error").is_none());
            h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":1,"data":STANDARD.encode(vec![0u8;960])})).await;
            let recovered = h
                .recv_until(|v| {
                    v["params"]["update"]["type"] == "response_done" || v["id"] == prompt
                })
                .await;
            assert_eq!(recovered["params"]["update"]["responseId"], "recovered");
            let close = h
                .send(
                    "_buzz/unstable/realtime/close",
                    json!({"sessionId":sid,"streamId":stream}),
                )
                .await;
            h.recv_until(|v| v["id"] == close).await;
            assert_eq!(
                h.recv_until(|v| v["id"] == prompt).await["result"]["stopReason"],
                "end_turn"
            );
        }
        server.await.unwrap();
        h.shutdown().await;
    }
}

#[tokio::test]
async fn realtime_duplex_capture_and_backchannels_preserve_turn_ownership() {
    for (supported, bad_sequence) in [(true, false), (false, false), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"duplex"}}),
            )
            .await;
            let update = recv(&mut ws).await;
            let mut session = update["session"].clone();
            if supported {
                session["frankie"] = json!({"duplex_audio":true,"backchannels":true});
            }
            send(&mut ws, json!({"type":"session.updated","session":session})).await;
            let input = recv(&mut ws).await;
            assert_eq!(input["type"], "input_audio_buffer.append");
            assert_eq!(
                STANDARD.decode(input["audio"].as_str().unwrap()).unwrap(),
                vec![1u8; 960]
            );
            send(&mut ws, json!({"type":"input_audio_buffer.cleared"})).await;
            if supported {
                assert_eq!(
                    STANDARD
                        .decode(input["playback_audio"].as_str().unwrap())
                        .unwrap(),
                    vec![2u8; 960]
                );
                send(&mut ws,json!({"type":"frankie.backchannel.delta","id":"aside","sequence":if bad_sequence {1} else {0},"delta":STANDARD.encode(vec![3u8;3840])})).await;
                if !bad_sequence {
                    send(
                        &mut ws,
                        json!({"type":"frankie.backchannel.done","id":"aside","samples":1920}),
                    )
                    .await;
                }
            }
            if !supported {
                assert!(input.get("playback_audio").is_none());
            }
            if !bad_sequence {
                // Listener audio must never cause response.create or a conversation item.
                assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
                send(
                    &mut ws,
                    json!({"type":"input_audio_buffer.committed","item_id":"user"}),
                )
                .await;
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                text(&mut ws, "reply").await;
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
        });
        let mut h = Harness::spawn_with_env(&url, &options()).await;
        let id = h.send("initialize",json!({"protocolVersion":1,"clientCapabilities":{"_meta":{"buzz":{"realtimeAudio":1}}}})).await;
        h.recv_until(|v| v["id"] == id).await;
        let id = h
            .send("session/new", json!({"cwd":dir.path(),"mcpServers":[]}))
            .await;
        let sid = h.recv_until(|v| v["id"] == id).await["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        let prompt = h
            .send(
                "session/prompt",
                json!({"sessionId":sid,"prompt":[],"_meta":{"buzz":{"realtimeAudio":1}}}),
            )
            .await;
        let ready = h
            .recv_until(|v| v["params"]["update"]["type"] == "ready" || v["id"] == prompt)
            .await;
        let stream = ready["params"]["streamId"].as_str().unwrap();
        let id=h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":0,"data":STANDARD.encode(vec![1u8;960]),"playbackData":STANDARD.encode(vec![2u8;958])})).await;
        assert!(h.recv_until(|v| v["id"] == id).await.get("error").is_some());
        h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":0,"data":STANDARD.encode(vec![1u8;960]),"playbackData":STANDARD.encode(vec![2u8;960])})).await;
        h.recv_until(|v| v["params"]["update"]["type"] == "input_cleared")
            .await;
        if bad_sequence {
            assert!(h
                .recv_until(|v| v["id"] == prompt)
                .await
                .get("error")
                .is_some());
        } else {
            if supported {
                let audio = h
                    .recv_until(|v| {
                        v["params"]["update"]["type"] == "backchannel_audio" || v["id"] == prompt
                    })
                    .await;
                assert_eq!(audio["params"]["update"]["id"], "aside");
                assert_eq!(
                    STANDARD
                        .decode(audio["params"]["update"]["data"].as_str().unwrap())
                        .unwrap(),
                    vec![3u8; 3840]
                );
                h.recv_until(|v| v["params"]["update"]["type"] == "backchannel_done")
                    .await;
            }
            h.send("_buzz/unstable/realtime/append",json!({"sessionId":sid,"streamId":stream,"sequence":1,"data":STANDARD.encode(vec![0u8;960])})).await;
            let reply = h
                .recv_until(|v| {
                    v["params"]["update"]["type"] == "response_done" || v["id"] == prompt
                })
                .await;
            assert_eq!(reply["params"]["update"]["responseId"], "reply");
            let close = h
                .send(
                    "_buzz/unstable/realtime/close",
                    json!({"sessionId":sid,"streamId":stream}),
                )
                .await;
            h.recv_until(|v| v["id"] == close).await;
            assert_eq!(
                h.recv_until(|v| v["id"] == prompt).await["result"]["stopReason"],
                "end_turn"
            );
        }
        server.await.unwrap();
        h.shutdown().await;
    }
}
