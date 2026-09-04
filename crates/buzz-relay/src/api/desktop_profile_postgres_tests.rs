//! Private Desktop profiles through the production HTTP and WebSocket paths.
use super::postgres_tests::bridge_handler_test_state;
use super::*;
use axum::{body::Body, http::Request};
use buzz_core::kind::{KIND_DESKTOP_OBSERVATION, KIND_DESKTOP_PROFILE};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;
use tower::ServiceExt;

async fn post(
    state: &Arc<AppState>,
    host: &str,
    path: &str,
    keys: &Keys,
    body: Value,
    signed: bool,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", host);
    if signed {
        let proof = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", &format!("https://{host}{path}")]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        request = request.header(
            "authorization",
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&proof).unwrap())
            ),
        );
    } else {
        request = request.header("x-pubkey", keys.public_key().to_hex());
    }
    let response = crate::router::build_router(state.clone())
        .oneshot(
            request
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn drain(rx: &mut tokio::sync::mpsc::Receiver<axum::extract::ws::Message>) -> Vec<Value> {
    let mut frames = vec![];
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            panic!("text frame")
        };
        frames.push(serde_json::from_str(&text).unwrap());
    }
    frames
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desktop_profile_authenticated_owner_query_and_private_storage() {
    assert_private_desktop(KIND_DESKTOP_PROFILE).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desktop_observation_authenticated_owner_query_and_private_storage() {
    assert_private_desktop(KIND_DESKTOP_OBSERVATION).await;
}

async fn assert_private_desktop(kind: u32) {
    let mut state = bridge_handler_test_state()
        .await
        .expect("test infrastructure");
    Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).require_auth_token = true;
    let host = format!("desktop-read-{}.example", uuid::Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let tenant = TenantContext::resolved(community, &host);
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let profile = buzz_core::desktop_profile::DesktopProfile::new(
        format!("wss://{host}"),
        uuid::Uuid::new_v4().simple().to_string(),
    )
    .unwrap();
    let id = profile.id.clone();
    let event = if kind == KIND_DESKTOP_PROFILE {
        profile.sign(&owner).unwrap()
    } else {
        buzz_core::desktop_observation::DesktopObservation::new(profile)
            .sign(&owner)
            .unwrap()
    };
    let (status, result) = post(&state, &host, "/events", &owner, json!(event), true).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["accepted"], true, "{result}");
    // Match Desktop's actual bounded owner+kind inventory and exact-coordinate probe.
    let own = json!([{"kinds":[kind], "authors":[owner.public_key().to_hex()], "limit":100}]);
    let exact =
        json!([{"kinds":[kind], "authors":[owner.public_key().to_hex()], "#d":[id], "limit":1}]);
    for filters in [&own, &exact] {
        let (status, rows) = post(&state, &host, "/query", &owner, filters.clone(), true).await;
        assert_eq!(status, StatusCode::OK, "{rows}");
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["id"], event.id.to_hex());
        let (status, result) =
            post(&state, &host, "/query", &outsider, filters.clone(), true).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
        let (status, result) = post(&state, &host, "/query", &owner, filters.clone(), false).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{result}");
    }
    // Known IDs cannot grant an authenticated outsider read access either.
    let known = json!([{"ids":[event.id.to_hex()], "kinds":[kind,1]}]);
    let (status, rows) = post(&state, &host, "/query", &outsider, known, true).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows, json!([]));
    let other_host = format!("other-{host}");
    state
        .db
        .ensure_configured_community(&other_host)
        .await
        .unwrap();
    let (status, rows) = post(&state, &other_host, "/query", &owner, own.clone(), true).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows, json!([]));
    // Inspect the generated column: searching for plaintext in ciphertext proves nothing.
    let mut tx = state.db.begin_event_write_transaction().await.unwrap();
    let indexed: bool = sqlx::query_scalar(
        "SELECT search_tsv IS NOT NULL FROM events WHERE id = $1 AND community_id = $2",
    )
    .bind(event.id.to_bytes().as_slice())
    .bind(community.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert!(!indexed, "private ciphertext must not enter FTS");
    // WS REQ is the frontend transport; bind its real authenticated owner path.
    for who in [&owner, &outsider] {
        let (mut conn, mut rx) = crate::connection::tests::test_conn_with_auth(
            crate::connection::AuthState::Authenticated(buzz_auth::AuthContext {
                pubkey: who.public_key(),
                scopes: buzz_auth::Scope::all_known(),
                channel_ids: None,
                auth_method: buzz_auth::AuthMethod::Nip42,
                agent_owner_pubkey: None,
            }),
        );
        Arc::get_mut(&mut conn).unwrap().tenant = tenant.clone();
        crate::handlers::req::handle_req(
            "desktops".into(),
            serde_json::from_value(own.clone()).unwrap(),
            vec![],
            conn,
            state.clone(),
        )
        .await;
        let frames = drain(&mut rx);
        let rows: Vec<_> = frames.iter().filter(|frame| frame[0] == "EVENT").collect();
        if who.public_key() == owner.public_key() {
            assert_eq!(rows.len(), 1, "{frames:?}");
            assert_eq!(rows[0][2]["id"], event.id.to_hex());
            assert!(frames.iter().any(|frame| frame[0] == "EOSE"), "{frames:?}");
        } else {
            assert!(rows.is_empty(), "{frames:?}");
            assert!(
                frames.iter().any(|frame| frame[0] == "CLOSED"),
                "{frames:?}"
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn aged_desktop_profile_retries_through_production_ingest_without_resigning() {
    let mut state = bridge_handler_test_state()
        .await
        .expect("test infrastructure");
    Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).require_auth_token = true;
    let host = format!("desktop-retry-{}.example", uuid::Uuid::new_v4().simple());
    state.db.ensure_configured_community(&host).await.unwrap();
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let profile = buzz_core::desktop_profile::DesktopProfile::new(
        format!("wss://{host}"),
        uuid::Uuid::new_v4().simple().to_string(),
    )
    .unwrap();
    let prepared = profile.sign(&owner).unwrap();
    // Model bytes committed during yesterday's offline first launch. Neither
    // the first submission nor its duplicate is re-dated or re-signed below.
    let now = Timestamp::now().as_secs();
    let aged = EventBuilder::new(prepared.kind, &prepared.content)
        .tags(prepared.tags.iter().cloned())
        .custom_created_at(Timestamp::from(now - 86_400))
        .sign_with_keys(&owner)
        .unwrap();
    let raw = json!(aged);
    for _ in 0..2 {
        let (status, result) = post(&state, &host, "/events", &owner, raw.clone(), true).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["accepted"], true, "{result}");
        let (status, rows) = post(
            &state,
            &host,
            "/query",
            &owner,
            json!([{"kinds":[KIND_DESKTOP_PROFILE], "authors":[owner.public_key().to_hex()], "ids":[aged.id.to_hex()]}]),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{rows}");
        assert_eq!(rows.as_array().unwrap().len(), 1);
        for field in [
            "id",
            "pubkey",
            "kind",
            "created_at",
            "tags",
            "content",
            "sig",
        ] {
            assert_eq!(rows[0][field], raw[field], "stored {field} changed");
        }
        let stored: nostr::Event = serde_json::from_value(rows[0].clone()).unwrap();
        assert_eq!(
            buzz_core::desktop_profile::DesktopProfile::read(
                &stored,
                &owner,
                &format!("wss://{host}")
            )
            .unwrap(),
            profile
        );
    }
    // The age exception grants no signer authority and bypasses no envelope or
    // signature checks. These calls use the real HTTP -> shared ingest path.
    let (status, result) = post(&state, &host, "/events", &outsider, raw.clone(), true).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
    let mut corrupt = raw.clone();
    corrupt["content"] = json!(format!("{}x", aged.content));
    let (status, result) = post(&state, &host, "/events", &owner, corrupt, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{result}");
    let invalid = EventBuilder::new(aged.kind, &aged.content)
        .tag(Tag::identifier("invalid-coordinate"))
        .custom_created_at(aged.created_at)
        .sign_with_keys(&owner)
        .unwrap();
    let future = EventBuilder::new(aged.kind, &aged.content)
        .tags(aged.tags.iter().cloned())
        .custom_created_at(Timestamp::from(now + 86_400))
        .sign_with_keys(&owner)
        .unwrap();
    let ordinary = EventBuilder::text_note("old ordinary event")
        .custom_created_at(aged.created_at)
        .sign_with_keys(&owner)
        .unwrap();
    for rejected in [invalid, future, ordinary] {
        let (status, result) = post(&state, &host, "/events", &owner, json!(rejected), true).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{result}");
    }
}
