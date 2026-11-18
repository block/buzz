//! Community operations through the production HTTP router, with real signed
//! NIP-98 bodies. TCP coverage below exercises the same serving router.
use super::*;
use axum::http::StatusCode;
use base64::Engine;
use sha2::Digest;

async fn fixture() -> Fixture {
    let mut f = Fixture::new().await;
    Arc::make_mut(&mut Arc::get_mut(&mut f.state).unwrap().config).community_artifacts_enabled =
        true;
    let mut connection = f.pool.acquire().await.unwrap();
    sqlx::query(
        "SET buzz.community_artifact_contract_ready='all-backends-enforce-community-audience'",
    )
    .execute(&mut *connection)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../scripts/maintenance/community-artifact-contract.sql"
    ))
    .execute(&mut *connection)
    .await
    .unwrap();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'owner')")
        .bind(f.community.as_uuid())
        .bind(f.owner.public_key().to_hex())
        .execute(&f.pool)
        .await
        .unwrap();
    f
}
fn revision(f: &Fixture, keys: &Keys, d: Uuid, op: &str, prev: Option<&Event>) -> Event {
    let mut tags = vec![
        vec!["ar".into(), "1".into()],
        vec!["community".into(), f.community.to_string()],
        vec!["d".into(), d.to_string()],
        vec!["type".into(), "example.settings".into()],
        vec!["op".into(), op.into()],
    ];
    if op != "delete" {
        tags.push(vec!["title".into(), "Settings".into()]);
    }
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    EventBuilder::new(
        Kind::Custom(45010),
        if op == "delete" {
            ""
        } else {
            "private artifact needle"
        },
    )
    .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
    .sign_with_keys(keys)
    .unwrap()
}
fn authorization(f: &Fixture, keys: &Keys, path: &str, bytes: &[u8]) -> String {
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags([
            Tag::parse(["u", &format!("https://{}{path}", f.host)]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(sha2::Sha256::digest(bytes))]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
    )
}
async fn signed(f: &Fixture, keys: &Keys, path: &str, body: Value) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let bytes = serde_json::to_vec(&body).unwrap();
    let response = crate::router::build_router(Arc::clone(&f.state))
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("host", &f.host)
                .header("content-type", "application/json")
                .header("authorization", authorization(f, keys, path, &bytes))
                .body(axum::body::Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn policy(f: &Fixture, policy: &str, prev: Option<&Event>) -> Event {
    EventBuilder::new(Kind::Custom(9034), "")
        .tags([
            Tag::parse(["community", &f.community.to_string()]).unwrap(),
            Tag::parse(["type", "example.settings"]).unwrap(),
            Tag::parse(["policy", policy]).unwrap(),
            Tag::parse([
                "prev",
                &prev.map(|e| e.id.to_hex()).unwrap_or_else(|| "0".into()),
            ])
            .unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap()
}
async fn accepted(f: &Fixture, keys: &Keys, event: &Event) {
    let (status, body) = signed(f, keys, "/events", json!(event)).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["accepted"], true, "{body}");
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn signed_http_policy_replay_tenant_privacy_and_moderation() {
    let f = fixture().await;
    let d = Uuid::new_v4();
    let event = revision(&f, &f.peer, d, "create", None);
    accepted(&f, &f.peer, &event).await;
    let current = json!([{"artifact":"current","scope":"community","#type":["example.settings"]}]);
    let (status, body) = signed(&f, &f.peer, "/query", current.clone()).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body[0]["id"], event.id.to_hex());
    for (ids, expected) in [
        (vec![d.simple().to_string()], json!([])),
        (
            vec![Uuid::new_v4().to_string(), d.to_string()],
            json!([event]),
        ),
    ] {
        let filter = json!([{"artifact":"history","scope":"community","#d":ids}]);
        let (status, body) = signed(&f, &f.peer, "/query", filter).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, expected);
    }
    assert!(
        !f.request_as(&f.peer, "/query", current.clone())
            .await
            .0
            .is_success(),
        "unsigned dev header cannot read"
    );
    for query in [
        json!([{"kinds":[45010]}]),
        json!([{"ids":[event.id.to_hex()]}]),
        json!([{"kinds":[45010],"search":"needle"}]),
        json!([{"artifact":"current"}]),
        json!([{"artifact":"history","#d":[d]}]),
    ] {
        let (status, body) = signed(&f, &f.peer, "/query", query).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body, json!([]));
    }
    let (status, body) = signed(&f, &f.peer, "/count", json!([{"kinds":[45010]}])).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["count"], 0);
    for path in ["/count"] {
        assert!(!signed(&f, &f.peer, path, current.clone())
            .await
            .0
            .is_success());
    }
    for query in [
        json!([{"artifact":"incremental","scope":"community","query":{"artifact":"current"}}]),
        json!([{"artifact":"incremental","query":{"artifact":"current","scope":"community"}}]),
    ] {
        assert!(!signed(&f, &f.peer, "/query", query).await.0.is_success());
    }
    let command = policy(&f, "admins", None);
    accepted(&f, &f.owner, &command).await;
    accepted(&f, &f.owner, &command).await;
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE community_id=$1 AND object_id=$2")
            .bind(f.community.as_uuid())
            .bind(command.id.to_hex())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(audits, 1);
    let (status, body) = signed(
        &f,
        &f.peer,
        "/query",
        json!([{"artifact":"policy","scope":"community","#type":["example.settings"]}]),
    )
    .await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(
        body,
        json!({"policy":"admins","revision":command.id.to_hex()})
    );
    accepted(&f, &f.peer, &event).await; // An acknowledgement cannot authorize a new edit.
    let update = revision(&f, &f.peer, d, "update", Some(&event));
    assert!(!signed(&f, &f.peer, "/events", json!(update))
        .await
        .0
        .is_success());
    let wrong_host = format!("other-{}.example", Uuid::new_v4());
    let wrong = f
        .state
        .db
        .ensure_configured_community(&wrong_host)
        .await
        .unwrap()
        .id;
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'owner')")
        .bind(wrong.as_uuid())
        .bind(f.owner.public_key().to_hex())
        .execute(&f.pool)
        .await
        .unwrap();
    let mut wrong_f = Fixture {
        state: Arc::clone(&f.state),
        pool: f.pool.clone(),
        host: wrong_host,
        community: wrong,
        home: f.home,
        private: f.private,
        owner: f.owner.clone(),
        peer: f.peer.clone(),
    };
    for (keys, target) in [(&f.owner, &command), (&f.peer, &event)] {
        let (status, body) = signed(&wrong_f, keys, "/events", json!(target)).await;
        assert!(!status.is_success(), "{body}");
    }
    wrong_f.host = f.host.clone(); // Reuse only for malformed scope controls below.
                                   // Authority precedes target-specific refusal. A protected community
                                   // revision and an unknown ID must have the same kind-5 response.
    let mut refusals = Vec::new();
    for target in [event.id, nostr::EventId::from_byte_array([0xab; 32])] {
        let deletion = EventBuilder::new(Kind::Custom(5), "")
            .tags([Tag::event(target)])
            .sign_with_keys(&f.owner)
            .unwrap();
        let response = signed(&f, &f.owner, "/events", json!(deletion)).await;
        assert!(!response.0.is_success());
        refusals.push((response.0, response.1["error"].clone()));
    }
    assert_eq!(refusals[0], refusals[1]);
    for path in ["/query", "/count"] {
        assert_eq!(
            signed(&f, &f.peer, path, json!([{ "scope":"community" }]))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let moderation = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["community", &f.community.to_string()]).unwrap(),
            Tag::event(event.id),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    accepted(&f, &f.owner, &moderation).await;
    let (_, body) = signed(&f, &f.peer, "/query", current.clone()).await;
    assert_eq!(body, json!([]));
    accepted(&f, &f.peer, &event).await; // Ledger receipt survives redaction.
    sqlx::query(
        "INSERT INTO community_bans(community_id,pubkey,actor_pubkey,banned) VALUES($1,$2,$3,true)",
    )
    .bind(f.community.as_uuid())
    .bind(f.peer.public_key().as_bytes().as_slice())
    .bind(f.owner.public_key().as_bytes().as_slice())
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(!signed(&f, &f.peer, "/events", json!(event))
        .await
        .0
        .is_success());
    assert!(!signed(&f, &f.peer, "/query", current).await.0.is_success());
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn policy_command_preserves_relay_admin_timeout_and_ban_rules() {
    let f = fixture().await;
    let command = policy(&f, "admins", None);
    accepted(&f, &f.owner, &command).await;
    let next = policy(&f, "everyone", Some(&command));
    sqlx::query("INSERT INTO community_bans(community_id,pubkey,actor_pubkey,muted_until) VALUES($1,$2,$2,clock_timestamp()+interval '5 minutes')")
        .bind(f.community.as_uuid())
        .bind(f.owner.public_key().as_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    f.state.invalidate_restrictions_local(f.community);
    for event in [&next, &command] {
        let (status, body) = signed(&f, &f.owner, "/events", json!(event)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("restricted: you are timed out"),
            "{body}"
        );
    }
    let read = json!([{"artifact":"policy","scope":"community","#type":["example.settings"]}]);
    let (status, body) = signed(&f, &f.owner, "/query", read).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"policy":"admins","revision":command.id.to_hex()})
    );
    let content = revision(&f, &f.owner, Uuid::new_v4(), "create", None);
    assert!(!signed(&f, &f.owner, "/events", json!(content))
        .await
        .0
        .is_success());
    sqlx::query(
        "UPDATE community_bans SET banned=true,muted_until=NULL WHERE community_id=$1 AND pubkey=$2",
    )
        .bind(f.community.as_uuid())
        .bind(f.owner.public_key().as_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    f.state.invalidate_restrictions_local(f.community);
    assert!(!signed(&f, &f.owner, "/events", json!(command))
        .await
        .0
        .is_success());
}

/// Seed an unsupported legacy event-store row to prove privacy fences before
/// SQL LIMIT/rank/count and raw hydration, even with the feature switch off.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn ordinary_apis_withhold_unexpected_community_event_rows() {
    let mut f = Fixture::new().await;
    Arc::make_mut(&mut Arc::get_mut(&mut f.state).unwrap().config).community_artifacts_enabled =
        false;
    let event = revision(&f, &f.owner, Uuid::new_v4(), "create", None);
    f.state
        .db
        .insert_event(f.community, &event, Some(f.home))
        .await
        .unwrap();
    for query in [
        json!([{"kinds":[45010]}]),
        json!([{"ids":[event.id.to_hex()]}]),
        json!([{"kinds":[45010],"search":"needle"}]),
        json!([{"artifact":"history","#d":[event.tags.identifier().unwrap()]}]),
    ] {
        let (status, body) = signed(&f, &f.owner, "/query", query).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body, json!([]));
    }
    let (_, body) = signed(&f, &f.owner, "/count", json!([{"kinds":[45010]}])).await;
    assert_eq!(body["count"], 0);
    assert!(f
        .state
        .db
        .get_event_by_id(f.community, event.id.as_bytes())
        .await
        .unwrap()
        .is_none());
    let stored = buzz_core::StoredEvent::new(event, Some(f.home));
    let recipient = (Uuid::new_v4(), "ordinary".into());
    assert!(crate::handlers::event::filter_fanout_by_access(
        &f.state,
        f.community,
        &stored,
        vec![recipient.clone()],
        None
    )
    .await
    .is_empty());
    assert!(crate::handlers::event::route_fanout_matches(
        &f.state,
        f.community,
        &stored,
        vec![recipient],
        None
    )
    .await
    .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn channel_artifact_with_community_tag_value_remains_visible() {
    use crate::handlers::group_state_postgres_tests::{req_event_ids, test_conn};

    let mut f = Fixture::new().await;
    Arc::make_mut(&mut Arc::get_mut(&mut f.state).unwrap().config).community_artifacts_enabled =
        false;
    let d = Uuid::new_v4();
    let event = f.revision(d, "create", None);
    let event = resign(
        &event,
        &f.owner,
        &[["project", "community"]],
        event.created_at.as_secs(),
    );
    f.publish(&event).await;
    let id = event.id.to_hex();
    let filter = json!({"kinds":[45010],"#h":[f.home]});

    for read in [
        filter.clone(),
        json!({"ids":[id]}),
        json!({"artifact":"current","#d":[d]}),
        json!({"artifact":"history","#d":[d]}),
        json!({"kinds":[45010],"#h":[f.home],"search":"searchable"}),
    ] {
        let (status, body) = f.request("/query", json!([read])).await;
        assert!(status.is_success(), "{read}: {status} {body}");
        assert_eq!(body.as_array().unwrap().len(), 1, "{read}: {body}");
        assert_eq!(body[0]["id"], id, "{read}: {body}");
    }
    let (status, body) = f.request("/count", json!([filter])).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["count"], 1, "{body}");

    let tenant = crate::tenant::bind_community(&f.state.db, &f.host)
        .await
        .unwrap();
    assert_eq!(
        req_event_ids(&f.state, &tenant, &f.owner, filter.clone()).await,
        std::slice::from_ref(&id)
    );
    let (conn, mut rx) = test_conn(&tenant, &f.owner);
    let filter = serde_json::from_value(filter).unwrap();
    crate::handlers::count::handle_count("c".into(), vec![filter], conn, f.state.clone()).await;
    let axum::extract::ws::Message::Text(frame) = rx.try_recv().expect("COUNT reply") else {
        panic!("expected a WebSocket text frame");
    };
    let message: Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(message, json!(["COUNT", "c", {"count":1}]));

    let db = &f.state.db;
    let bytes = event.id.as_bytes();
    assert_eq!(
        db.get_event_by_id(f.community, bytes)
            .await
            .unwrap()
            .map(|stored| stored.event.id),
        Some(event.id)
    );
    assert_eq!(
        db.get_event_by_id_including_deleted(f.community, bytes)
            .await
            .unwrap()
            .map(|stored| stored.event.id),
        Some(event.id)
    );
    assert_eq!(
        db.admin_get_event_preview(*f.community.as_uuid(), bytes)
            .await
            .unwrap()
            .map(|preview| preview.id),
        Some(id)
    );
}

/// The production policy command must roll back its CAS if its hash-chain
/// audit cannot persist, and the exact command can then be retried.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn policy_audit_failure_rolls_back_cas_and_retry_keeps_chain_valid() {
    let f = fixture().await;
    let command = policy(&f, "admins", None);
    sqlx::query("ALTER TABLE audit_log ADD CONSTRAINT probe_audit_failure CHECK (false) NOT VALID")
        .execute(&f.pool)
        .await
        .unwrap();
    let (status, _) = signed(&f, &f.owner, "/events", json!(command)).await;
    assert!(!status.is_success());
    let state: Value =
        sqlx::query_scalar("SELECT artifact_type_policies FROM communities WHERE id=$1")
            .bind(f.community.as_uuid())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(state, json!({}));
    sqlx::query("ALTER TABLE audit_log DROP CONSTRAINT probe_audit_failure")
        .execute(&f.pool)
        .await
        .unwrap();
    accepted(&f, &f.owner, &command).await;
    accepted(&f, &f.owner, &command).await;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE community_id=$1 AND object_id=$2")
            .bind(f.community.as_uuid())
            .bind(command.id.to_hex())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert!(buzz_audit::AuditService::new(f.pool.clone())
        .verify_chain(f.community, 1, i64::MAX)
        .await
        .unwrap());
}
