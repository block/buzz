use super::*;
use crate::acp::AcpClient;
use crate::pool::{OwnedAgent, SessionState, TaskMeta};
use crate::scope::SessionScope;

fn config(value: &str) -> Value {
    json!({"effortSessionToken":"fbf7259f-74df-4859-8b53-c0d8b77fb21e","configOptions":[{"id":"native-reasoning","category":"thought_level","type":"select",
        "currentValue":value,"options":[{"value":"low"},{"value":"high"}]}],"relayUrl":"ws://test.invalid"})
}

async fn agent(scope: &SessionScope, file: &std::path::Path, behavior: &str) -> OwnedAgent {
    let script = r#"import json,sys,time
for line in sys.stdin:
 r=json.loads(line)
 with open(sys.argv[1],'a') as f: f.write(json.dumps(r)+'\n')
 if sys.argv[2]=='hang': time.sleep(60)
 if sys.argv[2]=='reject':
  print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':{'code':-32602,'message':'unsupported effort'}}),flush=True)
 else:
  value=r['params']['value'] if sys.argv[2]=='apply' else 'low'
  opts=[{'id':'native-reasoning','category':'thought_level','type':'select','currentValue':value,'options':[{'value':'low'},{'value':'high'}]}]
  print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'configOptions':opts}}),flush=True)
"#;
    let acp = AcpClient::spawn(
        "python3",
        &[
            "-u".into(),
            "-c".into(),
            script.into(),
            file.to_string_lossy().to_string(),
            behavior.into(),
        ],
        &[],
        false,
    )
    .await
    .unwrap();
    let mut state = SessionState::default();
    state.sessions.insert(scope.clone(), "same-session".into());
    state.configs.insert(scope.clone(), config("low"));
    state.turn_counts.insert(scope.clone(), 3);
    OwnedAgent {
        index: 0,
        acp,
        state,
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: Some("low".into()),
        agent_name: "effort-test".into(),
        goose_system_prompt_supported: None,
        protocol_version: 2,
    }
}

fn request(channel: Uuid, session: &str, effort: &str) -> Value {
    json!({"type":"switch_effort","requestId":Uuid::new_v4(),"channelId":channel,
        "sessionId":session,"sessionToken":"fbf7259f-74df-4859-8b53-c0d8b77fb21e","effort":effort})
}

fn status(observer: &ObserverHandle) -> String {
    observer
        .snapshot()
        .iter()
        .rev()
        .find(|e| e.kind == "control_result")
        .unwrap()
        .payload["status"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn applies_native_rpc_to_same_session_and_preserves_sibling_defaults_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let channel = Uuid::new_v4();
    let scope = SessionScope::Thread {
        channel_id: channel,
        root_event_id: "a".repeat(64),
    };
    let sibling = SessionScope::Thread {
        channel_id: channel,
        root_event_id: "b".repeat(64),
    };
    let mut worker = agent(&scope, &file, "apply").await;
    worker
        .state
        .sessions
        .insert(sibling.clone(), "sibling-session".into());
    worker.state.configs.insert(sibling.clone(), config("low"));
    let mut pool = AgentPool::from_slots(vec![Some(worker)]);
    let observer = ObserverHandle::in_process();
    let pick = request(channel, "same-session", "high");
    pool.queue_live_effort(&pick, Some(&observer));
    assert_eq!(status(&observer), "queued");
    assert!(
        pool.try_claim(Some(&scope)).is_none(),
        "next turn must wait for effort"
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "applied");
    let worker = pool.try_claim(Some(&scope)).unwrap();
    assert_eq!(worker.state.sessions[&scope], "same-session");
    assert_eq!(worker.state.turn_counts[&scope], 3);
    assert_eq!(worker.startup_effort.as_deref(), Some("low"));
    assert_eq!(
        worker.state.configs[&scope]["configOptions"][0]["currentValue"],
        "high"
    );
    assert_eq!(
        worker.state.configs[&sibling]["configOptions"][0]["currentValue"],
        "low"
    );
    let captured = observer
        .snapshot()
        .into_iter()
        .find(|e| e.kind == "session_config_captured")
        .unwrap();
    assert_eq!(captured.session_id.as_deref(), Some("same-session"));
    let rpc: Value = serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
    assert_eq!(rpc["method"], "session/set_config_option");
    assert_eq!(
        rpc["params"],
        json!({"sessionId":"same-session","configId":"native-reasoning","value":"high"})
    );
    pool.return_agent(worker);
    pool.queue_live_effort(&pick, Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(
        std::fs::read_to_string(file).unwrap().lines().count(),
        1,
        "replay cannot execute twice"
    );
}

#[tokio::test]
async fn busy_turn_queues_without_cancelling_then_applies_before_next_claim() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let channel = Uuid::new_v4();
    let scope = SessionScope::Conversation {
        channel_id: channel,
    };
    let worker = agent(&scope, &file, "apply").await;
    let mut pool = AgentPool::from_slots(vec![None]);
    let observer = ObserverHandle::in_process();
    let task = tokio::spawn(std::future::pending::<()>());
    let id = task.id();
    let (control_tx, mut control_rx) = tokio::sync::oneshot::channel();
    pool.task_map_mut().insert(
        id,
        TaskMeta {
            agent_index: 0,
            channel_id: Some(channel),
            scope: Some(scope.clone()),
            turn_id: "busy".into(),
            recoverable_batch: None,
            control_tx: Some(control_tx),
            steer_tx: None,
            successful_steer_deliveries: Default::default(),
        },
    );
    pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert!(
        !file.exists(),
        "must not write while a response owns the stream"
    );
    assert!(
        matches!(
            control_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "must not cancel the response"
    );
    pool.task_map_mut().remove(&id);
    task.abort();
    pool.return_agent(worker);
    assert!(pool.try_claim(Some(&scope)).is_none());
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "applied");
    assert!(pool.try_claim(Some(&scope)).is_some());
}

#[tokio::test]
async fn rejects_unsupported_and_stale_targets_without_any_rpc() {
    for (session, effort, expected) in [
        ("same-session", "ultra", "unsupported"),
        ("expired-session", "high", "stale_session"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rpc");
        let channel = Uuid::new_v4();
        let scope = SessionScope::Conversation {
            channel_id: channel,
        };
        let mut pool = AgentPool::from_slots(vec![Some(agent(&scope, &file, "apply").await)]);
        let observer = ObserverHandle::in_process();
        pool.queue_live_effort(&request(channel, session, effort), Some(&observer));
        pool.apply_pending_effort(Some(&observer)).await;
        assert_eq!(status(&observer), expected);
        assert!(!file.exists());
        assert_eq!(
            pool.agents_mut()[0].as_ref().unwrap().state.configs[&scope],
            config("low")
        );
    }
}

#[tokio::test]
async fn adapter_rejection_preserves_session_and_mismatched_ack_is_unconfirmed() {
    for (behavior, expected) in [("reject", "rejected"), ("mismatch", "unconfirmed")] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rpc");
        let channel = Uuid::new_v4();
        let scope = SessionScope::Conversation {
            channel_id: channel,
        };
        let mut pool = AgentPool::from_slots(vec![Some(agent(&scope, &file, behavior).await)]);
        let observer = ObserverHandle::in_process();
        pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
        pool.apply_pending_effort(Some(&observer)).await;
        assert_eq!(status(&observer), expected);
        let worker = pool.try_claim(Some(&scope)).unwrap();
        assert_eq!(worker.state.sessions[&scope], "same-session");
        assert_eq!(
            worker.state.configs[&scope]["configOptions"][0]["currentValue"],
            "low"
        );
    }
}

#[tokio::test]
async fn bounds_pending_controls_and_releases_expired_claim_fences() {
    let mut pool = AgentPool::from_slots(vec![]);
    let observer = ObserverHandle::in_process();
    for _ in 0..CAPACITY {
        pool.queue_live_effort(&request(Uuid::new_v4(), "session", "high"), Some(&observer));
    }
    pool.queue_live_effort(
        &request(Uuid::new_v4(), "overflow", "high"),
        Some(&observer),
    );
    assert_eq!(status(&observer), "busy");
    assert_eq!(pool.live_effort.pending.len(), CAPACITY);
    for item in &mut pool.live_effort.pending {
        item.received = Instant::now() - EXPIRY - Duration::from_secs(1);
    }
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "expired");
    assert!(pool.live_effort.pending.is_empty());
}

#[tokio::test]
async fn timeout_retires_the_stream_without_claiming_application() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let channel = Uuid::new_v4();
    let scope = SessionScope::Conversation {
        channel_id: channel,
    };
    let mut pool = AgentPool::from_slots(vec![Some(agent(&scope, &file, "hang").await)]);
    let observer = ObserverHandle::in_process();
    pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "unconfirmed");
    assert!(pool.agents_mut()[0].is_none());
}

#[tokio::test]
async fn pending_edit_holds_exact_owner_without_blocking_another_idle_worker() {
    let dir = tempfile::tempdir().unwrap();
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let sibling = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let owner = agent(&scope, &dir.path().join("owner"), "apply").await;
    let mut other = agent(&sibling, &dir.path().join("sibling"), "apply").await;
    other.index = 1;
    let mut pool = AgentPool::from_slots(vec![Some(owner), Some(other)]);
    pool.queue_live_effort(&request(scope.channel_id(), "same-session", "high"), None);
    assert!(
        pool.try_claim(Some(&scope)).is_none(),
        "do not fall through and duplicate the held session on another worker"
    );
    assert_eq!(pool.try_claim(Some(&sibling)).unwrap().index, 1);
    pool.apply_pending_effort(None).await;
    assert_eq!(pool.try_claim(Some(&scope)).unwrap().index, 0);
    assert!(
        !dir.path().join("sibling").exists(),
        "never mutate the sibling adapter"
    );
}

#[tokio::test]
async fn duplicate_adapter_session_ids_do_not_select_an_arbitrary_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let channel = Uuid::new_v4();
    let first = SessionScope::Thread {
        channel_id: channel,
        root_event_id: "a".repeat(64),
    };
    let second = SessionScope::Thread {
        channel_id: channel,
        root_event_id: "b".repeat(64),
    };
    let owner = agent(&first, &dir.path().join("first"), "apply").await;
    let mut other = agent(&second, &dir.path().join("second"), "apply").await;
    other.index = 1;
    let mut pool = AgentPool::from_slots(vec![Some(owner), Some(other)]);
    let observer = ObserverHandle::in_process();
    pool.queue_live_effort(&request(channel, "same-session", "high"), Some(&observer));
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "unavailable");
    assert!(!dir.path().join("first").exists());
    assert!(!dir.path().join("second").exists());
    assert!(pool.try_claim(Some(&first)).is_some());
    assert!(pool.try_claim(Some(&second)).is_some());
}

#[tokio::test]
async fn encrypted_owner_route_rejects_foreign_stale_and_tampered_effort_controls() {
    let owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let outsider = nostr::Keys::generate();
    for case in ["owner", "foreign", "stale", "tampered"] {
        let sender = if case == "foreign" { &outsider } else { &owner };
        let pick = request(Uuid::new_v4(), "session", "high");
        let encrypted =
            crate::encrypt_observer_payload(sender, &agent.public_key(), &pick).unwrap();
        let mut builder = buzz_sdk::build_agent_observer_frame(
            &agent.public_key().to_hex(),
            &agent.public_key().to_hex(),
            "control",
            &encrypted,
        )
        .unwrap();
        if case == "stale" {
            builder = builder.custom_created_at(nostr::Timestamp::from(
                nostr::Timestamp::now().as_secs() - 301,
            ));
        }
        let mut event = builder.sign_with_keys(sender).unwrap();
        if case == "tampered" {
            event.content.push('x');
        }
        let observer = ObserverHandle::in_process();
        let mut pool = AgentPool::from_slots(vec![]);
        let (publisher, _rx) = crate::relay::RelayEventPublisher::test_pair();
        crate::handle_relay_observer_control_event(
            &agent,
            event,
            &mut pool,
            Some(&observer),
            &owner.public_key().to_hex(),
            publisher,
        );
        assert_eq!(
            pool.live_effort.pending.len(),
            usize::from(case == "owner"),
            "{case}"
        );
        if case == "owner" {
            assert_eq!(status(&observer), "queued");
        } else {
            assert!(observer.snapshot().is_empty(), "{case}");
        }
    }
}

#[tokio::test]
async fn busy_duplicate_session_cannot_make_an_idle_sibling_look_unique() {
    let dir = tempfile::tempdir().unwrap();
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let sibling = SessionScope::Thread {
        channel_id: scope.channel_id(),
        root_event_id: "a".repeat(64),
    };
    let owner = agent(&scope, &dir.path().join("owner"), "apply").await;
    let mut busy = agent(&sibling, &dir.path().join("busy"), "apply").await;
    busy.index = 1;
    let mut pool = AgentPool::from_slots(vec![Some(owner), None]);
    let task = tokio::spawn(std::future::pending::<()>());
    pool.task_map_mut().insert(
        task.id(),
        TaskMeta {
            agent_index: 1,
            channel_id: Some(scope.channel_id()),
            scope: Some(sibling),
            turn_id: "busy".into(),
            recoverable_batch: None,
            control_tx: None,
            steer_tx: None,
            successful_steer_deliveries: Default::default(),
        },
    );
    let observer = ObserverHandle::in_process();
    pool.queue_live_effort(
        &request(scope.channel_id(), "same-session", "high"),
        Some(&observer),
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "queued");
    assert!(!dir.path().join("owner").exists());
    pool.task_map_mut().remove(&task.id());
    task.abort();
    pool.return_agent(busy);
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "unavailable");
    assert!(!dir.path().join("owner").exists());
    assert!(!dir.path().join("busy").exists());
}

#[tokio::test]
async fn native_config_id_and_grouped_values_use_the_reported_wire_shape() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut worker = agent(&scope, &file, "apply").await;
    let option = &mut worker.state.configs.get_mut(&scope).unwrap()["configOptions"][0];
    option.as_object_mut().unwrap().remove("id");
    option["configId"] = json!("spec-reasoning");
    option["options"] = json!([{"name":"Levels", "options":[{"value":"high"}]}]);
    let mut pool = AgentPool::from_slots(vec![Some(worker)]);
    let observer = ObserverHandle::in_process();
    pool.queue_live_effort(
        &request(scope.channel_id(), "same-session", "high"),
        Some(&observer),
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "applied");
    let rpc: Value = serde_json::from_str(
        std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(rpc["params"]["configId"], "spec-reasoning");
}

#[test]
fn configuration_snapshots_follow_every_session_invalidation_boundary() {
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    for boundary in ["scope", "channel", "all"] {
        let mut state = SessionState::default();
        state.sessions.insert(scope.clone(), "session".into());
        state.configs.insert(scope.clone(), config("high"));
        match boundary {
            "scope" => {
                state.invalidate_scope(&scope);
            }
            "channel" => {
                state.invalidate_channel(&scope.channel_id());
            }
            _ => state.invalidate_all(),
        }
        assert!(state.configs.is_empty(), "{boundary}");
        assert!(state.sessions.is_empty(), "{boundary}");
    }
}

#[test]
fn native_snapshot_capacity_keeps_existing_targets_and_reopens_after_invalidation() {
    let mut state = SessionState::default();
    for _ in 0..CONFIG_CAPACITY {
        state.remember_effort_config(
            &SessionScope::Conversation {
                channel_id: Uuid::new_v4(),
            },
            &mut config("low"),
        );
    }
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut snapshot = config("low");
    state.remember_effort_config(&scope, &mut snapshot);
    assert_eq!(snapshot["liveEffortSwitching"], false);
    assert!(!state.configs.contains_key(&scope));
    let retained = state.configs.keys().next().unwrap().clone();
    state.remember_effort_config(&retained, &mut config("high"));
    assert_eq!(
        state.configs[&retained]["configOptions"][0]["currentValue"],
        "high"
    );
    state.invalidate_scope(&retained);
    state.remember_effort_config(&scope, &mut snapshot);
    assert_eq!(snapshot["liveEffortSwitching"], true);
    assert_eq!(state.configs.len(), CONFIG_CAPACITY);
    snapshot["oversized"] = json!("x".repeat(MAX_CONFIG_BYTES));
    state.remember_effort_config(&scope, &mut snapshot);
    assert_eq!(snapshot["liveEffortSwitching"], false);
    assert!(!state.configs.contains_key(&scope));
}

#[tokio::test]
async fn a_reused_adapter_session_id_cannot_receive_an_old_conversations_edit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rpc");
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut worker = agent(&scope, &file, "apply").await;
    worker.state.configs.get_mut(&scope).unwrap()["effortSessionToken"] = json!(Uuid::new_v4());
    let mut pool = AgentPool::from_slots(vec![Some(worker)]);
    let observer = ObserverHandle::in_process();
    pool.queue_live_effort(
        &request(scope.channel_id(), "same-session", "high"),
        Some(&observer),
    );
    pool.apply_pending_effort(Some(&observer)).await;
    assert_eq!(status(&observer), "stale_session");
    assert!(!file.exists());
    assert!(pool.try_claim(Some(&scope)).is_some());
}
