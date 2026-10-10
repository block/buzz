use super::*;

async fn fixture() -> (Arc<AppState>, String, buzz_core::CommunityId, Keys) {
    let host = format!("restricted-invite-{}.example", Uuid::new_v4().simple());
    let state = invite_test_state(&host)
        .await
        .expect("isolated PostgreSQL state");
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let owner = Keys::generate();
    state
        .db
        .add_relay_member(community, &owner.public_key().to_hex(), "owner", None)
        .await
        .unwrap();
    (state, host, community, owner)
}

async fn post_with_owner_tag(
    state: Arc<AppState>,
    host: &str,
    path: &str,
    agent: &Keys,
    body: String,
    auth_tag: &str,
) -> axum::response::Response {
    let auth = nip98_auth_header(agent, &format!("https://{host}{path}"), body.as_bytes());
    build_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::HOST, host)
                .header(header::AUTHORIZATION, auth)
                .header("x-auth-tag", auth_tag)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn signed_nonmember_claims_reject_restrictions_without_consuming_invite() {
    let (state, host, community, issuer) = fixture().await;
    let code = mint_code(
        state.clone(),
        &host,
        &issuer,
        serde_json::json!({"max_uses":1}),
    )
    .await;
    let claimant = Keys::generate();
    state
        .db
        .ban_community_member(
            community,
            claimant.public_key().as_bytes(),
            issuer.public_key().as_bytes(),
            None,
            None,
        )
        .await
        .unwrap();
    let key = derive_invite_key(&state.relay_keypair);
    let (legacy, _) = crate::invite_token::mint_invite(&key, community, 3600);
    for bearer in [&code, &legacy] {
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &claimant,
            serde_json::json!({"code":bearer}).to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(read_json(response).await["error"], "invite_restricted");
    }
    assert!(!state
        .db
        .is_relay_member(community, &claimant.public_key().to_hex())
        .await
        .unwrap());
    let unrestricted = Keys::generate();
    assert_eq!(
        post_json(
            state,
            &host,
            "/api/invites/claim",
            &unrestricted,
            serde_json::json!({"code":code}).to_string()
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn verified_owner_blocks_first_agent_claim_but_forged_owner_does_not_bind() {
    let (state, host, community, issuer) = fixture().await;
    let code = mint_code(state.clone(), &host, &issuer, serde_json::json!({})).await;
    let owner = Keys::generate();
    state
        .db
        .ban_community_member(
            community,
            owner.public_key().as_bytes(),
            issuer.public_key().as_bytes(),
            None,
            None,
        )
        .await
        .unwrap();
    let agent = Keys::generate();
    let tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();
    let response = post_with_owner_tag(
        state.clone(),
        &host,
        "/api/invites/claim",
        &agent,
        serde_json::json!({"code":code}).to_string(),
        &tag,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(read_json(response).await["error"], "invite_restricted");
    assert!(!state
        .db
        .is_relay_member(community, &agent.public_key().to_hex())
        .await
        .unwrap());
    let other_agent = Keys::generate();
    // A credential signed for another key cannot bind this claimant's owner.
    let response = post_with_owner_tag(
        state.clone(),
        &host,
        "/api/invites/claim",
        &other_agent,
        serde_json::json!({"code":code}).to_string(),
        &tag,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!state
        .db
        .is_agent_owner(
            community,
            other_agent.public_key().as_bytes(),
            owner.public_key().as_bytes()
        )
        .await
        .unwrap());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn committed_issuer_ban_survives_unban_through_http() {
    let (state, host, community, issuer) = fixture().await;
    let code = mint_code(state.clone(), &host, &issuer, serde_json::json!({})).await;
    state
        .db
        .ban_community_member(
            community,
            issuer.public_key().as_bytes(),
            issuer.public_key().as_bytes(),
            None,
            None,
        )
        .await
        .unwrap();
    state
        .db
        .unban_community_member(
            community,
            issuer.public_key().as_bytes(),
            issuer.public_key().as_bytes(),
        )
        .await
        .unwrap();
    let response = post_json(
        state.clone(),
        &host,
        "/api/invites/claim",
        &Keys::generate(),
        serde_json::json!({"code":code}).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(read_json(response).await["error"], "invite_invalid");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cutoff_rejects_only_v1_and_restriction_lookup_failure_never_admits() {
    let (state, host, community, issuer) = fixture().await;
    let code = mint_code(state.clone(), &host, &issuer, serde_json::json!({})).await;
    let key = derive_invite_key(&state.relay_keypair);
    let (legacy, _) = crate::invite_token::mint_invite(&key, community, 3600);
    let Ok(mut state) = Arc::try_unwrap(state) else {
        panic!("exclusive fixture state");
    };
    Arc::make_mut(&mut state.config).invite_v1_invalid_after = Some(0);
    let state = Arc::new(state);
    let response = post_json(
        state.clone(),
        &host,
        "/api/invites/claim",
        &Keys::generate(),
        serde_json::json!({"code":legacy}).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(read_json(response).await["error"], "invite_expired");
    assert_eq!(
        post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &Keys::generate(),
            serde_json::json!({"code":code}).to_string()
        )
        .await
        .status(),
        StatusCode::OK
    );
    // This test owns its per-process database; never alters a shared fixture.
    sqlx::query("ALTER TABLE community_bans RENAME TO unavailable_restrictions")
        .execute(state.db.pool())
        .await
        .unwrap();
    let claimant = Keys::generate();
    let response = post_json(
        state.clone(),
        &host,
        "/api/invites/claim",
        &claimant,
        serde_json::json!({"code":code}).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!state
        .db
        .is_relay_member(community, &claimant.public_key().to_hex())
        .await
        .unwrap());
}
