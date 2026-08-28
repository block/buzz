//! Real-storage regressions for workflow wake lifecycle boundaries.
use super::integration_tests::test_state_with_redis;
use super::*;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use buzz_core::{
    channel::{ChannelType, ChannelVisibility, MemberRole},
    tenant::CommunityId,
};
use buzz_db::CreateCommunityWithOwnerResult;
use nostr::{Event, Keys, Timestamp};

struct Fixture {
    state: Arc<AppState>,
    community: CommunityId,
    host: String,
    channel: Uuid,
    owner: Keys,
    agent: Keys,
    workflow: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let state = test_state_with_redis(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into()),
        )
        .await;
        let owner = Keys::generate();
        let agent = Keys::generate();
        let host = format!("wake-{}.example", Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &owner.public_key().to_hex())
            .await
            .expect("community")
        {
            CreateCommunityWithOwnerResult::Created(record) => record.id,
            other => panic!("unexpected {other:?}"),
        };
        state
            .db
            .ensure_user(community, &owner.public_key().to_bytes())
            .await
            .expect("owner user");
        let channel = state
            .db
            .create_channel(
                community,
                "wake",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &owner.public_key().to_bytes(),
                None,
            )
            .await
            .expect("channel")
            .id;
        state
            .db
            .ensure_user(community, &agent.public_key().to_bytes())
            .await
            .expect("agent");
        state
            .db
            .update_user_profile(
                community,
                &agent.public_key().to_bytes(),
                Some("Worker"),
                None,
                None,
                None,
            )
            .await
            .expect("name");
        state
            .db
            .add_member(
                community,
                channel,
                &agent.public_key().to_bytes(),
                MemberRole::Bot,
                Some(&owner.public_key().to_bytes()),
            )
            .await
            .expect("member");
        Self {
            state,
            community,
            host,
            channel,
            owner,
            agent,
            workflow: Uuid::new_v4(),
        }
    }
    fn connection(
        &self,
    ) -> (
        Arc<crate::connection::ConnectionState>,
        tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
    ) {
        use crate::connection::{AuthState, ConnectionState};
        use std::{collections::HashMap, sync::atomic::AtomicU8};
        use tokio::sync::{mpsc, Mutex, RwLock};
        let (send_tx, rx) = mpsc::channel(16);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(4);
        let conn = Arc::new(ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::TenantContext::resolved(self.community, &self.host),
            remote_addr: "127.0.0.1:1234".parse().expect("address"),
            auth_state: RwLock::new(AuthState::Authenticated(buzz_auth::AuthContext {
                pubkey: self.agent.public_key(),
                scopes: vec![],
                channel_ids: None,
                auth_method: buzz_auth::AuthMethod::Nip42,
                agent_owner_pubkey: None,
            })),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel: tokio_util::sync::CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        });
        self.state.conn_manager.register(
            conn.conn_id,
            conn.send_tx.clone(),
            conn.ctrl_tx.clone(),
            None,
            conn.cancel.clone(),
            self.community,
            conn.backpressure_count.clone(),
            conn.subscriptions.clone(),
            conn.grace_limit,
        );
        self.state
            .conn_manager
            .set_authenticated_pubkey(conn.conn_id, self.agent.public_key().to_bytes().to_vec());
        (conn, rx)
    }
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("host", self.host.parse().expect("host"));
        headers.insert(
            "x-pubkey",
            self.agent.public_key().to_hex().parse().expect("pubkey"),
        );
        headers
    }
    async fn revision(&self, timestamp: u64) -> Event {
        let definition = "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: '@Worker work'\n";
        let event = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_WORKFLOW_DEF as u16),
            definition,
        )
        .custom_created_at(Timestamp::from(timestamp))
        .tags([
            Tag::parse(["d", &self.workflow.to_string()]).expect("d"),
            Tag::parse(["h", &self.channel.to_string()]).expect("h"),
        ])
        .sign_with_keys(&self.owner)
        .expect("definition");
        let mut tx = self.state.db.begin_transaction().await.expect("tx");
        self.state
            .db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                self.community,
                &event,
                &self.workflow.to_string(),
                Some(self.channel),
                buzz_db::replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await
            .expect("replace");
        self.state
            .db
            .upsert_workflow(
                &mut tx,
                self.community,
                self.workflow,
                Some(self.channel),
                &self.owner.public_key().to_bytes(),
                "wake",
                "{}",
                &[0; 32],
                event.id.as_bytes(),
            )
            .await
            .expect("materialize");
        tx.commit().await.expect("commit");
        event
    }
    async fn authority(
        &self,
        run: Uuid,
        message: &str,
    ) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
        crate::api::workflows::workflow_wake_authority(
            State(self.state.clone()),
            Path((run, message.to_owned())),
            self.headers(),
        )
        .await
    }
}

fn next_frame(
    rx: &mut tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
) -> serde_json::Value {
    let axum::extract::ws::Message::Text(text) = rx.try_recv().expect("frame") else {
        panic!("expected text frame");
    };
    serde_json::from_str(&text).expect("frame JSON")
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn captured_revision_survives_replacement_but_not_revocation() {
    let f = Fixture::new().await;
    let a = f.revision(Timestamp::now().as_secs()).await;
    let run = f
        .state
        .db
        .create_workflow_run(f.community, f.workflow, Some(a.id.as_bytes()), None, None)
        .await
        .expect("run");
    let message = RelayActionSink::new(&f.state)
        .send_message(
            WorkflowMessageContext {
                community_id: f.community,
                run_id: run,
                step_id: "notify".into(),
                definition_event_id: Some(a.id.as_bytes().to_vec()),
            },
            &f.channel.to_string(),
            "@Worker work",
            &f.owner.public_key().to_hex(),
            None,
        )
        .await
        .expect("message");
    f.revision(a.created_at.as_secs() + 1).await;
    assert!(f
        .state
        .db
        .get_event_by_id(f.community, a.id.as_bytes())
        .await
        .expect("live read")
        .is_none());
    let authority = f
        .authority(run, &message)
        .await
        .expect("captured authority");
    assert_eq!(authority.0["definition"]["id"], a.id.to_hex());
    f.state
        .db
        .soft_delete_event_and_update_thread(f.community, a.id.as_bytes(), None, None)
        .await
        .expect("explicit revoke superseded revision");
    assert_eq!(
        f.authority(run, &message).await.expect_err("revoked").0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn removed_open_channel_member_cannot_read_or_count_wakes() {
    let f = Fixture::new().await;
    let a = f.revision(Timestamp::now().as_secs()).await;
    let run = f
        .state
        .db
        .create_workflow_run(f.community, f.workflow, Some(a.id.as_bytes()), None, None)
        .await
        .expect("run");
    let message = RelayActionSink::new(&f.state)
        .send_message(
            WorkflowMessageContext {
                community_id: f.community,
                run_id: run,
                step_id: "notify".into(),
                definition_event_id: Some(a.id.as_bytes().to_vec()),
            },
            &f.channel.to_string(),
            "@Worker work",
            &f.owner.public_key().to_hex(),
            None,
        )
        .await
        .expect("message");
    let _ = f.authority(run, &message).await.expect("member authority");
    let filter = serde_json::json!({"kinds":[buzz_core::kind::KIND_WORKFLOW_MENTION_WAKE], "#p":[f.agent.public_key().to_hex()], "#h":[f.channel.to_string()]});
    let body =
        axum::body::Bytes::from(serde_json::to_vec(&serde_json::json!([filter])).expect("body"));
    let before =
        crate::api::bridge::query_events(State(f.state.clone()), f.headers(), body.clone())
            .await
            .expect("query");
    assert_eq!(before.0.as_array().expect("events").len(), 1);
    // Exercise the actual WS handlers and live send path, retaining the same
    // connection/subscription across removal to expose stale access state.
    let (conn, mut frames) = f.connection();
    let filters: Vec<nostr::Filter> = serde_json::from_slice(&body).expect("filters");
    crate::handlers::req::handle_req(
        "wakes".into(),
        filters.clone(),
        conn.clone(),
        f.state.clone(),
    )
    .await;
    assert_eq!(next_frame(&mut frames)[0], "EVENT");
    assert_eq!(next_frame(&mut frames)[0], "EOSE");
    crate::handlers::count::handle_count(
        "count".into(),
        filters.clone(),
        conn.clone(),
        f.state.clone(),
    )
    .await;
    assert_eq!(next_frame(&mut frames)[2]["count"], 1);
    let wake: Event = serde_json::from_value(before.0[0].clone()).expect("wake");
    let stored = buzz_core::StoredEvent::new(wake, Some(f.channel));
    crate::handlers::event::fan_out_event_to_local_subscribers(&f.state, f.community, &stored)
        .await;
    assert_eq!(next_frame(&mut frames)[0], "EVENT");
    f.state
        .db
        .remove_member(
            f.community,
            f.channel,
            &f.agent.public_key().to_bytes(),
            &f.owner.public_key().to_bytes(),
        )
        .await
        .expect("remove");
    assert!(f
        .state
        .db
        .get_accessible_channel_ids(f.community, &f.agent.public_key().to_bytes())
        .await
        .expect("open readability")
        .contains(&f.channel));
    assert_eq!(
        f.authority(run, &message)
            .await
            .expect_err("not membership")
            .0,
        StatusCode::FORBIDDEN
    );
    let after = crate::api::bridge::query_events(State(f.state.clone()), f.headers(), body.clone())
        .await
        .expect("query after removal");
    assert!(after.0.as_array().expect("events").is_empty());
    let count = crate::api::bridge::count_events(State(f.state.clone()), f.headers(), body)
        .await
        .expect("count after removal");
    assert_eq!(count.0["count"], 0);
    crate::handlers::event::fan_out_event_to_local_subscribers(&f.state, f.community, &stored)
        .await;
    assert!(
        frames.try_recv().is_err(),
        "stale subscription must not deliver"
    );
    crate::handlers::req::handle_req(
        "wakes".into(),
        filters.clone(),
        conn.clone(),
        f.state.clone(),
    )
    .await;
    assert_eq!(next_frame(&mut frames)[0], "EOSE", "no historical EVENT");
    crate::handlers::count::handle_count("count".into(), filters, conn, f.state.clone()).await;
    assert_eq!(next_frame(&mut frames)[2]["count"], 0);
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn notification_failure_rolls_back_message_mentions_and_thread_metadata() {
    let f = Fixture::new().await;
    let message = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "@Worker work")
        .tags([
            Tag::parse(["h", &f.channel.to_string()]).expect("h"),
            Tag::public_key(f.agent.public_key()),
        ])
        .sign_with_keys(&f.state.relay_keypair)
        .expect("message");
    let wake = WorkflowMentionWake::new(
        f.agent.public_key(),
        f.channel,
        Uuid::new_v4(),
        message.id,
        message.id,
    )
    .sign(&f.state.relay_keypair)
    .expect("wake");
    // The second notification fails inside the transaction, after the message,
    // metadata, mentions and first recipient have been written.
    let rejected = EventBuilder::new(Kind::Custom(22242), "auth cannot persist")
        .sign_with_keys(&f.state.relay_keypair)
        .expect("rejected event");
    let meta = || buzz_db::event::ThreadMetadataParams {
        event_id: message.id.as_bytes(),
        event_created_at: chrono::DateTime::from_timestamp(message.created_at.as_secs() as i64, 0)
            .expect("ts"),
        channel_id: f.channel,
        parent_event_id: None,
        parent_event_created_at: None,
        root_event_id: None,
        root_event_created_at: None,
        depth: 0,
        broadcast: false,
    };
    assert!(f
        .state
        .db
        .insert_event_with_notifications(
            f.community,
            &message,
            f.channel,
            Some(meta()),
            &[wake.clone(), rejected]
        )
        .await
        .is_err());
    for event in [&message, &wake] {
        assert!(f
            .state
            .db
            .get_event_by_id(f.community, event.id.as_bytes())
            .await
            .expect("rollback read")
            .is_none());
    }
    assert!(f
        .state
        .db
        .get_thread_metadata_by_event(f.community, message.id.as_bytes())
        .await
        .expect("metadata rollback")
        .is_none());
    let mut tx = f.state.db.begin_transaction().await.expect("read mentions");
    let mentions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(message.id.as_bytes().as_slice())
    .fetch_one(&mut *tx)
    .await
    .expect("mentions");
    assert_eq!(mentions, 0);
    tx.rollback().await.expect("read rollback");
    // Commit without any fan-out: a new historical read still recovers all rows.
    let second = WorkflowMentionWake::new(
        f.owner.public_key(),
        f.channel,
        Uuid::new_v4(),
        message.id,
        message.id,
    )
    .sign(&f.state.relay_keypair)
    .expect("second wake");
    let rows = f
        .state
        .db
        .insert_event_with_notifications(
            f.community,
            &message,
            f.channel,
            Some(meta()),
            &[wake.clone(), second.clone()],
        )
        .await
        .expect("commit bundle");
    assert_eq!(rows.len(), 3);
    for event in [&message, &wake, &second] {
        assert!(f
            .state
            .db
            .get_event_by_id(f.community, event.id.as_bytes())
            .await
            .expect("replay read")
            .is_some());
    }
}
