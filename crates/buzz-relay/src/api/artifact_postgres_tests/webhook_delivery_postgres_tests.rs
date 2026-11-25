//! NIP-WH delivery intake through the production router and WebSocket
//! handler: only a configured gateway key may submit deliveries, only over
//! HTTP, and a queued delivery is never served.
use super::*;

const INTEGRATION: &str = "0f4c1a52-7d3e-4b8a-9c61-2e5f8d7a1b30";
const NOT_INGRESS: &str = "restricted: not an authorized webhook ingress";

/// A fixture whose relay trusts a gateway key that is no community member.
pub(super) async fn with_gateway() -> (Fixture, Keys) {
    let mut f = Fixture::new().await;
    let gateway = Keys::generate();
    let mut state = (*f.state).clone();
    Arc::make_mut(&mut state.config).webhook.ingress_pubkeys = vec![gateway.public_key()];
    f.state = Arc::new(state);
    (f, gateway)
}

pub(super) fn delivery(key: &Keys, delivery_id: &str, created_at: u64) -> Event {
    let content = json!({
        "version": 1, "type": "pull_request.opened", "delivery_id": delivery_id,
        "payload": {"action": "opened"},
    });
    EventBuilder::new(Kind::Custom(20701), content.to_string())
        .tags([
            Tag::parse(["integration", INTEGRATION]).unwrap(),
            Tag::parse(["t", "provider:github"]).unwrap(),
            Tag::parse(["delivery", delivery_id]).unwrap(),
        ])
        .custom_created_at(nostr::Timestamp::from_secs(created_at))
        .sign_with_keys(key)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn gateway_deliveries_are_queued_and_never_served() {
    let (f, gateway) = with_gateway().await;
    // An hour old: past the general 900 s window, inside the delivery window,
    // as after a relay outage the gateway buffered through.
    let now = nostr::Timestamp::now().as_secs();
    let queued = delivery(&gateway, "d-1", now - 3600);

    let (status, body) = f.request_as(&gateway, "/events", json!(queued)).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(
        body,
        json!({"event_id": queued.id.to_hex(), "accepted": true, "message": ""})
    );
    let (status, body) = f.request_as(&gateway, "/events", json!(queued)).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["accepted"], true, "{body}");
    assert_eq!(body["message"], "duplicate: already have this event");

    let row: (Uuid, String, String, Value) = sqlx::query_as(
        "SELECT integration_id, provider, state, event FROM webhook_deliveries \
         WHERE community_id = $1 AND delivery_event_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(queued.id.as_bytes().as_slice())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(row.0.to_string(), INTEGRATION);
    assert_eq!((row.1.as_str(), row.2.as_str()), ("github", "pending"));
    assert_eq!(row.3, json!(queued));

    // Queued, not stored: no read path has it.
    for filter in [json!([{"kinds": [20701]}]), json!([{"ids": [queued.id]}])] {
        let (status, body) = f.request("/query", filter.clone()).await;
        assert!(status.is_success(), "{filter}: {status}: {body}");
        assert_eq!(body, json!([]), "{filter}");
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE id = $1")
        .bind(queued.id.as_bytes().as_slice())
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);

    // A malformed delivery is refused for the gateway to dead-letter.
    let mut tags = queued.tags.to_vec();
    tags.retain(|t| t.kind().to_string() != "delivery");
    let untagged = EventBuilder::new(Kind::Custom(20701), queued.content.clone())
        .tags(tags)
        .sign_with_keys(&gateway)
        .unwrap();
    let (status, body) = f.request_as(&gateway, "/events", json!(untagged)).await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string()
            .contains("invalid: delivery needs one integration, t and delivery tag"),
        "{body}"
    );

    // The gateway's NIP-98 key cannot carry another signer's delivery.
    let forged = delivery(&f.peer, "d-2", now);
    let (status, body) = f.request_as(&gateway, "/events", json!(forged)).await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("does not match"), "{body}");
}

/// Members cannot submit deliveries over HTTP, nobody can over WebSocket
/// (whose ephemeral path would broadcast one to every subscriber), and records
/// come only from the relay.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn clients_cannot_submit_webhook_deliveries_or_records() {
    let (f, gateway) = with_gateway().await;
    let now = nostr::Timestamp::now().as_secs();

    let from_member = delivery(&f.owner, "d-1", now);
    let (status, body) = f.request_as(&f.owner, "/events", json!(from_member)).await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains(NOT_INGRESS), "{body}");

    let from_gateway = delivery(&gateway, "d-2", now);
    let auth = crate::connection::AuthState::Authenticated(buzz_auth::AuthContext {
        pubkey: gateway.public_key(),
        scopes: vec![],
        channel_ids: None,
        auth_method: buzz_auth::AuthMethod::Nip42,
        agent_owner_pubkey: None,
    });
    let (mut conn, mut send_rx) = crate::connection::tests::test_conn_with_auth(auth);
    Arc::get_mut(&mut conn).unwrap().tenant =
        buzz_core::TenantContext::resolved(f.community, f.host.clone());
    crate::handlers::event::handle_event(from_gateway.clone(), conn, f.state.clone()).await;
    let axum::extract::ws::Message::Text(text) = send_rx.try_recv().unwrap() else {
        panic!("expected an OK frame");
    };
    let ack: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        ack,
        json!([
            "OK",
            from_gateway.id.to_hex(),
            false,
            "restricted: webhook deliveries are accepted only over HTTP"
        ])
    );

    let record = EventBuilder::new(Kind::Custom(40701), "{}")
        .tags([
            Tag::parse(["h", &f.home.to_string()]).unwrap(),
            Tag::parse(["integration", INTEGRATION]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    let (status, body) = f.request_as(&f.owner, "/events", json!(record)).await;
    assert!(
        !status.is_success() && body.to_string().contains("restricted"),
        "{status}: {body}"
    );
}
