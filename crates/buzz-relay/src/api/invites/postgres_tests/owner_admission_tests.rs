mod postgres_tests {
    use super::super::restriction_tests::postgres_tests::{fixture, post_with_owner_tag};
    use super::super::*;

    async fn users(state: &AppState, community: buzz_core::CommunityId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM users WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(state.db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn invalid_signed_claims_do_not_materialize_fresh_owner_identities() {
        let (state, host, community, _issuer) = fixture().await;
        let before = users(&state, community).await;
        let unknown_v2 = buzz_core::invite::encode_v2_code(&[42; 32]);
        for code in ["garbage", "v2.bad", &unknown_v2] {
            let owner = Keys::generate();
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
            assert_eq!(users(&state, community).await, before);
            assert!(!state
                .db
                .is_relay_member(community, &agent.public_key().to_hex())
                .await
                .unwrap());
        }
        let owner = Keys::generate();
        let agent = Keys::generate();
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();
        for attempt in 0..=CLAIM_RATE_LIMIT {
            let response = post_with_owner_tag(
                state.clone(),
                &host,
                "/api/invites/claim",
                &agent,
                serde_json::json!({"code":"garbage"}).to_string(),
                &tag,
            )
            .await;
            assert_eq!(
                response.status(),
                if attempt == CLAIM_RATE_LIMIT {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_eq!(users(&state, community).await, before);
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verified_owner_is_committed_only_with_successful_v1_and_v2_claims() {
        let (state, host, community, issuer) = fixture().await;
        let v2 = mint_code(
            state.clone(),
            &host,
            &issuer,
            serde_json::json!({"max_uses":1}),
        )
        .await;
        let key = derive_invite_key(&state.relay_keypair);
        let (v1, _) = crate::invite_token::mint_invite(&key, community, 3600);
        for code in [v1, v2] {
            let owner = Keys::generate();
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
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(read_json(response).await["status"], "joined");
            assert!(state
                .db
                .is_agent_owner(
                    community,
                    agent.public_key().as_bytes(),
                    owner.public_key().as_bytes()
                )
                .await
                .unwrap());
            let before = users(&state, community).await;
            let response = post_with_owner_tag(
                state.clone(),
                &host,
                "/api/invites/claim",
                &agent,
                serde_json::json!({"code":code}).to_string(),
                &tag,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(read_json(response).await["status"], "already_member");
            assert_eq!(users(&state, community).await, before);
            let impostor_owner = Keys::generate();
            let mismatched =
                buzz_sdk::nip_oa::compute_auth_tag(&impostor_owner, &agent.public_key(), "")
                    .unwrap();
            let response = post_with_owner_tag(
                state.clone(),
                &host,
                "/api/invites/claim",
                &agent,
                serde_json::json!({"code":code}).to_string(),
                &mismatched,
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(read_json(response).await["error"], "invite_owner_mismatch");
            assert_eq!(users(&state, community).await, before);
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn banned_verified_owner_creates_no_users_in_either_claim_format() {
        let (state, host, community, issuer) = fixture().await;
        let v2 = mint_code(
            state.clone(),
            &host,
            &issuer,
            serde_json::json!({"max_uses":1}),
        )
        .await;
        let key = derive_invite_key(&state.relay_keypair);
        let (v1, _) = crate::invite_token::mint_invite(&key, community, 3600);
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
        let before = users(&state, community).await;
        for code in [v1, v2] {
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
            assert_eq!(users(&state, community).await, before);
        }
    }
}
