//! Opt-in provider acceptance: two real turns around the production effort queue.
//! Requires BUZZ_TEST_CODEX_ADAPTER and an already-authenticated CODEX_HOME.
use super::*;
use crate::acp::AcpClient;
use crate::pool::{OwnedAgent, SessionState, TaskMeta};
use crate::scope::SessionScope;

#[tokio::test]
#[ignore = "requires authenticated Codex; performs two real provider turns"]
async fn real_codex_preserves_conversation_across_queued_effort_change() {
    let command = std::env::var("BUZZ_TEST_CODEX_ADAPTER").expect("set BUZZ_TEST_CODEX_ADAPTER");
    let cwd = tempfile::tempdir().unwrap();
    let mut acp = AcpClient::spawn(&command, &[], &[], false).await.unwrap();
    let wire_observer = ObserverHandle::in_process();
    acp.set_observer(Some(wire_observer.clone()), 0);
    let init = acp.initialize().await.unwrap();
    let session = acp
        .session_new_full(
            cwd.path().to_str().unwrap(),
            vec![],
            None,
            Some("Buzz live effort acceptance"),
        )
        .await
        .unwrap();
    let session_id = session.session_id;
    let model = std::env::var("BUZZ_TEST_CODEX_MODEL").unwrap_or_else(|_| "gpt-5.6-sol".into());
    let method = crate::acp::resolve_model_switch_method(&session.raw, &model)
        .expect("model advertised by the real adapter");
    let config = match method {
        crate::acp::ModelSwitchMethod::ConfigOption {
            config_id,
            option_value,
        } => {
            acp.session_set_config_option(&session_id, &config_id, &option_value)
                .await
        }
        crate::acp::ModelSwitchMethod::SetModel { model_id } => {
            acp.session_set_model(&session_id, &model_id).await
        }
    }
    .unwrap();
    let option = effort_option(&config).expect("native thought_level option");
    assert!(supports_value(&option["options"], "high"));
    let mut low = acp
        .session_set_config_option(&session_id, option["id"].as_str().unwrap(), "low")
        .await
        .unwrap();
    assert_eq!(effort_option(&low).unwrap()["currentValue"], "low");
    let session_token = Uuid::new_v4();
    low["effortSessionToken"] = json!(session_token);
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut state = SessionState::default();
    state.sessions.insert(scope.clone(), session_id.clone());
    state.configs.insert(scope.clone(), low);
    let mut worker = OwnedAgent {
        index: 0,
        acp,
        state,
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: Some("low".into()),
        agent_name: "codex".into(),
        goose_system_prompt_supported: None,
        protocol_version: 2,
    };
    let observer = ObserverHandle::in_process();
    let mut pool = AgentPool::from_slots(vec![None]);
    let nonce = Uuid::new_v4().to_string();
    let prompt = format!("Do not use any tools. Remember this verification word for the next turn: {nonce}. Reply with exactly REMEMBERED.");
    let first_session = session_id.clone();
    let running = tokio::spawn(async move {
        let response = worker
            .acp
            .session_prompt_with_idle_timeout(
                &first_session,
                &prompt,
                Duration::from_secs(90),
                Duration::from_secs(180),
            )
            .await
            .unwrap();
        (worker, response)
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !wire_observer
            .snapshot()
            .iter()
            .any(|event| event.kind == "acp_write" && event.payload["method"] == "session/prompt")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first provider prompt was written");
    assert!(
        !running.is_finished(),
        "queue while the real response is running"
    );
    let task_id = running.id();
    let (control_tx, mut control_rx) = tokio::sync::oneshot::channel();
    pool.task_map_mut().insert(
        task_id,
        TaskMeta {
            agent_index: 0,
            channel_id: Some(scope.channel_id()),
            scope: Some(scope.clone()),
            turn_id: "real-provider-turn".into(),
            recoverable_batch: None,
            control_tx: Some(control_tx),
            steer_tx: None,
            successful_steer_deliveries: Default::default(),
        },
    );
    let request = json!({"requestId":Uuid::new_v4(),"channelId":scope.channel_id(),"sessionId":session_id,"sessionToken":session_token,"effort":"high"});
    pool.queue_live_effort(&request, Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(
        observer.snapshot().last().unwrap().payload["status"],
        "queued"
    );
    assert!(matches!(
        control_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    let (worker, _) = running.await.unwrap();
    let first_text = response_text(&wire_observer, &session_id, 0);
    assert!(
        first_text.contains("REMEMBERED"),
        "unexpected first response: {first_text}"
    );
    pool.task_map_mut().remove(&task_id);
    pool.return_agent(worker);
    assert!(
        pool.try_claim(Some(&scope)).is_none(),
        "pending edit fences next dispatch"
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(
        observer.snapshot().last().unwrap().payload["status"],
        "applied"
    );
    let mut worker = pool.try_claim(Some(&scope)).unwrap();
    assert_eq!(worker.state.sessions[&scope], session_id);
    assert_eq!(
        effort_option(&worker.state.configs[&scope]).unwrap()["currentValue"],
        "high"
    );
    assert_eq!(worker.startup_effort.as_deref(), Some("low"));
    let second_start = wire_observer.snapshot().last().unwrap().seq + 1;
    let _ = worker.acp.session_prompt_with_idle_timeout(&session_id, "Do not use tools. Reply with only the verification word I gave you in the previous turn.", Duration::from_secs(90), Duration::from_secs(180)).await.unwrap();
    let second_text = response_text(&wire_observer, &session_id, second_start);
    assert!(
        second_text.contains(&nonce),
        "conversation memory lost: {second_text}"
    );
    worker.acp.shutdown().await;
    println!("REAL_CODEX_EFFORT_OK adapter={} model={} session={} low->high; busy queued; unchanged session; memory retained; startup default low", init["agentInfo"], model, session_id);
}

fn response_text(observer: &ObserverHandle, session_id: &str, start_seq: u64) -> String {
    observer
        .snapshot()
        .iter()
        .filter_map(|event| {
            let msg = &event.payload;
            (event.seq >= start_seq
                && event.kind == "acp_read"
                && msg["method"] == "session/update"
                && msg["params"]["sessionId"] == session_id
                && msg["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
                .then(|| msg["params"]["update"]["content"]["text"].as_str())
                .flatten()
        })
        .collect()
}
