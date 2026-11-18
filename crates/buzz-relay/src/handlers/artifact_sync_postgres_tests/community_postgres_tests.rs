use super::*;
use crate::connection::AuthState;
use crate::handlers::ingest::{HttpAuthMethod, IngestAuth};
use crate::handlers::{artifact_sync, close, event as event_handler};
use buzz_auth::Scope;

async fn community_fixture() -> Fixture {
    let mut state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let config = Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config);
    config.community_artifacts_enabled = true;
    config.require_relay_membership = true;
    let host = format!("community-sync-{}.example", Uuid::new_v4());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let mut conn = state.db.pool().acquire().await.unwrap();
    sqlx::query(
        "SET buzz.community_artifact_contract_ready='all-backends-enforce-community-audience'",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../scripts/maintenance/community-artifact-contract.sql"
    ))
    .execute(&mut *conn)
    .await
    .unwrap();
    drop(conn);
    tokio::spawn(Arc::clone(&state.pubsub).run_subscriber());
    state
        .pubsub
        .link_state()
        .wait_for(|link| link.up)
        .await
        .unwrap();
    Fixture {
        state,
        tenant: TenantContext::resolved(community, &host),
    }
}
async fn member(f: &Fixture, keys: &Keys, role: &str) {
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,$3)")
        .bind(f.tenant.community().as_uuid())
        .bind(keys.public_key().to_hex())
        .bind(role)
        .execute(f.state.db.pool())
        .await
        .unwrap();
}
fn auth(keys: &Keys) -> IngestAuth {
    IngestAuth::Http {
        pubkey: keys.public_key(),
        scopes: vec![Scope::MessagesWrite, Scope::MessagesRead, Scope::AdminUsers],
        auth_method: HttpAuthMethod::Nip98,
    }
}
fn signed_revision(f: &Fixture, keys: &Keys, d: Uuid, prev: Option<&Event>) -> Event {
    let mut tags = vec![
        vec!["ar".into(), "1".into()],
        vec!["community".into(), f.tenant.community().to_string()],
        vec!["type".into(), "example.settings".into()],
        vec!["d".into(), d.to_string()],
        vec!["title".into(), "Settings".into()],
        vec![
            "op".into(),
            if prev.is_some() {
                "update".into()
            } else {
                "create".into()
            },
        ],
    ];
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    EventBuilder::new(Kind::Custom(45010), "private snapshot")
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(keys)
        .unwrap()
}
async fn revision(f: &Fixture, keys: &Keys, d: Uuid, prev: Option<&Event>) -> Event {
    let event = signed_revision(f, keys, d, prev);
    assert!(
        ingest_event(&f.state, &f.tenant, event.clone(), auth(keys))
            .await
            .unwrap()
            .accepted
    );
    event
}
fn connect(f: &Fixture, keys: &Keys) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
    let (conn, rx) = super::connect(f, keys);
    if let AuthState::Authenticated(ctx) = &mut *conn.auth_state.lock().unwrap() {
        ctx.scopes = vec![Scope::MessagesRead, Scope::MessagesWrite];
    }
    (conn, rx)
}
fn filter() -> Value {
    json!({"artifact":"current","scope":"community","#type":["example.settings"]})
}
async fn remove(f: &Fixture, keys: &Keys) {
    sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
        .bind(f.tenant.community().as_uuid())
        .bind(keys.public_key().to_hex())
        .execute(f.state.db.pool())
        .await
        .unwrap();
}
fn named_sync(
    f: &Fixture,
    conn: &Arc<ConnectionState>,
    sub: &str,
    filter: Value,
) -> tokio::task::JoinHandle<()> {
    let sub = sub.to_owned();
    let filter = buzz_core::artifact_sync::parse_sync(&filter);
    let permit = f
        .state
        .handler_semaphore
        .clone()
        .try_acquire_owned()
        .unwrap();
    let (conn, state) = (Arc::clone(conn), Arc::clone(&f.state));
    tokio::spawn(async move {
        let owner = close::admit(&sub, &conn).await.unwrap();
        artifact_sync::handle_sync(sub, owner, filter, conn, state, permit).await;
    })
}
async fn ended(task: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn community_signal_lag_preserves_channel_sync() {
    use buzz_pubsub::artifact_signal::{ArtifactChange, ArtifactSignal};
    use tokio::sync::{broadcast::error::TryRecvError, oneshot};

    let f = community_fixture().await;
    let owner = Keys::generate();
    member(&f, &owner, "owner").await;
    let home = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let (community, mut community_rx) = connect(&f, &owner);
    let (channel, mut channel_rx) = connect(&f, &owner);
    let community_task = start_sync(&f, &community, filter());
    let channel_task = start_sync(&f, &channel, sync_filter(&[home]));
    assert!(initial_set(&mut community_rx).await.is_empty());
    assert!(initial_set(&mut channel_rx).await.is_empty());

    // Subscribe now and start polling on a separate runtime only after Redis
    // fills the receiver. This exercises actual lag without a test hook.
    let (ready, ready_rx) = oneshot::channel();
    let (resume, resume_rx) = oneshot::channel();
    let cancel = f.state.community_revalidator_cancel.clone();
    let _shutdown = cancel.clone().drop_guard();
    let state = Arc::clone(&f.state);
    let worker = tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let signals = {
            let _enter = runtime.enter();
            artifact_sync::spawn_signals(state)
        };
        ready.send(()).unwrap();
        if resume_rx.blocking_recv().is_ok() {
            runtime.block_on(signals).unwrap();
        }
    });
    ready_rx.await.unwrap();

    let mut probe = f.state.pubsub.subscribe_artifacts();
    let signal = ArtifactSignal {
        d: Uuid::new_v4(),
        artifact_type: "example.other".into(),
        cause: ArtifactChange::Changed,
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        // The local artifact broadcast holds 4096 messages.
        for _ in 0..=4096 {
            f.state
                .pubsub
                .publish_artifact(&f.tenant, &signal)
                .await
                .unwrap();
        }
        while probe.len() <= 4096 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("local artifact signal receiver overflowed");
    assert!(matches!(probe.try_recv(), Err(TryRecvError::Lagged(_))));
    resume.send(()).unwrap();

    assert_eq!(
        frame(&mut community_rx).await,
        json!(["CLOSED", "s", "resync: live delivery lagged"])
    );
    ended(community_task).await;
    // Let the receiver finish handling lag before checking the channel sync.
    cancel.cancel();
    worker.await.unwrap();

    let created = super::revision(&f, &owner, home, Uuid::new_v4(), "create", None).await;
    assert_eq!(live_event(&mut channel_rx).await, created.id.to_hex());
    close::handle_close("s".into(), channel, Arc::clone(&f.state)).await;
    ended(channel_task).await;
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn socket_revocation_or_local_policy_change_stops_initial_snapshot() {
    for change in 2..6 {
        let mut f = community_fixture().await;
        Arc::make_mut(&mut Arc::get_mut(&mut f.state).unwrap().config)
            .nip_fi
            .mode = buzz_auth::NipFiMode::Shadow;
        let keys = Keys::generate();
        let owner = Keys::generate();
        if change >= 3 {
            Arc::get_mut(&mut f.state).unwrap().nip_fi_deny_map =
                Some(Arc::new(buzz_auth::NipFiDenyMap::new(
                    8,
                    vec![buzz_auth::IssuerCapacity {
                        issuer: "test-issuer".into(),
                        capacity: 8,
                    }],
                )));
        }
        member(&f, &keys, "member").await;
        revision(&f, &keys, Uuid::new_v4(), None).await;
        let (mut conn, mut rx) = connect(&f, &keys);
        if change >= 3 {
            Arc::get_mut(&mut conn).unwrap().nip_fi_assertion = Some(
                buzz_auth::VerifiedAssertion::for_test(
                    Some(keys.public_key()),
                    vec![chrono::Utc::now() + chrono::Duration::hours(1)],
                )
                .with_owner_for_test(owner.public_key()),
            );
        }
        let token = close::admit("s", &conn).await.unwrap();
        let pause = Arc::new(close::test_seam::Pause::default());
        let task = tokio::spawn(
            artifact_sync::test_seam::QUEUE_CHECK_PAUSE.scope(
                Arc::clone(&pause),
                artifact_sync::handle_sync(
                    "s".into(),
                    token,
                    buzz_core::artifact_sync::parse_sync(&filter()),
                    Arc::clone(&conn),
                    Arc::clone(&f.state),
                    f.state
                        .handler_semaphore
                        .clone()
                        .try_acquire_owned()
                        .unwrap(),
                ),
            ),
        );
        tokio::time::timeout(Duration::from_secs(10), pause.reached.notified())
            .await
            .unwrap();
        match change {
            2 => {
                sqlx::query("UPDATE communities SET requires_nip_fi=true,policy_generation=policy_generation+1 WHERE id=$1")
                    .bind(f.tenant.community().as_uuid()).execute(f.state.db.pool()).await.unwrap();
                f.state.invalidate_community_host(&f.tenant);
            }
            3 | 4 => {
                let key = if change == 3 {
                    keys.public_key()
                } else {
                    owner.public_key()
                };
                assert_eq!(
                    f.state
                        .nip_fi_deny_map
                        .as_ref()
                        .unwrap()
                        .merge_cross_pod_deny(
                            "test-issuer",
                            &key,
                            chrono::Utc::now() + chrono::Duration::hours(1),
                            chrono::Utc::now(),
                        ),
                    buzz_auth::CrossPodMergeResult::Merged
                );
            }
            _ => conn.nip_fi_gate.expire(|| {}).await,
        }
        pause.resume.notify_one();
        if change != 5 {
            let response = frame(&mut rx).await;
            assert_eq!(response[0], "CLOSED", "checkpoint {change}");
            assert!(response[2].as_str().unwrap().starts_with("resync:"));
        }
        ended(task).await;
        no_frame(&mut rx).await;
        if change == 2 {
            let reopened = start_sync(&f, &conn, filter());
            let response = frame(&mut rx).await;
            assert_eq!(response[0], "CLOSED");
            assert!(response[2].as_str().unwrap().starts_with("restricted:"));
            ended(reopened).await;
            no_frame(&mut rx).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn live_head_admission_denial_requires_resync() {
    let f = community_fixture().await;
    let signals = artifact_sync::spawn_signals(Arc::clone(&f.state));
    let writer = Keys::generate();
    let reader = Keys::generate();
    member(&f, &writer, "owner").await;
    member(&f, &reader, "member").await;
    let d = Uuid::new_v4();
    let first = revision(&f, &writer, d, None).await;
    let (conn, mut rx) = connect(&f, &reader);
    let task = start_sync(&f, &conn, filter());
    assert_eq!(initial_set(&mut rx).await, vec![first.id.to_hex()]);

    // Omit the invalidation so the next head read discovers the revocation.
    remove(&f, &reader).await;
    revision(&f, &writer, d, Some(&first)).await;
    assert_eq!(
        frame(&mut rx).await,
        json!(["CLOSED", "s", "resync: community access changed"])
    );
    ended(task).await;
    assert!(rx.try_recv().is_err());
    signals.abort();
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn community_sync_rejects_unauthenticated_readless_and_channel_bound_credentials() {
    let f = community_fixture().await;
    let keys = Keys::generate();
    member(&f, &keys, "member").await;
    for case in 0..3 {
        let (conn, mut rx) = connect(&f, &keys);
        {
            let mut state = conn.auth_state.lock().unwrap();
            if case == 0 {
                *state = AuthState::Failed;
            } else if let AuthState::Authenticated(ctx) = &mut *state {
                if case == 1 {
                    ctx.scopes = vec![Scope::MessagesWrite];
                } else {
                    ctx.channel_ids = Some(vec![]);
                }
            }
        }
        let task = start_sync(&f, &conn, filter());
        assert_eq!(frame(&mut rx).await[0], "CLOSED");
        ended(task).await;
        no_frame(&mut rx).await;
    }
    let first = revision(&f, &keys, Uuid::new_v4(), None).await;
    let restricted = IngestAuth::Nip42 {
        pubkey: keys.public_key(),
        scopes: vec![Scope::MessagesWrite],
        channel_ids: Some(vec![]),
        conn_id: Uuid::new_v4(),
    };
    assert!(ingest_event(&f.state, &f.tenant, first, restricted)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn two_nodes_resync_matching_community_artifacts_on_redaction() {
    let f = community_fixture().await;
    let local_fanout = artifact_sync::spawn_signals(Arc::clone(&f.state));
    let mut other = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let config = Arc::make_mut(&mut Arc::get_mut(&mut other).unwrap().config);
    config.community_artifacts_enabled = true;
    config.require_relay_membership = true;
    tokio::spawn(Arc::clone(&other.pubsub).run_subscriber());
    other
        .pubsub
        .link_state()
        .wait_for(|link| link.up)
        .await
        .unwrap();
    let remote = Fixture {
        state: other,
        tenant: f.tenant.clone(),
    };
    let remote_fanout = artifact_sync::spawn_signals(Arc::clone(&remote.state));
    // Retain the ordinary topic independently: community payloads must never
    // enter its unchanged ChannelEvent receiver on either relay.
    let mut ordinary_a = f.state.pubsub.subscribe_local();
    let mut ordinary_b = remote.state.pubsub.subscribe_local();
    for node in [&f, &remote] {
        node.state
            .pubsub
            .retain_topic(&node.tenant, buzz_pubsub::EventTopic::Global)
            .await;
        node.state
            .pubsub
            .confirm_topic(&node.tenant, buzz_pubsub::EventTopic::Global)
            .await
            .unwrap();
    }
    let owner = Keys::generate();
    let reader = Keys::generate();
    member(&f, &owner, "owner").await;
    member(&f, &reader, "member").await;
    let first = revision(&f, &owner, Uuid::new_v4(), None).await;
    let (a, mut arx) = connect(&f, &reader);
    let (b, mut brx) = connect(&remote, &reader);
    let at = start_sync(&f, &a, filter());
    let bt = start_sync(&remote, &b, filter());
    assert_eq!(initial_set(&mut arx).await, vec![first.id.to_hex()]);
    assert_eq!(initial_set(&mut brx).await, vec![first.id.to_hex()]);
    let current = revision(
        &f,
        &owner,
        first.tags.identifier().unwrap().parse().unwrap(),
        Some(&first),
    )
    .await;
    for rx in [&mut arx, &mut brx] {
        let message = frame(rx).await;
        assert_eq!(message[0], "EVENT");
        assert_eq!(message[2]["id"], current.id.to_hex());
    }
    let unrelated = json!({"artifact":"current","scope":"community","#type":["example.other"]});
    let other_task = named_sync(&remote, &b, "other", unrelated);
    assert!(initial_set(&mut brx).await.is_empty());
    // An equally named identity on a different Redis tenant cannot reach us.
    let wrong = TenantContext::resolved(
        buzz_core::CommunityId::from_uuid(Uuid::new_v4()),
        "other.example",
    );
    f.state
        .pubsub
        .publish_artifact(
            &wrong,
            &buzz_pubsub::artifact_signal::ArtifactSignal {
                d: first.tags.identifier().unwrap().parse().unwrap(),
                artifact_type: "example.settings".into(),
                cause: buzz_pubsub::artifact_signal::ArtifactChange::Redacted,
            },
        )
        .await
        .unwrap();
    for rx in [&mut arx, &mut brx] {
        no_frame(rx).await;
    }
    let historical = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["community", &f.tenant.community().to_string()]).unwrap(),
            Tag::event(first.id),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    ingest_event(&f.state, &f.tenant, historical, auth(&owner))
        .await
        .unwrap();
    for rx in [&mut arx, &mut brx] {
        no_frame(rx).await;
    }
    assert!(ordinary_a.try_recv().is_err());
    assert!(ordinary_b.try_recv().is_err());
    let ordinary = EventBuilder::new(Kind::TextNote, "ordinary")
        .sign_with_keys(&owner)
        .unwrap();
    f.state
        .pubsub
        .publish_event(&f.tenant, buzz_pubsub::EventTopic::Global, &ordinary)
        .await
        .unwrap();
    for rx in [&mut ordinary_a, &mut ordinary_b] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
                .event
                .id,
            ordinary.id
        );
        assert!(rx.try_recv().is_err());
    }
    let moderation = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["community", &f.tenant.community().to_string()]).unwrap(),
            Tag::event(current.id),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(
        ingest_event(&f.state, &f.tenant, moderation, auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    for rx in [&mut arx, &mut brx] {
        let message = frame(rx).await;
        assert_eq!(message[0], "CLOSED");
        assert!(message[2].as_str().unwrap().starts_with("resync:"));
    }
    ended(at).await;
    ended(bt).await;
    no_frame(&mut brx).await;
    assert!(remote.state.artifact_syncs.contains(b.conn_id, "other"));
    close::handle_close("other".into(), Arc::clone(&b), Arc::clone(&remote.state)).await;
    assert_eq!(frame(&mut brx).await[0], "CLOSED");
    ended(other_task).await;
    for node in [&f, &remote] {
        assert_eq!(
            node.state
                .pubsub
                .topic_refcount(&node.tenant, buzz_pubsub::EventTopic::CommunityArtifact)
                .await,
            0
        );
    }
    local_fanout.abort();
    remote_fanout.abort();
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn community_sync_retries_after_snapshot_capacity_is_exhausted() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

    async fn read_frame(
        client: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match client.next().await {
                    Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    other => panic!("expected a relay frame, got {other:?}"),
                }
            }
        })
        .await
        .expect("relay frame in time")
    }

    let mut f = community_fixture().await;
    let keys = Keys::generate();
    member(&f, &keys, "member").await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    // A two-connection writer leaves one connection available for admission.
    Arc::get_mut(&mut f.state).unwrap().db = buzz_db::Db::from_pool(pool);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::router::build_router(Arc::clone(&f.state));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut request = format!("ws://{address}/").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", f.tenant.host().parse().unwrap());
    let (mut client, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let auth_challenge = read_frame(&mut client).await;
    assert_eq!(auth_challenge[0], "AUTH");
    let relay = crate::api::bridge::nip42_expected_relay_url(&f.state.config.relay_url, &f.tenant);
    let auth = EventBuilder::new(Kind::Authentication, "")
        .tags([
            Tag::parse(["relay", &relay]).unwrap(),
            Tag::parse(["challenge", auth_challenge[1].as_str().unwrap()]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    client
        .send(Message::Text(json!(["AUTH", auth]).to_string().into()))
        .await
        .unwrap();
    let auth_response = read_frame(&mut client).await;
    assert_eq!(auth_response[0], "OK", "{auth_response}");
    assert_eq!(auth_response[2], true, "{auth_response}");

    let principal = buzz_db::community_artifact::CommunityArtifactPrincipal {
        pubkey: keys.public_key().to_bytes().to_vec(),
        verified_owner: None,
        owner_admission: false,
        require_membership: true,
        reject_required_nip_fi: false,
    };
    let held = f
        .state
        .db
        .open_community_artifact_snapshot(f.tenant.community(), &principal)
        .await
        .unwrap();
    client
        .send(Message::Text(
            json!(["REQ", "busy", filter()]).to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        read_frame(&mut client).await,
        json!(["CLOSED", "busy", "error: sync read failed"])
    );

    held.finish().await.unwrap();
    client
        .send(Message::Text(
            json!(["REQ", "retry", filter()]).to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(read_frame(&mut client).await, json!(["EOSE", "retry"]));
    client.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn unavailable_current_admission_has_no_snapshot_or_eose() {
    let f = community_fixture().await;
    let keys = Keys::generate();
    member(&f, &keys, "member").await;
    revision(&f, &keys, Uuid::new_v4(), None).await;
    let (conn, mut rx) = connect(&f, &keys);
    // Simulate an unavailable authorization relation on this isolated DB.
    sqlx::query("ALTER TABLE community_bans RENAME TO probe_unavailable_bans")
        .execute(f.state.db.pool())
        .await
        .unwrap();
    let task = start_sync(&f, &conn, filter());
    let response = frame(&mut rx).await;
    assert_eq!(response[0], "CLOSED");
    ended(task).await;
    assert!(
        rx.try_recv().is_err(),
        "no EVENT or EOSE after failed admission"
    );
}

/// Direct AUTH may record an OA owner while OA admission is disabled. That
/// owner still restricts reads/writes, but only enabled OA or verified FI may
/// supply a missing roster entry. Exercise actual signed AUTH and EVENT.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn owner_restrictions_and_roster_fallback_remain_separate_after_direct_auth() {
    for mode in 0..3 {
        let mut f = community_fixture().await;
        Arc::make_mut(&mut Arc::get_mut(&mut f.state).unwrap().config).allow_nip_oa_auth =
            mode == 1;
        let owner = Keys::generate();
        let agent = Keys::generate();
        member(&f, &owner, "owner").await;
        member(&f, &agent, "member").await;
        let (mut conn, mut rx) = connect(&f, &agent);
        if mode == 2 {
            let inner = Arc::get_mut(&mut conn).unwrap();
            inner.nip_fi_assertion = Some(
                buzz_auth::VerifiedAssertion::for_test(
                    Some(agent.public_key()),
                    vec![chrono::Utc::now() + chrono::Duration::hours(1)],
                )
                .with_owner_for_test(owner.public_key()),
            );
        }
        *conn.auth_state.lock().unwrap() = AuthState::Pending {
            challenge: "owner-context".into(),
            started_at: std::time::Instant::now(),
        };
        let proof: Vec<String> = serde_json::from_str(
            &buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap(),
        )
        .unwrap();
        let auth = EventBuilder::new(Kind::Authentication, "")
            .tags([
                Tag::parse([
                    "relay",
                    &crate::api::bridge::nip42_expected_relay_url(
                        &f.state.config.relay_url,
                        &f.tenant,
                    ),
                ])
                .unwrap(),
                Tag::parse(["challenge", "owner-context"]).unwrap(),
                Tag::parse(proof).unwrap(),
            ])
            .sign_with_keys(&agent)
            .unwrap();
        crate::handlers::auth::handle_auth(auth.clone(), Arc::clone(&conn), Arc::clone(&f.state))
            .await;
        let reply = frame(&mut rx).await;
        assert_eq!(reply[2], true, "mode {mode}: AUTH: {reply}");
        let AuthState::Authenticated(ctx) = conn.auth_state_snapshot() else {
            panic!("AUTH failed")
        };
        assert_eq!(ctx.agent_owner_pubkey, Some(owner.public_key()));
        let first = signed_revision(&f, &agent, Uuid::new_v4(), None);
        event_handler::handle_event(first.clone(), Arc::clone(&conn), Arc::clone(&f.state)).await;
        assert_eq!(frame(&mut rx).await[2], true);
        let sync = start_sync(&f, &conn, filter());
        assert_eq!(initial_set(&mut rx).await, vec![first.id.to_hex()]);
        // The same key and OA proof on a second socket have no FI credential.
        // Its live admission must be checked independently of the FI-backed socket.
        let oa_only = if mode == 2 {
            let (other, mut other_rx) = connect(&f, &agent);
            *other.auth_state.lock().unwrap() = AuthState::Pending {
                challenge: "owner-context".into(),
                started_at: std::time::Instant::now(),
            };
            crate::handlers::auth::handle_auth(auth, Arc::clone(&other), Arc::clone(&f.state))
                .await;
            assert_eq!(frame(&mut other_rx).await[2], true);
            let task = start_sync(&f, &other, filter());
            assert_eq!(initial_set(&mut other_rx).await, vec![first.id.to_hex()]);
            Some((other, other_rx, task))
        } else {
            None
        };
        let signal = buzz_pubsub::artifact_signal::ArtifactSignal {
            d: first.tags.identifier().unwrap().parse().unwrap(),
            artifact_type: "example.settings".into(),
            cause: buzz_pubsub::artifact_signal::ArtifactChange::Changed,
        };
        remove(&f, &agent).await; // Deliberately omit disconnects.
        f.state
            .artifact_syncs
            .signal_community(f.tenant.community(), &signal);
        if let Some((other, mut other_rx, task)) = oa_only {
            assert_eq!(frame(&mut other_rx).await[0], "CLOSED");
            ended(task).await;
            event_handler::handle_event(
                signed_revision(&f, &agent, Uuid::new_v4(), None),
                other,
                Arc::clone(&f.state),
            )
            .await;
            assert_eq!(frame(&mut other_rx).await[2], false);
        }
        if mode == 0 {
            assert_eq!(frame(&mut rx).await[0], "CLOSED");
            ended(sync).await;
        } else {
            let live = frame(&mut rx).await;
            assert_eq!(live[0], "EVENT");
            assert_eq!(live[2]["id"], first.id.to_hex());
            close::handle_close("s".into(), Arc::clone(&conn), Arc::clone(&f.state)).await;
            assert_eq!(frame(&mut rx).await[0], "CLOSED");
            ended(sync).await;
        }
        let next = signed_revision(&f, &agent, Uuid::new_v4(), None);
        event_handler::handle_event(next, Arc::clone(&conn), Arc::clone(&f.state)).await;
        assert_eq!(
            frame(&mut rx).await[2],
            mode != 0,
            "mode {mode}: fallback write"
        );
        if mode == 0 {
            member(&f, &agent, "member").await;
        }
        sqlx::query("INSERT INTO community_bans(community_id,pubkey,actor_pubkey,muted_until) VALUES($1,$2,$2,clock_timestamp()+interval '5 minutes')")
            .bind(f.tenant.community().as_uuid()).bind(owner.public_key().as_bytes().as_slice())
            .execute(f.state.db.pool()).await.unwrap();
        event_handler::handle_event(first.clone(), Arc::clone(&conn), Arc::clone(&f.state)).await;
        assert_eq!(
            frame(&mut rx).await[2],
            true,
            "timeout permits exact replay"
        );
        let next = signed_revision(&f, &agent, Uuid::new_v4(), None);
        event_handler::handle_event(next, Arc::clone(&conn), Arc::clone(&f.state)).await;
        assert_eq!(
            frame(&mut rx).await[2],
            false,
            "owner timeout blocks new content"
        );
        let sync = start_sync(&f, &conn, filter());
        assert!(
            !initial_set(&mut rx).await.is_empty(),
            "timeout permits reads"
        );
        sqlx::query("UPDATE community_bans SET banned=true WHERE community_id=$1 AND pubkey=$2")
            .bind(f.tenant.community().as_uuid())
            .bind(owner.public_key().as_bytes().as_slice())
            .execute(f.state.db.pool())
            .await
            .unwrap();
        f.state
            .artifact_syncs
            .signal_community(f.tenant.community(), &signal);
        assert_eq!(frame(&mut rx).await[0], "CLOSED");
        ended(sync).await;
        event_handler::handle_event(first, Arc::clone(&conn), Arc::clone(&f.state)).await;
        assert_eq!(frame(&mut rx).await[2], false, "owner ban blocks replay");
        no_frame(&mut rx).await;
    }
}
