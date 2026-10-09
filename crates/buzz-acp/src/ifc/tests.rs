use super::*;
use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use nostr::{EventBuilder, Keys, Kind, Tag};

use crate::{
    acp::AcpError,
    pool::{self, ChannelInfoResolver, PromptOutcome, SessionState},
    queue::BatchEvent,
    relay::ChannelInfo,
};

fn event(keys: &Keys, kind: u32, content: &str, tags: Vec<Vec<String>>) -> Event {
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
        .sign_with_keys(keys)
        .unwrap()
}

struct Source {
    policy: Value,
    history: Value,
    policy_during_read: Option<Value>,
}

async fn query(State(state): State<Arc<Mutex<Source>>>, Json(filters): Json<Value>) -> Json<Value> {
    let mut source = state.lock().unwrap();
    let kinds = filters[0]["kinds"].as_array().unwrap();
    if kinds.contains(&json!(KIND_NIP29_GROUP_MEMBERS)) {
        assert_eq!(filters[0]["consistency"], "strong");
        Json(source.policy.clone())
    } else if kinds.contains(&json!(KIND_NIP29_GROUP_METADATA)) {
        Json(json!([source.policy[0]]))
    } else if kinds.contains(&json!(KIND_STREAM_MESSAGE)) {
        let history = source.history.clone();
        if let Some(policy) = source.policy_during_read.take() {
            source.policy = policy;
        }
        Json(history)
    } else {
        Json(json!([]))
    }
}

struct Fixture {
    config: ReadConfig,
    rest: RestClient,
    batch: FlushBatch,
    source: Arc<Mutex<Source>>,
    relay: Keys,
    owner: Keys,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let (agent, relay, owner) = (Keys::generate(), Keys::generate(), Keys::generate());
        let channel_id = Uuid::new_v4();
        let message = event(
            &owner,
            KIND_STREAM_MESSAGE,
            "history",
            vec![vec!["h".into(), channel_id.to_string()]],
        );
        let metadata = event(
            &relay,
            KIND_NIP29_GROUP_METADATA,
            "",
            vec![
                vec!["d".into(), channel_id.to_string()],
                vec!["t".into(), "dm".into()],
                vec!["private".into()],
            ],
        );
        let members = event(
            &relay,
            KIND_NIP29_GROUP_MEMBERS,
            "",
            vec![
                vec!["d".into(), channel_id.to_string()],
                vec!["p".into(), owner.public_key().to_hex()],
                vec!["p".into(), agent.public_key().to_hex()],
            ],
        );
        let source = Arc::new(Mutex::new(Source {
            policy: json!([metadata, members]),
            history: json!([message]),
            policy_during_read: None,
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/query", post(query))
            .with_state(source.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            config: ReadConfig {
                channel_id,
                community_id: Uuid::new_v4(),
                relay_pubkey: relay.public_key(),
            },
            rest: RestClient {
                http: reqwest::Client::new(),
                base_url,
                keys: agent,
                auth_tag_json: None,
            },
            batch: FlushBatch {
                channel_id,
                scope: SessionScope::Conversation { channel_id },
                events: vec![BatchEvent {
                    event: message,
                    prompt_tag: "test".into(),
                    received_at: std::time::Instant::now(),
                    edit: None,
                }],
                cancelled_events: vec![],
                cancel_reason: None,
            },
            source,
            relay,
            owner,
            server,
        }
    }

    fn changed_policy(&self, change_members: bool) -> Value {
        let source = self.source.lock().unwrap();
        let mut metadata: Event = serde_json::from_value(source.policy[0].clone()).unwrap();
        metadata
            .tags
            .push(Tag::parse(["about", "new topic"]).unwrap());
        let members: Event = serde_json::from_value(source.policy[1].clone()).unwrap();
        let mut member_tags: Vec<Tag> = members.tags.iter().rev().cloned().collect();
        if change_members {
            member_tags.push(Tag::parse(["p", &Keys::generate().public_key().to_hex()]).unwrap());
        }
        json!([
            EventBuilder::new(metadata.kind, "")
                .tags(metadata.tags)
                .sign_with_keys(&self.relay)
                .unwrap(),
            EventBuilder::new(members.kind, "reissued")
                .tags(member_tags)
                .sign_with_keys(&self.relay)
                .unwrap(),
        ])
    }
}

#[tokio::test]
async fn read_pins_domain_and_ignores_cosmetic_edits_and_member_order() {
    let f = Fixture::new().await;
    let mut state = SessionState::default();
    let owner = Some(f.owner.public_key());
    f.config
        .read_history(&f.rest, &f.batch, owner, &mut state.ifc_sessions, 10)
        .await
        .unwrap();
    let domain = state.ifc_sessions[&f.batch.scope].domain().clone();
    let edited = f.changed_policy(false);
    f.source.lock().unwrap().policy = edited;
    assert_eq!(
        f.config
            .read_history(&f.rest, &f.batch, owner, &mut state.ifc_sessions, 10)
            .await
            .unwrap(),
        json!([f.batch.events[0].event])
    );
    assert_eq!(state.ifc_sessions[&f.batch.scope].domain(), &domain);
    let changed = f.changed_policy(true);
    f.source.lock().unwrap().policy = changed;
    assert!(f
        .config
        .read_history(&f.rest, &f.batch, owner, &mut state.ifc_sessions, 10)
        .await
        .is_err());
    state.invalidate_channel(&f.config.channel_id);
    assert!(state.ifc_sessions.is_empty());
    f.config
        .read_history(&f.rest, &f.batch, owner, &mut state.ifc_sessions, 10)
        .await
        .unwrap();
    assert_ne!(state.ifc_sessions[&f.batch.scope].domain(), &domain);
    state.invalidate_all();
    assert!(state.ifc_sessions.is_empty());
}

#[tokio::test]
async fn policy_change_during_read_aborts_prompt_and_discards_both_sessions() {
    let f = Fixture::new().await;
    let changed = f.changed_policy(true);
    f.source.lock().unwrap().policy_during_read = Some(changed);
    let mut ctx = pool::tests::make_prompt_context_no_owner();
    ctx.rest_client = f.rest.clone();
    ctx.agent_keys = f.rest.keys.clone();
    ctx.context_message_limit = 10;
    ctx.dedup_mode = crate::config::DedupMode::Queue;
    ctx.ifc_read = Some(serde_json::from_value(json!({
        "channel_id": f.config.channel_id, "community_id": f.config.community_id, "relay_pubkey": f.config.relay_pubkey,
    })).unwrap());
    ctx.channel_info = ChannelInfoResolver::new(
        HashMap::from([(
            f.config.channel_id,
            ChannelInfo {
                name: "dm".into(),
                channel_type: "dm".into(),
                description: None,
            },
        )]),
        f.rest.clone(),
    );
    let agent = pool::tests::idle_agent_with_session(0, f.batch.scope.clone()).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        pool::run_prompt_task(
            agent,
            Some(f.batch.clone()),
            None,
            Arc::new(ctx),
            tx,
            None,
            "ifc-turn".into(),
            Default::default(),
        ),
    )
    .await
    .unwrap();
    let mut result = rx.recv().await.unwrap();
    assert!(matches!(
        result.outcome,
        PromptOutcome::Error(AcpError::IfcRead(_))
    ));
    assert_eq!(
        result.batch.unwrap().events[0].event.id,
        f.batch.events[0].event.id
    );
    assert!(result.agent.state.sessions.is_empty());
    assert!(result.agent.state.ifc_sessions.is_empty());
    result.agent.acp.shutdown().await;
}

#[tokio::test]
async fn invalid_signature_or_wrong_channel_never_returns_history() {
    for wrong_channel in [false, true] {
        let f = Fixture::new().await;
        let mut message = f.batch.events[0].event.clone();
        if wrong_channel {
            message = event(
                &f.owner,
                KIND_STREAM_MESSAGE,
                "other DM",
                vec![vec!["h".into(), Uuid::new_v4().to_string()]],
            );
        } else {
            message.content = "tampered".into();
        }
        f.source.lock().unwrap().history = json!([message]);
        assert!(f
            .config
            .read_history(
                &f.rest,
                &f.batch,
                Some(f.owner.public_key()),
                &mut HashMap::new(),
                10
            )
            .await
            .is_err());
    }
}
