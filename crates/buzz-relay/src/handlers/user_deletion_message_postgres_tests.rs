//! Exercise both transport entry points and the real message transaction.

use std::{sync::Arc, time::Duration};

use buzz_core::TenantContext;
use buzz_db::user_deletion_fences::{self as fences, FenceIdentity};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
use uuid::Uuid;

use super::group_state_postgres_tests::{community, create_channel, test_conn};
use crate::state::AppState;

struct Fixture {
    state: Arc<AppState>,
    pool: sqlx::PgPool,
    tenant: TenantContext,
    channel: Uuid,
    user: Keys,
    agent: Keys,
    peer: Keys,
    url: String,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .expect("Postgres and Redis");
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let tenant = community(&state, "message-deletion").await;
        let (user, agent, peer) = (Keys::generate(), Keys::generate(), Keys::generate());
        state
            .db
            .ensure_user(tenant.community(), user.public_key().as_bytes())
            .await
            .unwrap();
        state
            .db
            .ensure_user(tenant.community(), agent.public_key().as_bytes())
            .await
            .unwrap();
        state
            .db
            .set_agent_owner(
                tenant.community(),
                agent.public_key().as_bytes(),
                user.public_key().as_bytes(),
            )
            .await
            .unwrap();
        let channel = create_channel(
            &state,
            &tenant,
            &peer,
            &[
                &["p", &user.public_key().to_hex()],
                &["p", &agent.public_key().to_hex()],
            ],
        )
        .await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = crate::router::build_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            url,
            server,
            state,
            pool,
            tenant,
            channel,
            user,
            agent,
            peer,
        }
    }

    fn message(&self, key: &Keys, kind: u16, root: Option<&Event>) -> Event {
        let mut tags = vec![Tag::parse(["h", &self.channel.to_string()]).unwrap()];
        if let Some(root) = root {
            tags.push(Tag::parse(["e", &root.id.to_hex(), "", "root"]).unwrap());
            tags.push(Tag::parse(["e", &root.id.to_hex(), "", "reply"]).unwrap());
        }
        EventBuilder::new(Kind::Custom(kind), Uuid::new_v4().to_string())
            .tags(tags)
            .sign_with_keys(key)
            .unwrap()
    }

    async fn fence(&self) {
        let request = self
            .state
            .db
            .user_deletion_store()
            .submit(&format!("demo:{}", Uuid::new_v4()), "operator", None)
            .await
            .unwrap();
        let store = self.state.db.entity_deletion_store();
        store
            .approve(
                request.id,
                "reviewer",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
        let lease = store
            .claim(request.id, "worker", Duration::from_secs(300))
            .await
            .unwrap()
            .unwrap();
        let mut tx = self.pool.begin().await.unwrap();
        let user = FenceIdentity::user(self.user.public_key());
        let agent = FenceIdentity::agent(self.tenant.community(), self.agent.public_key());
        fences::lock_request(&mut tx, &lease).await.unwrap();
        fences::lock_exclusive(&mut tx, &[user, agent])
            .await
            .unwrap();
        fences::insert(&mut tx, user, &lease, None).await.unwrap();
        fences::insert(&mut tx, agent, &lease, Some(self.user.public_key()))
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn send(&self, key: &Keys, event: &Event, websocket: bool) -> (bool, String) {
        if websocket {
            let (conn, mut rx) = test_conn(&self.tenant, key);
            if let crate::connection::AuthState::Authenticated(ctx) =
                &mut *conn.auth_state.lock().unwrap()
            {
                ctx.scopes = vec![buzz_auth::Scope::MessagesWrite];
                // Intentionally no cached owner: the write must use PostgreSQL.
                ctx.agent_owner_pubkey = None;
            }
            super::event::handle_event(event.clone(), conn, self.state.clone()).await;
            let axum::extract::ws::Message::Text(text) = rx.try_recv().unwrap() else {
                panic!("expected OK");
            };
            let frame: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(frame[0], "OK", "{frame}");
            assert_eq!(frame[1], event.id.to_hex());
            (
                frame[2].as_bool().unwrap(),
                frame[3].as_str().unwrap().to_string(),
            )
        } else {
            let response = reqwest::Client::new()
                .post(format!("{}/events", self.url))
                .header("host", self.tenant.host())
                .header("x-pubkey", key.public_key().to_hex())
                .json(event)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body: Value = response.json().await.unwrap();
            if status.is_success() {
                (
                    body["accepted"].as_bool().unwrap(),
                    body["message"].as_str().unwrap_or("").to_owned(),
                )
            } else {
                assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");
                (false, body["error"].as_str().unwrap().to_owned())
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn http_and_websocket_messages_honor_user_and_agent_fences() {
    let f = Fixture::new().await;
    let root = f.message(&f.peer, 9, None);
    assert!(f.send(&f.peer, &root, false).await.0);
    for websocket in [false, true] {
        for key in [&f.user, &f.agent, &f.peer] {
            assert!(
                f.send(key, &f.message(key, 9, Some(&root)), websocket)
                    .await
                    .0
            );
        }
    }
    let counters: (i64, i64) = sqlx::query_as("SELECT reply_count::bigint, descendant_count::bigint FROM thread_metadata WHERE community_id = $1 AND event_id = $2").bind(f.tenant.community().as_uuid()).bind(root.id.as_bytes().as_slice()).fetch_one(&f.pool).await.unwrap();
    assert_eq!(counters, (6, 6), "fixture must create real replies");
    f.fence().await;
    for websocket in [false, true] {
        for key in [&f.user, &f.agent] {
            for kind in [9, 40002, 45001, 45003] {
                let event = f.message(key, kind, Some(&root));
                let (accepted, reason) = f.send(key, &event, websocket).await;
                assert!(!accepted, "fenced event accepted");
                assert_eq!(
                    reason,
                    "restricted: identity is restricted for user deletion"
                );
                assert!(f
                    .state
                    .db
                    .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                    .await
                    .unwrap()
                    .is_none());
            }
        }
    }
    let after: (i64, i64) = sqlx::query_as("SELECT reply_count::bigint, descendant_count::bigint FROM thread_metadata WHERE community_id = $1 AND event_id = $2").bind(f.tenant.community().as_uuid()).bind(root.id.as_bytes().as_slice()).fetch_one(&f.pool).await.unwrap();
    assert_eq!(after, counters, "rejected replies must not update counters");
    for websocket in [false, true] {
        assert!(
            f.send(&f.peer, &f.message(&f.peer, 9, Some(&root)), websocket)
                .await
                .0
        );
    }
}
