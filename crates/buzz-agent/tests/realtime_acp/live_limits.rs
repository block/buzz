use super::*;

#[tokio::test]
async fn live_audio_budget_is_two_minutes_per_reply_and_recovers_after_cutoff() {
    for advertised in [None, Some(48_000), Some(24_000 * 300)] {
        let samples = advertised.unwrap_or(24_000 * 120).min(24_000 * 120);
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            send(
                &mut ws,
                json!({"type":"session.created","session":{"id":"limits"}}),
            )
            .await;
            let mut update = recv(&mut ws).await;
            if let Some(limit) = advertised {
                update["session"]["frankie"] = json!({"max_output_audio_samples":limit});
            }
            send(
                &mut ws,
                json!({"type":"session.updated","session":update["session"]}),
            )
            .await;
            // Full budget twice proves ordinary replies reset it; a third reply
            // proves a cutoff does not poison the persistent conversation.
            for turn in 0..3 {
                assert_eq!(recv(&mut ws).await["type"], "input_audio_buffer.append");
                send(
                    &mut ws,
                    json!({"type":"input_audio_buffer.committed","item_id":format!("input{turn}")}),
                )
                .await;
                assert_eq!(recv(&mut ws).await["type"], "response.create");
                let id = format!("r{turn}");
                let item = format!("a{turn}");
                created(&mut ws, &id).await;
                // Below the 1 MiB wire frame cap; duration is a sample count,
                // so testing two minutes does not require sleeping two minutes.
                for _ in 0..8 {
                    send(&mut ws, json!({"type":"response.output_audio.delta","response_id":id,"item_id":item,"content_index":0,"delta":STANDARD.encode(vec![0u8;samples / 8 * 2])})).await;
                }
                if turn == 1 {
                    send(&mut ws, json!({"type":"response.output_audio.delta","response_id":id,"item_id":item,"content_index":0,"delta":"AAA="})).await;
                    let cancel = recv(&mut ws).await;
                    assert_eq!(cancel["type"], "response.cancel");
                    assert_eq!(cancel["response_id"], id);
                    // Completion may already be in flight when cancellation arrives.
                    let status = if advertised.is_some() {
                        "completed"
                    } else {
                        "cancelled"
                    };
                    send(&mut ws, json!({"type":"response.done","response":{"id":id,"status":status,"output":[call("discarded")]}})).await;
                    let truncate = recv(&mut ws).await;
                    assert_eq!(truncate["type"], "conversation.item.truncate");
                    assert_eq!(truncate["item_id"], item);
                    assert_eq!(truncate["audio_end_ms"], samples / 24);
                    send(&mut ws, json!({"type":"conversation.item.truncated","item_id":item,"content_index":0,"audio_end_ms":samples / 24})).await;
                } else {
                    done(&mut ws, &id, vec![]).await;
                }
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
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
        let stream = ready["params"]["streamId"].as_str().unwrap();
        for turn in 0..3 {
            h.send("_buzz/unstable/realtime/append", json!({"sessionId":sid,"streamId":stream,"sequence":turn,"data":STANDARD.encode(vec![0u8;960])})).await;
            let mut emitted = 0;
            let mut limited = false;
            let mut cleared = false;
            loop {
                let event = h.recv().await;
                assert_ne!(event["id"], prompt, "live session ended: {event}");
                assert_ne!(event["method"], "session/request_permission");
                let update = &event["params"]["update"];
                assert_ne!(update["sessionUpdate"], "tool_call");
                match update["type"].as_str() {
                    Some("audio") => {
                        assert_eq!(update["startSample"], emitted);
                        emitted += STANDARD
                            .decode(update["data"].as_str().unwrap())
                            .unwrap()
                            .len()
                            / 2;
                        assert!(emitted <= samples);
                    }
                    Some("clear") => {
                        assert_eq!(turn, 1);
                        cleared = true;
                        h.send("_buzz/unstable/realtime/playback", json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":format!("r{turn}"),"itemId":format!("a{turn}"),"contentIndex":0,"playedSamples":emitted,"stopped":true}})).await;
                    }
                    Some("response_limited") => {
                        assert_eq!(update["reason"], "max_output_audio");
                        limited = true;
                    }
                    Some("response_done") => break,
                    _ => {}
                }
            }
            assert_eq!(emitted, samples);
            assert_eq!(limited, turn == 1);
            assert_eq!(cleared, turn == 1);
            if turn != 1 {
                let i = h.send("_buzz/unstable/realtime/playback", json!({"sessionId":sid,"streamId":stream,"playback":{"responseId":format!("r{turn}"),"itemId":format!("a{turn}"),"contentIndex":0,"playedSamples":emitted}})).await;
                assert!(h.recv_until(|v| v["id"] == i).await.get("error").is_none());
            }
        }
        h.send(
            "_buzz/unstable/realtime/close",
            json!({"sessionId":sid,"streamId":stream}),
        )
        .await;
        assert_eq!(
            h.recv_until(|v| v["id"] == prompt).await["result"]["stopReason"],
            "end_turn"
        );
        server.await.unwrap();
        h.shutdown().await;
    }
}
