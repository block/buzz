//! Shared fixtures for user-deletion transport regressions.

use std::{sync::Arc, time::Duration};

use buzz_core::TenantContext;
use buzz_db::user_deletion_fences::{self as fences, FenceIdentity};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
use uuid::Uuid;

use super::group_state_postgres_tests::{community, create_channel, test_conn};
use crate::state::AppState;

pub(super) struct Fixture {
    pub(super) state: Arc<AppState>,
    pub(super) pool: sqlx::PgPool,
    pub(super) tenant: TenantContext,
    pub(super) channel: Uuid,
    pub(super) user: Keys,
    pub(super) agent: Keys,
    pub(super) peer: Keys,
    url: String,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub(super) async fn new() -> Self {
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

    pub(super) fn message(&self, key: &Keys, kind: u16, root: Option<&Event>) -> Event {
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

    pub(super) async fn fence(&self) {
        self.pending_fence().await.commit().await.unwrap();
    }

    pub(super) async fn pending_fence(&self) -> sqlx::Transaction<'static, sqlx::Postgres> {
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
        tx
    }

    pub(super) async fn send(&self, key: &Keys, event: &Event, websocket: bool) -> (bool, String) {
        let (accepted, message, status) = self.submit(key, event, websocket).await;
        if let Some(status) = status.filter(|s| !s.is_success()) {
            assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{message}");
        }
        (accepted, message)
    }

    pub(super) async fn submit(
        &self,
        key: &Keys,
        event: &Event,
        websocket: bool,
    ) -> (bool, String, Option<axum::http::StatusCode>) {
        if websocket {
            let (conn, mut rx) = test_conn(&self.tenant, key);
            if let crate::connection::AuthState::Authenticated(ctx) =
                &mut *conn.auth_state.lock().unwrap()
            {
                ctx.scopes = buzz_auth::Scope::all_known();
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
                None,
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
                    Some(status),
                )
            } else {
                (
                    false,
                    body["error"].as_str().unwrap().to_owned(),
                    Some(status),
                )
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(super) fn action(
    key: &Keys,
    channel: Uuid,
    kind: u16,
    content: &str,
    extra: Vec<Vec<String>>,
) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    tags.extend(extra.into_iter().map(|tag| Tag::parse(tag).unwrap()));
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(key)
        .unwrap()
}

pub(super) async fn snapshot(f: &Fixture) -> Vec<Value> {
    let mut rows = Vec::new();
    for table in [
        "events",
        "channels",
        "channel_members",
        "reactions",
        "thread_metadata",
        "event_mentions",
        "operator_listener_outbox",
        "artifact_heads",
        "artifact_revisions",
        "workflows",
    ] {
        let query = format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb) FROM {table} t WHERE community_id=$1");
        rows.push(
            sqlx::query_scalar(sqlx::AssertSqlSafe(query))
                .bind(f.tenant.community().as_uuid())
                .fetch_one(&f.pool)
                .await
                .unwrap(),
        );
    }
    rows
}
