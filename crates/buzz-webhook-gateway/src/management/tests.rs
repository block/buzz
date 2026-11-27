use super::*;
use crate::secrets::{SecretKey, SecretKeyring};
use crate::store::{MemoryStore, Store};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use std::sync::{atomic::AtomicBool, Arc};
use tower::ServiceExt;

pub(super) struct Fixture {
    pub(super) private: Router,
    pub(super) relay: Keys,
    pub(super) store: Arc<MemoryStore>,
    secrets: Arc<SecretKeyring>,
}

pub(super) fn fixture() -> Fixture {
    let relay = Keys::generate();
    let store = Arc::new(MemoryStore::default());
    let secrets =
        Arc::new(SecretKeyring::new(vec![SecretKey::new("test", &[7; 32]).unwrap()]).unwrap());
    let state = AppState {
        store: store.clone(),
        secrets: secrets.clone(),
        relay_pubkeys: Arc::new(vec![relay.public_key()]),
        intake: crate::http::IntakeLimits {
            max_body_bytes: 1024,
            timestamp_tolerance_seconds: 300,
        },
        rate_limiter: Arc::new(crate::intake::RateLimiter::new(100)),
        now: chrono::Utc::now,
        accepting: Arc::new(AtomicBool::new(true)),
        public_scheme: crate::config::RelayScheme::Https,
    };
    let (_, private) = crate::http::router(state);
    Fixture {
        private,
        relay,
        store,
        secrets,
    }
}

fn signed_header(keys: &Keys, method: &str, url: &str, body: Option<&[u8]>) -> String {
    let mut tags = vec![
        Tag::parse(["u", url]).unwrap(),
        Tag::parse(["method", method]).unwrap(),
        Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
    ];
    if let Some(body) = body {
        tags.push(Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap());
    }
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap();
    format!("Nostr {}", STANDARD.encode(event.as_json()))
}

pub(super) async fn call(
    fx: &Fixture,
    keys: &Keys,
    method: &str,
    path: &str,
    signed_path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let bytes = body.map(|b| serde_json::to_vec(&b).unwrap());
    let auth = signed_header(
        keys,
        method,
        &format!("https://ingress.example.org{signed_path}"),
        bytes.as_deref(),
    );
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", auth);
    if bytes.is_some() {
        request = request.header("content-type", "application/json");
    }
    let request = request.body(Body::from(bytes.unwrap_or_default())).unwrap();
    let response = fx.private.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, json)
}

pub(super) fn put_body(operation_id: Uuid, revision: i64) -> serde_json::Value {
    serde_json::json!({
        "operation_id": operation_id,
        "revision": revision,
        "community_host": "acme.communities.example.org",
        "provider": "github",
    })
}

#[tokio::test]
async fn relay_creates_reads_and_idempotently_retries_an_integration() {
    let fx = fixture();
    let id = Uuid::new_v4();
    let path = format!("/v1/integrations/{id}");
    let op = Uuid::new_v4();

    let (status, body) = call(&fx, &fx.relay, "PUT", &path, &path, Some(put_body(op, 1))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 1);
    assert_eq!(body["provider"], "github");
    assert_eq!(body["enabled"], true);
    assert_eq!(body["has_secret"], false);
    assert_eq!(
        body["delivery_url"],
        format!("https://acme.communities.example.org/webhooks/{id}")
    );

    let (status, replay) = call(&fx, &fx.relay, "PUT", &path, &path, Some(put_body(op, 1))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay, body);

    let (status, stale) = call(
        &fx,
        &fx.relay,
        "PUT",
        &path,
        &path,
        Some(put_body(Uuid::new_v4(), 1)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        stale,
        serde_json::json!({ "error": "stale_revision", "current": 1 })
    );

    let mut moved = put_body(Uuid::new_v4(), 2);
    moved["community_host"] = serde_json::json!("other.communities.example.org");
    let (status, refused) = call(&fx, &fx.relay, "PUT", &path, &path, Some(moved)).await;
    assert_eq!(status, StatusCode::CONFLICT, "queued messages stay bound");
    assert_eq!(refused["error"], "community_host_immutable");

    let (status, read) = call(&fx, &fx.relay, "GET", &path, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["community_host"], "acme.communities.example.org");
    assert_eq!(read["revision"], 1);

    let op_path = format!("/v1/operations/{op}");
    let (status, recorded) = call(&fx, &fx.relay, "GET", &op_path, &op_path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recorded["integration_id"], serde_json::json!(id));
    assert_eq!(recorded["response"], body);

    let other = format!("/v1/integrations/{}", Uuid::new_v4());
    let (status, conflict) =
        call(&fx, &fx.relay, "PUT", &other, &other, Some(put_body(op, 1))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"], "operation_conflict");

    let (status, conflict) = call(&fx, &fx.relay, "PUT", &path, &path, Some(put_body(op, 2))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"], "operation_conflict");

    let disable_path = format!("{path}/disable");
    let reused = serde_json::json!({ "operation_id": op, "revision": 1 });
    let (status, conflict) = call(
        &fx,
        &fx.relay,
        "POST",
        &disable_path,
        &disable_path,
        Some(reused),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"], "operation_conflict");

    assert_eq!(body["delivery_mode"], "channel_subscriptions");
    assert_eq!(body["group_key_action"], "reply_in_thread");
    let mut configured = put_body(Uuid::new_v4(), 2);
    for (field, value) in [
        ("delivery_mode", "caller_selects_channel"),
        ("channel_expr", "payload.channel_id"),
        ("message_expr", "\"PR #\" + string(payload.number)"),
        ("message_data_expr", "{\"status\": payload.action}"),
        ("group_key_expr", "string(payload.number)"),
        ("group_key_action", "update_message"),
    ] {
        configured[field] = serde_json::json!(value);
    }
    let (status, updated) = call(
        &fx,
        &fx.relay,
        "PUT",
        &path,
        &path,
        Some(configured.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    for field in [
        "delivery_mode",
        "channel_expr",
        "message_expr",
        "message_data_expr",
        "group_key_expr",
        "group_key_action",
    ] {
        assert_eq!(updated[field], configured[field], "{field}");
    }
    let mut changed_action = configured.clone();
    changed_action["group_key_action"] = serde_json::json!("reply_in_thread");
    let (status, conflict) = call(&fx, &fx.relay, "PUT", &path, &path, Some(changed_action)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"], "operation_conflict");

    let mut changed = configured;
    changed["group_key_expr"] = serde_json::json!("payload.id");
    let (status, conflict) = call(&fx, &fx.relay, "PUT", &path, &path, Some(changed)).await;
    assert_eq!(status, StatusCode::CONFLICT, "the config is fingerprinted");
    assert_eq!(conflict["error"], "operation_conflict");

    for (field, bad) in [
        (
            "channel_expr",
            serde_json::json!({ "delivery_mode": "caller_selects_channel" }),
        ),
        ("channel_expr", serde_json::json!({ "channel_expr": "1" })),
        (
            "message_expr",
            serde_json::json!({ "message_expr": "true" }),
        ),
        (
            "message_data_expr",
            serde_json::json!({ "message_data_expr": "payload.a ==" }),
        ),
        (
            "group_key_expr",
            serde_json::json!({ "group_key_expr": "[\"a\"]" }),
        ),
    ] {
        let mut body = put_body(Uuid::new_v4(), 3);
        for (key, value) in bad.as_object().unwrap() {
            body[key] = value.clone();
        }
        let (status, refused) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert_eq!(refused["error"], "invalid_expression");
        assert_eq!(refused["field"], field, "{refused}");
    }
    let mut unknown_action = put_body(Uuid::new_v4(), 3);
    unknown_action["group_key_action"] = serde_json::json!("replace_thread");
    let (status, _) = call(&fx, &fx.relay, "PUT", &path, &path, Some(unknown_action)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut unknown_mode = put_body(Uuid::new_v4(), 3);
    unknown_mode["delivery_mode"] = serde_json::json!("broadcast");
    let (status, _) = call(&fx, &fx.relay, "PUT", &path, &path, Some(unknown_mode)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn secrets_are_sealed_with_the_keyring_and_rotated_with_grace() {
    let fx = fixture();
    let id = Uuid::new_v4();
    let path = format!("/v1/integrations/{id}");
    call(
        &fx,
        &fx.relay,
        "PUT",
        &path,
        &path,
        Some(put_body(Uuid::new_v4(), 1)),
    )
    .await;

    let secret_path = format!("{path}/secret");
    let first = serde_json::json!({
        "operation_id": Uuid::new_v4(),
        "revision": 2,
        "secret": "whsec_first",
        "grace_seconds": 0,
    });
    let (status, body) = call(
        &fx,
        &fx.relay,
        "PUT",
        &secret_path,
        &secret_path,
        Some(first),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["has_secret"], true);
    assert_eq!(body["rotating_until"], serde_json::Value::Null);

    let second = serde_json::json!({
        "operation_id": Uuid::new_v4(),
        "revision": 3,
        "secret": "whsec_second",
        "grace_seconds": 600,
    });
    let (status, body) = call(
        &fx,
        &fx.relay,
        "PUT",
        &secret_path,
        &secret_path,
        Some(second),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["rotating_until"].is_string());

    let secrets = fx
        .store
        .verifying_secrets(id, chrono::Utc::now())
        .await
        .unwrap();
    let plaintexts: Vec<Vec<u8>> = secrets
        .iter()
        .map(|s| fx.secrets.open(&s.ciphertext).unwrap())
        .collect();
    assert_eq!(
        plaintexts,
        vec![b"whsec_second".to_vec(), b"whsec_first".to_vec()]
    );
    assert!(secrets.iter().all(|s| s.key_id == "test"));
    assert!(secrets
        .iter()
        .all(|s| !s.ciphertext.windows(5).any(|w| w == b"whsec")));

    let (status, body) = call(&fx, &fx.relay, "GET", &path, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["revision"], 3);
    assert_eq!(body["has_secret"], true);
}

#[tokio::test]
async fn deliveries_are_listed_without_payloads_and_paged() {
    let fx = fixture();
    let id = Uuid::new_v4();
    let path = format!("/v1/integrations/{id}");
    call(
        &fx,
        &fx.relay,
        "PUT",
        &path,
        &path,
        Some(put_body(Uuid::new_v4(), 1)),
    )
    .await;
    for (n, secs) in [("d1", 0), ("d2", 10), ("d3", 10), ("d4", 20)] {
        fx.store
            .insert_delivery(crate::store::NewDelivery {
                integration_id: id,
                delivery_id: n.into(),
                event_type: "pull_request.opened".into(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: b"{\"secret\":\"payload\"}".to_vec(),
                received_at: chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap(),
            })
            .await
            .unwrap();
    }
    let deliveries = format!("{path}/deliveries");
    let (status, body) = call(
        &fx,
        &fx.relay,
        "GET",
        &format!("{deliveries}?limit=2"),
        &deliveries,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<_> = body["deliveries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["delivery_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["d4", "d3"]);
    assert_eq!(body["deliveries"][0]["state"], "queued");
    assert_eq!(
        body["deliveries"][0]["messages"],
        serde_json::json!({ "queued": 0, "accepted": 0, "failed": 0 })
    );
    assert!(body["deliveries"][0].get("body").is_none());
    assert!(!body.to_string().contains("payload"));
    let next = body["next"].as_str().unwrap().to_owned();
    let (status, body) = call(
        &fx,
        &fx.relay,
        "GET",
        &format!("{deliveries}?limit=2&cursor={next}"),
        &deliveries,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<_> = body["deliveries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["delivery_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["d2", "d1"]);
    let next = body["next"].as_str().unwrap().to_owned();
    let (status, body) = call(
        &fx,
        &fx.relay,
        "GET",
        &format!("{deliveries}?limit=2&cursor={next}"),
        &deliveries,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deliveries"].as_array().unwrap().len(), 0);
    assert!(body["next"].is_null());
    let (status, _) = call(
        &fx,
        &fx.relay,
        "GET",
        &format!("{deliveries}?cursor=garbage"),
        &deliveries,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &fx,
        &fx.relay,
        "GET",
        &format!("{deliveries}?limit=0"),
        &deliveries,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &fx,
        &fx.relay,
        "GET",
        &format!("{deliveries}?bogus=1"),
        &deliveries,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let missing = format!("/v1/integrations/{}/deliveries", Uuid::new_v4());
    let (status, _) = call(&fx, &fx.relay, "GET", &missing, &missing, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disable_and_enable_flip_the_flag_under_revision_control() {
    let fx = fixture();
    let id = Uuid::new_v4();
    let path = format!("/v1/integrations/{id}");
    call(
        &fx,
        &fx.relay,
        "PUT",
        &path,
        &path,
        Some(put_body(Uuid::new_v4(), 1)),
    )
    .await;
    let envelope =
        |revision: i64| serde_json::json!({ "operation_id": Uuid::new_v4(), "revision": revision });

    let disable = format!("{path}/disable");
    let (status, body) = call(
        &fx,
        &fx.relay,
        "POST",
        &disable,
        &disable,
        Some(envelope(2)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], false);

    let enable = format!("{path}/enable");
    let (status, body) = call(&fx, &fx.relay, "POST", &enable, &enable, Some(envelope(2))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = call(&fx, &fx.relay, "POST", &enable, &enable, Some(envelope(3))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], true);

    let missing = format!("/v1/integrations/{}/disable", Uuid::new_v4());
    let (status, _) = call(
        &fx,
        &fx.relay,
        "POST",
        &missing,
        &missing,
        Some(envelope(1)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauthorized_callers_and_mismatched_signatures_are_rejected() {
    let fx = fixture();
    let id = Uuid::new_v4();
    let path = format!("/v1/integrations/{id}");
    let body = put_body(Uuid::new_v4(), 1);

    let stranger = Keys::generate();
    let (status, _) = call(&fx, &stranger, "PUT", &path, &path, Some(body.clone())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let other = format!("/v1/integrations/{}", Uuid::new_v4());
    let (status, _) = call(&fx, &fx.relay, "PUT", &path, &other, Some(body.clone())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let signed_for_other_body = signed_header(
        &fx.relay,
        "PUT",
        &format!("https://ingress.example.org{path}"),
        Some(b"{}"),
    );
    let request = Request::builder()
        .method("PUT")
        .uri(&path)
        .header("authorization", signed_for_other_body)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = fx.private.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let request = Request::builder()
        .method("GET")
        .uri(&path)
        .body(Body::empty())
        .unwrap();
    let response = fx.private.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    assert!(fx.store.integration(id).await.unwrap().is_none());
}

#[tokio::test]
async fn malformed_requests_are_rejected_before_reaching_the_store() {
    let fx = fixture();
    let id = Uuid::new_v4();
    let path = format!("/v1/integrations/{id}");
    let mut unknown_field = put_body(Uuid::new_v4(), 1);
    unknown_field["extra"] = serde_json::json!(true);
    let mut bad_host = put_body(Uuid::new_v4(), 1);
    bad_host["community_host"] = serde_json::json!("Acme.Example.org");
    let mut bad_provider = put_body(Uuid::new_v4(), 1);
    bad_provider["provider"] = serde_json::json!("carrier_pigeon");
    let zero_revision = put_body(Uuid::new_v4(), 0);
    for body in [unknown_field, bad_host, bad_provider, zero_revision] {
        let (status, _) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    call(
        &fx,
        &fx.relay,
        "PUT",
        &path,
        &path,
        Some(put_body(Uuid::new_v4(), 1)),
    )
    .await;
    let secret_path = format!("{path}/secret");
    let too_long = serde_json::json!({
        "operation_id": Uuid::new_v4(),
        "revision": 2,
        "secret": "x".repeat(MAX_SECRET_BYTES + 1),
        "grace_seconds": 0,
    });
    let (status, _) = call(
        &fx,
        &fx.relay,
        "PUT",
        &secret_path,
        &secret_path,
        Some(too_long),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let oversized = vec![b' '; MAX_MANAGEMENT_BODY_BYTES + 1];
    let request = Request::builder()
        .method("PUT")
        .uri(&path)
        .header(
            "authorization",
            signed_header(
                &fx.relay,
                "PUT",
                &format!("https://ingress.example.org{path}"),
                Some(&oversized),
            ),
        )
        .body(Body::from(oversized))
        .unwrap();
    let response = fx.private.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    assert!(fx.store.integration(id).await.unwrap().unwrap().revision == 1);
}

#[test]
fn community_hosts_are_lowercase_dns_names() {
    for ok in [
        "a.example.org",
        "acme-1.communities.example.org",
        "localhost",
    ] {
        assert!(valid_community_host(ok), "{ok}");
    }
    for bad in [
        "",
        "Acme.example.org",
        "-a.example.org",
        "a..b",
        "a.example.org/",
        "a b",
    ] {
        assert!(!valid_community_host(bad), "{bad}");
    }
    assert!(!valid_community_host(
        &"a".repeat(MAX_COMMUNITY_HOST_BYTES + 1)
    ));
}
