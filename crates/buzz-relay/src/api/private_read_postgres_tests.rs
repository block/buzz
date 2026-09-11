//! Existing author-only reads through HTTP and WebSocket storage seams.
use super::postgres_tests::bridge_handler_test_state;
use super::*;
use axum::{body::Body, http::Request};
use buzz_core::kind::KIND_EVENT_REMINDER;
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
async fn existing_private_reads_preserve_visible_pages_and_counts() {
    let mut state = bridge_handler_test_state()
        .await
        .expect("test infrastructure");
    let host = format!("private-read-{}.example", uuid::Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let tenant = TenantContext::resolved(community, &host);
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let now = Timestamp::now().as_secs();
    // Ingest the existing reminder envelope, not a new kind or test-only row.
    // Terminal reminders may omit not_before; the relay never decrypts content.
    let ciphertext = nostr::nips::nip44::encrypt(
        owner.secret_key(),
        &owner.public_key(),
        r#"{"status":"done","note":"private reminder"}"#,
        nostr::nips::nip44::Version::V2,
    )
    .unwrap();
    let reminder = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), &ciphertext)
        .tags([Tag::parse(["d", &uuid::Uuid::new_v4().to_string()]).unwrap()])
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(&owner)
        .unwrap();
    let mut public = vec![];
    for text in ["first public note", "second public note"] {
        public.push(
            EventBuilder::text_note(text)
                .custom_created_at(Timestamp::from(now - 1))
                .sign_with_keys(&owner)
                .unwrap(),
        );
    }
    public.sort_by_key(|event| event.id);
    for event in std::iter::once(&reminder).chain(public.iter()) {
        let (status, result) = post(&state, &host, "/events", &owner, json!(event), true).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["accepted"], true, "{result}");
    }
    let own = json!([{"kinds":[30300], "authors":[owner.public_key().to_hex()]}]);
    let known = json!([{"ids":[reminder.id.to_hex()]}]);
    let mixed = json!([{"kinds":[30300,1], "limit":1}]);
    // Exercise the existing production NIP-98 contract and development X-Pubkey
    // contract. Both must page/count using the reader selected by that mode.
    for strict in [false, true] {
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).require_auth_token = strict;
        for route in ["/query", "/count"] {
            if strict {
                let (status, result) = post(&state, &host, route, &owner, own.clone(), false).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{route}: {result}");
            }
            let (status, result) = post(&state, &host, route, &outsider, own.clone(), strict).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
            let (status, result) = post(&state, &host, route, &owner, own.clone(), strict).await;
            assert_eq!(status, StatusCode::OK, "{result}");
            if route == "/query" {
                assert_eq!(result[0]["id"], reminder.id.to_hex());
            } else {
                assert_eq!(result["count"], 1);
            }
            let (status, result) = post(
                &state,
                &host,
                route,
                &outsider,
                json!([{"kinds":[1]}]),
                false,
            )
            .await;
            assert_eq!(
                status,
                if strict {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::OK
                },
                "{result}"
            );
        }
        for who in [&owner, &outsider] {
            let is_owner = who.public_key() == owner.public_key();
            let (status, rows) = post(&state, &host, "/query", who, mixed.clone(), strict).await;
            assert_eq!(status, StatusCode::OK, "{rows}");
            assert_eq!(rows.as_array().unwrap().len(), 1);
            assert_eq!(
                rows[0]["id"],
                if is_owner {
                    reminder.id.to_hex()
                } else {
                    public[0].id.to_hex()
                }
            );
            // COUNT ignores the page limit, but must count only visible rows.
            let (status, result) = post(&state, &host, "/count", who, mixed.clone(), strict).await;
            assert_eq!(status, StatusCode::OK, "{result}");
            assert_eq!(result["count"], if is_owner { 3 } else { 2 });
            let (status, rows) = post(&state, &host, "/query", who, known.clone(), strict).await;
            assert_eq!(status, StatusCode::OK, "{rows}");
            assert_eq!(rows.as_array().unwrap().len(), usize::from(is_owner));
            let (status, result) = post(&state, &host, "/count", who, known.clone(), strict).await;
            assert_eq!(status, StatusCode::OK, "{result}");
            assert_eq!(result["count"], u64::from(is_owner));
        }
    }
    // Both offset and composite cursors must page over visible records, including ties.
    for filters in [
        json!([{"kinds":[30300,1], "limit":1, "page":2}]),
        json!([{"kinds":[30300,1], "limit":1, "until":now-1, "before_id":public[0].id.to_hex()}]),
    ] {
        let (status, rows) = post(&state, &host, "/query", &outsider, filters, true).await;
        assert_eq!(status, StatusCode::OK, "{rows}");
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["id"], public[1].id.to_hex());
    }
    // WS uses its existing authenticated principal, never an HTTP dev identity.
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
        let filters = serde_json::from_value(mixed.clone()).unwrap();
        crate::handlers::req::handle_req(
            "page".into(),
            filters,
            vec![],
            conn.clone(),
            state.clone(),
        )
        .await;
        let frames = drain(&mut rx);
        let rows: Vec<_> = frames.iter().filter(|frame| frame[0] == "EVENT").collect();
        assert_eq!(rows.len(), 1, "{frames:?}");
        let is_owner = who.public_key() == owner.public_key();
        assert_eq!(
            rows[0][2]["id"],
            if is_owner {
                reminder.id.to_hex()
            } else {
                public[0].id.to_hex()
            }
        );
        crate::handlers::count::handle_count(
            "count".into(),
            serde_json::from_value(mixed.clone()).unwrap(),
            conn,
            state.clone(),
        )
        .await;
        let frames = drain(&mut rx);
        assert_eq!(frames[0][0], "COUNT", "{frames:?}");
        assert_eq!(frames[0][2]["count"], if is_owner { 3 } else { 2 });
    }
    // More foreign reminders than COUNT's candidate budget must not turn an
    // outsider's small visible count into a 'narrower constraints' error. Seed
    // signed storage rows directly to avoid thousands of HTTP admission calls.
    for _ in 0..crate::handlers::req::COUNT_FALLBACK_CANDIDATE_LIMIT {
        let event = EventBuilder::new(reminder.kind, &ciphertext)
            .tags([Tag::parse(["d", &uuid::Uuid::new_v4().to_string()]).unwrap()])
            .custom_created_at(Timestamp::from(now))
            .sign_with_keys(&owner)
            .unwrap();
        assert!(
            state
                .db
                .insert_event(community, &event, None)
                .await
                .unwrap()
                .1
        );
    }
    let (status, result) = post(&state, &host, "/count", &outsider, mixed.clone(), true).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["count"], 2);
    // The budget must still reject genuinely over-budget visible candidate sets.
    let (status, _) = post(&state, &host, "/count", &owner, mixed.clone(), true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
