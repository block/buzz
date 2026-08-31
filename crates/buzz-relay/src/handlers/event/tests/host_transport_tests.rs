//! Parse EVENT frames and exercise the WS handler, shared ingest, and primary DB.
use super::*;
use axum::extract::ws::Message;
use buzz_auth::{AuthContext, AuthMethod, Scope};
use buzz_core::{host, tenant::TenantContext};
use nostr::{Event, Timestamp};
use serde_json::{json, Value};

fn connection(
    tenant: TenantContext,
    keys: &Keys,
) -> (
    Arc<crate::connection::ConnectionState>,
    mpsc::Receiver<Message>,
) {
    let (send_tx, rx) = mpsc::channel(32);
    let (ctrl_tx, _) = mpsc::channel(8);
    (
        Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant,
            remote_addr: "127.0.0.1:1".parse().unwrap(),
            auth_state: RwLock::new(crate::connection::AuthState::Authenticated(AuthContext {
                pubkey: keys.public_key(),
                scopes: vec![Scope::UsersWrite, Scope::MessagesWrite],
                channel_ids: None,
                auth_method: AuthMethod::Nip42,
                agent_owner_pubkey: None,
            })),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        }),
        rx,
    )
}

async fn publish(
    state: &Arc<crate::state::AppState>,
    conn: &Arc<crate::connection::ConnectionState>,
    rx: &mut mpsc::Receiver<Message>,
    event: &Event,
) -> Value {
    let crate::protocol::ClientMessage::Event(parsed) =
        crate::protocol::ClientMessage::parse(&json!(["EVENT", event]).to_string()).unwrap()
    else {
        panic!("EVENT frame");
    };
    super::super::handle_event(parsed, conn.clone(), state.clone()).await;
    let Message::Text(frame) = rx.try_recv().expect("handler must send ACK") else {
        panic!("text ACK");
    };
    let ack: Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(ack[0], "OK");
    assert_eq!(ack[1], event.id.to_hex());
    assert!(rx.try_recv().is_err());
    ack
}

fn resign(event: &Event, keys: &Keys, tags: Vec<Tag>, kind: Kind) -> Event {
    EventBuilder::new(kind, event.content.clone())
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated MULTIVERSE_TEST_DATABASE_URL and REDIS_URL; no skip fallback"]
async fn host_report_owner_ws_handler_ack_and_strict_negatives() {
    let url = std::env::var("MULTIVERSE_TEST_DATABASE_URL").expect("isolated PG required");
    assert_eq!(std::env::var("DATABASE_URL").unwrap(), url);
    let redis = std::env::var("REDIS_URL").expect("isolated Redis required");
    let state = super::fanout_access::test_state_with_redis_url(&redis).await;
    let community = state
        .db
        .ensure_configured_community(&format!("host-event-{}.test", Uuid::new_v4()))
        .await
        .unwrap();
    let tenant = TenantContext::resolved(community.id, community.host);
    let owner = Keys::generate();
    let host = Keys::generate();
    let outsider = Keys::generate();
    let (conn, mut rx) = connection(tenant.clone(), &owner);
    let now = Timestamp::now().as_secs();
    let reg = host::registration(&owner, host.public_key(), now).unwrap();
    let rep = host::report(
        &host,
        &reg,
        &host::Report {
            v: 1,
            name: "synthetic".into(),
            os: "test".into(),
            arch: "test".into(),
            launcher_version: "test".into(),
            runtimes: vec![],
            accepts_start: false,
            provisioned: vec![],
        },
        now,
    )
    .unwrap();
    for event in [&reg, &rep] {
        let ack = publish(&state, &conn, &mut rx, event).await;
        assert_eq!(ack[2], true, "{ack}");
        let stored = state
            .db
            .get_event_by_id(tenant.community(), &event.id.to_bytes())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.event, *event, "ACK must follow writer commit");
    }
    // Authorization precedes duplicate detection: even an accepted event is not
    // portable to an outsider, a host-authenticated socket, or a scoped token.
    for keys in [&outsider, &host] {
        let (foreign, mut foreign_rx) = connection(tenant.clone(), keys);
        let ack = publish(&state, &foreign, &mut foreign_rx, &rep).await;
        assert_eq!(ack[2], false, "{ack}");
        assert!(ack[3]
            .as_str()
            .unwrap()
            .contains("owner's global connection"));
    }
    let (scoped, mut scoped_rx) = connection(tenant.clone(), &owner);
    if let crate::connection::AuthState::Authenticated(ctx) = &mut *scoped.auth_state.write().await
    {
        ctx.channel_ids = Some(vec![]);
    }
    assert_eq!(
        publish(&state, &scoped, &mut scoped_rx, &rep).await[2],
        false
    );

    let tags: Vec<Tag> = rep.tags.iter().cloned().collect();
    let replace = |key: &str, value: String| -> Vec<Tag> {
        tags.iter()
            .map(|t| {
                if t.as_slice()[0] == key {
                    Tag::parse([key.to_owned(), value.clone()]).unwrap()
                } else {
                    t.clone()
                }
            })
            .collect()
    };
    let mut duplicate_owner = tags.clone();
    duplicate_owner.push(Tag::parse(["p", &owner.public_key().to_hex()]).unwrap());
    let negatives = [
        (
            resign(&rep, &outsider, tags.clone(), rep.kind),
            "signer does not match",
        ),
        (
            resign(
                &rep,
                &outsider,
                replace("x", outsider.public_key().to_hex()),
                rep.kind,
            ),
            "binding mismatch",
        ),
        (
            resign(
                &rep,
                &host,
                replace("L", "forged.namespace".into()),
                rep.kind,
            ),
            "namespace",
        ),
        (
            resign(&rep, &host, replace("e", "00".repeat(32)), rep.kind),
            "registration not found",
        ),
        (
            resign(
                &rep,
                &host,
                replace("valid_until", (now + 1000).to_string()),
                rep.kind,
            ),
            "lifetime",
        ),
        (
            resign(&rep, &host, duplicate_owner, rep.kind),
            "cardinality",
        ),
        (
            resign(&rep, &host, tags.clone(), Kind::Metadata),
            "pubkey does not match",
        ),
        (
            resign(
                &rep,
                &host,
                tags.clone(),
                Kind::Custom(KIND_PRESENCE_UPDATE as u16),
            ),
            "pubkey does not match",
        ),
        (
            resign(
                &rep,
                &host,
                tags,
                Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16),
            ),
            "pubkey does not match",
        ),
    ];
    for (event, reason) in negatives {
        let ack = publish(&state, &conn, &mut rx, &event).await;
        assert_eq!(ack[2], false, "{ack}");
        assert!(
            ack[3].as_str().unwrap().contains(reason),
            "{ack}: expected {reason}"
        );
        assert!(state
            .db
            .get_event_by_id(tenant.community(), &event.id.to_bytes())
            .await
            .unwrap()
            .is_none());
    }
    let (anonymous, mut anon_rx) = connection(tenant, &owner);
    *anonymous.auth_state.write().await = crate::connection::AuthState::Pending {
        challenge: "test".into(),
    };
    assert_eq!(
        publish(&state, &anonymous, &mut anon_rx, &rep).await[2],
        false
    );
}

#[tokio::test]
#[ignore = "requires isolated MULTIVERSE_TEST_DATABASE_URL and REDIS_URL; no skip fallback"]
async fn host_and_agent_runs_require_authority_and_preserve_other_placements() {
    let url = std::env::var("MULTIVERSE_TEST_DATABASE_URL").expect("isolated PG required");
    assert_eq!(std::env::var("DATABASE_URL").unwrap(), url);
    let redis = std::env::var("REDIS_URL").expect("isolated Redis required");
    let state = super::fanout_access::test_state_with_redis_url(&redis).await;
    let community = state
        .db
        .ensure_configured_community(&format!("runs-{}.test", Uuid::new_v4()))
        .await
        .unwrap();
    let tenant = TenantContext::resolved(community.id, community.host);
    let other_community = state
        .db
        .ensure_configured_community(&format!("runs-other-{}.test", Uuid::new_v4()))
        .await
        .unwrap();
    let other_tenant = TenantContext::resolved(other_community.id, other_community.host);
    let owner = Keys::generate();
    let host = Keys::generate();
    let agent = Keys::generate();
    let outsider = Keys::generate();
    let (conn, mut rx) = connection(tenant.clone(), &owner);
    let now = Timestamp::now().as_secs();
    let reg = host::registration(&owner, host.public_key(), now).unwrap();
    assert_eq!(publish(&state, &conn, &mut rx, &reg).await[2], true);
    let host_pulse = buzz_core::run_presence::pulse(
        &host,
        &"a".repeat(32),
        0,
        "online",
        None,
        Some(&reg.id.to_hex()),
        now,
    )
    .unwrap();
    assert_eq!(publish(&state, &conn, &mut rx, &host_pulse).await[2], true);
    for (scope, signer) in [
        (tenant.clone(), &host),
        (tenant.clone(), &outsider),
        (other_tenant.clone(), &owner),
    ] {
        let (foreign, mut foreign_rx) = connection(scope, signer);
        assert_eq!(
            publish(&state, &foreign, &mut foreign_rx, &host_pulse).await[2],
            false
        );
    }
    let (scoped, mut scoped_rx) = connection(tenant.clone(), &owner);
    if let crate::connection::AuthState::Authenticated(auth) = &mut *scoped.auth_state.write().await
    {
        auth.channel_ids = Some(vec![]);
    }
    assert_eq!(
        publish(&state, &scoped, &mut scoped_rx, &host_pulse).await[2],
        false
    );
    let mut tampered = host_pulse.clone();
    tampered.content = "offline".into();
    assert_eq!(publish(&state, &conn, &mut rx, &tampered).await[2], false);
    assert_eq!(
        state
            .pubsub
            .active_presence_runs(&tenant, &host.public_key(), now)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(state
        .pubsub
        .active_presence_runs(&other_tenant, &host.public_key(), now)
        .await
        .unwrap()
        .is_empty());

    let (agent_conn, mut agent_rx) = connection(tenant.clone(), &agent);
    let location = buzz_core::run_presence::Location {
        host: host.public_key().to_hex(),
        label: "Workshop".into(),
    };
    let pulse = |run: &str, seq, status| {
        buzz_core::run_presence::pulse(&agent, run, seq, status, Some(&location), None, now)
            .unwrap()
    };
    let first = pulse(&"b".repeat(32), 0, "online");
    let second = pulse(&"c".repeat(32), 0, "online");
    // Possessing the owner connection never permits impersonating an agent pulse.
    assert_eq!(publish(&state, &conn, &mut rx, &first).await[2], false);
    for event in [&first, &second] {
        assert_eq!(
            publish(&state, &agent_conn, &mut agent_rx, event).await[2],
            true
        );
    }
    let stop = pulse(&"b".repeat(32), 1, "offline");
    assert_eq!(
        publish(&state, &agent_conn, &mut agent_rx, &stop).await[2],
        true
    );
    // Delayed pulses are ACKed but cannot resurrect a stopped generation.
    assert_eq!(
        publish(&state, &agent_conn, &mut agent_rx, &first).await[2],
        true
    );
    let runs = state
        .pubsub
        .active_presence_runs(&tenant, &agent.public_key(), now)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run, "c".repeat(32));
    assert_eq!(runs[0].location, Some(location));
    assert!(state
        .pubsub
        .active_presence_runs(&tenant, &agent.public_key(), now + 180)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires isolated MULTIVERSE_TEST_DATABASE_URL and REDIS_URL; no skip fallback"]
async fn start_command_receipt_signed_admission_revocation_tenant_and_fts() {
    use buzz_core::host_execution::{self, Action, Command, Outcome, Receipt};
    let url = std::env::var("MULTIVERSE_TEST_DATABASE_URL").expect("isolated PG required");
    assert_eq!(std::env::var("DATABASE_URL").unwrap(), url);
    let state =
        super::fanout_access::test_state_with_redis_url(&std::env::var("REDIS_URL").unwrap()).await;
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let mut tenants = Vec::new();
    for _ in 0..2 {
        let community = state
            .db
            .ensure_configured_community(&format!("start-{}.test", Uuid::new_v4()))
            .await
            .unwrap();
        tenants.push(TenantContext::resolved(community.id, community.host));
    }
    let owner = Keys::generate();
    let host = Keys::generate();
    let outsider = Keys::generate();
    let (conn, mut rx) = connection(tenants[0].clone(), &owner);
    let now = Timestamp::now().as_secs();
    let reg = host::registration(&owner, host.public_key(), now).unwrap();
    assert_eq!(publish(&state, &conn, &mut rx, &reg).await[2], true);
    let request = Command {
        v: 1,
        operation: "ab".repeat(16),
        relay: "wss://start.test".into(),
        agent: Keys::generate().public_key().to_hex(),
        expires_at: now + 300,
        action: Action::Start {
            runtime: "goose".into(),
            revision: "cd".repeat(32),
        },
    };
    let command = host_execution::command(&owner, &reg, &request, now).unwrap();
    let receipt = host_execution::receipt(
        &host,
        &reg,
        &Receipt {
            v: 1,
            command: command.id.to_hex(),
            run: request.run().into(),
            request,
            outcome: Outcome::Spawned,
            observed_at: now,
        },
        now,
    )
    .unwrap();
    for event in [&command, &receipt] {
        for _ in 0..2 {
            // exact resend after lost ACK is admitted, never a second event
            let ack = publish(&state, &conn, &mut rx, event).await;
            assert_eq!(ack[2], true, "{ack}");
        }
        let indexed: bool = sqlx::query_scalar(
            "SELECT search_tsv IS NOT NULL FROM events WHERE community_id=$1 AND id=$2",
        )
        .bind(tenants[0].community().as_uuid())
        .bind(event.id.to_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            !indexed,
            "private command/receipt ciphertext is excluded from FTS"
        );
        for (tenant, signer) in [
            (tenants[0].clone(), &outsider),
            (tenants[0].clone(), &host),
            (tenants[1].clone(), &owner),
        ] {
            let (foreign, mut foreign_rx) = connection(tenant, signer);
            assert_eq!(
                publish(&state, &foreign, &mut foreign_rx, event).await[2],
                false
            );
        }
        let (scoped, mut scoped_rx) = connection(tenants[0].clone(), &owner);
        if let crate::connection::AuthState::Authenticated(ctx) =
            &mut *scoped.auth_state.write().await
        {
            ctx.channel_ids = Some(vec![]);
        }
        assert_eq!(
            publish(&state, &scoped, &mut scoped_rx, event).await[2],
            false
        );
        let forged = resign(
            event,
            &outsider,
            event.tags.iter().cloned().collect(),
            event.kind,
        );
        assert_eq!(publish(&state, &conn, &mut rx, &forged).await[2], false);
    }
    // Delete only this fixture's exact registration. Authorization must run
    // before duplicate detection, including replay of already stored ciphertext.
    assert!(
        buzz_db::event::soft_delete_event(&pool, tenants[0].community(), &reg.id.to_bytes())
            .await
            .unwrap()
    );
    for event in [&command, &receipt] {
        assert_eq!(publish(&state, &conn, &mut rx, event).await[2], false);
    }
}
