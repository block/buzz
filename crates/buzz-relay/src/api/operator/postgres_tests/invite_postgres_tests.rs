use super::*;

const PATH: &str = "/operator/communities/invites";

async fn mint(state: &Arc<AppState>, operator: &Keys, body: Value) -> axum::response::Response {
    signed_operator_request(
        state.clone(),
        operator,
        "POST",
        PATH,
        Some(body.to_string()),
    )
    .await
}

async fn error(response: axum::response::Response, status: StatusCode) -> String {
    assert_eq!(response.status(), status);
    read_json(response).await["error"]
        .as_str()
        .expect("error text")
        .to_owned()
}

/// Claim `code` on the community host the way a joining app does.
async fn claim(state: &Arc<AppState>, host: &str, joiner: &Keys, code: &str) -> StatusCode {
    let body = serde_json::json!({ "code": code }).to_string();
    let auth = nip98_auth_header(
        joiner,
        &format!("https://{host}/api/invites/claim"),
        "POST",
        Some(body.as_bytes()),
    );
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/invites/claim")
                .header(header::HOST, host)
                .header(header::AUTHORIZATION, auth)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response")
        .status()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn operator_mints_invites_only_for_relay_owners_and_admins() {
    let operator = Keys::generate();
    let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
        return;
    };
    let [owner, admin, member, outsider, joiner] = std::array::from_fn(|_| Keys::generate());
    let host = format!("community-{}.example", Uuid::new_v4().simple());
    assert_eq!(
        provision_community(state.clone(), &operator, &host, &owner)
            .await
            .status(),
        StatusCode::OK
    );
    let community = state
        .db
        .lookup_community_by_host(&host)
        .await
        .expect("lookup community")
        .expect("community exists")
        .id;
    for (keys, role) in [(&admin, "admin"), (&member, "member")] {
        state
            .db
            .add_relay_member(
                community,
                &keys.public_key().to_hex(),
                role,
                None,
                &state.relay_keypair,
            )
            .await
            .map(|write| write.outcome)
            .expect("seed member");
    }
    let request = |actor: &Keys, limits: Value| {
        let mut body = serde_json::json!({
            "community_id": community.to_string(),
            "actor_pubkey": actor.public_key().to_hex(),
        });
        body.as_object_mut()
            .expect("object")
            .extend(limits.as_object().expect("limits").clone());
        body
    };

    // An owner gets the /api/invites contract with its defaults.
    let response = mint(&state, &operator, request(&owner, serde_json::json!({}))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let json = read_json(response).await;
    let code = json["code"].as_str().expect("code").to_owned();
    assert_eq!(json["url"], format!("https://{host}/invite/{code}"));
    assert_eq!(json["max_uses"], Value::Null);
    let expires_at = json["expires_at"].as_i64().expect("expires_at");
    let default_expiry = chrono::Utc::now().timestamp() + 72 * 60 * 60;
    assert!((default_expiry - 60..=default_expiry).contains(&expires_at));
    // The code is an ordinary relay invite: the join flow claims it.
    assert_eq!(claim(&state, &host, &joiner, &code).await, StatusCode::OK);
    assert_eq!(
        state
            .db
            .get_relay_member(community, &joiner.public_key().to_hex())
            .await
            .expect("joiner lookup")
            .expect("joiner joined")
            .role,
        "member"
    );

    // An admin may mint too, with bounded limits.
    let response = mint(
        &state,
        &operator,
        request(
            &admin,
            serde_json::json!({ "ttl_secs": 3600, "max_uses": 5 }),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let json = read_json(response).await;
    assert_eq!(json["max_uses"], 5);
    assert_eq!(json["uses_remaining"], 5);

    // A restricted admin keeps the role but can no longer mint.
    state
        .db
        .ban_community_member(
            community,
            admin.public_key().as_bytes(),
            owner.public_key().as_bytes(),
            None,
            None,
        )
        .await
        .expect("ban admin");
    assert_eq!(
        mint(&state, &operator, request(&admin, serde_json::json!({})))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    // The relay's roster decides authority, whatever the operator asserts.
    for actor in [&member, &outsider] {
        assert_eq!(
            error(
                mint(&state, &operator, request(actor, serde_json::json!({}))).await,
                StatusCode::FORBIDDEN
            )
            .await,
            "only relay owners and admins can create invites"
        );
    }
    // Same bounds as /api/invites.
    for (limits, message) in [
        (
            serde_json::json!({ "ttl_secs": 30 * 24 * 60 * 60 + 1 }),
            "ttl_secs must be between",
        ),
        (
            serde_json::json!({ "max_uses": 0 }),
            "max_uses must be between",
        ),
        (
            serde_json::json!({ "max_uses": 10_001 }),
            "max_uses must be between",
        ),
    ] {
        let text = error(
            mint(&state, &operator, request(&owner, limits)).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert!(text.starts_with(message), "{text}");
    }
    let mut unknown = request(&owner, serde_json::json!({}));
    unknown["role"] = "owner".into();
    assert_eq!(
        mint(&state, &operator, unknown).await.status(),
        StatusCode::BAD_REQUEST
    );
    let mut bad_actor = request(&owner, serde_json::json!({}));
    bad_actor["actor_pubkey"] = "not-a-pubkey".into();
    assert_eq!(
        mint(&state, &operator, bad_actor).await.status(),
        StatusCode::BAD_REQUEST
    );
    // Only configured operators may vouch for a caller.
    assert_eq!(
        mint(
            &state,
            &Keys::generate(),
            request(&owner, serde_json::json!({}))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    // Unknown and archived communities have nothing to invite to.
    let mut unknown_community = request(&owner, serde_json::json!({}));
    unknown_community["community_id"] = Uuid::new_v4().to_string().into();
    assert_eq!(
        error(
            mint(&state, &operator, unknown_community).await,
            StatusCode::NOT_FOUND
        )
        .await,
        "community not found"
    );
    archive_for_owner_deletion(&state, &host, &owner).await;
    assert_eq!(
        error(
            mint(&state, &operator, request(&owner, serde_json::json!({}))).await,
            StatusCode::NOT_FOUND
        )
        .await,
        "community not found"
    );
}
