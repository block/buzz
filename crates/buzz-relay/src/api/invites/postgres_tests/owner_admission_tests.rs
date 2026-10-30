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
    async fn mint_rejects_a_different_verified_owner_without_retrying_or_rebinding() {
        let (state, host, community, agent) = fixture().await;
        let recorded_owner = Keys::generate();
        for key in [&agent, &recorded_owner] {
            state
                .db
                .ensure_user_for_authorization(community, key.public_key().as_bytes())
                .await
                .unwrap();
        }
        assert!(state
            .db
            .set_agent_owner_for_authorization(
                community,
                agent.public_key().as_bytes(),
                recorded_owner.public_key().as_bytes(),
            )
            .await
            .unwrap());
        let other_owner = Keys::generate();
        let tag =
            buzz_sdk::nip_oa::compute_auth_tag(&other_owner, &agent.public_key(), "").unwrap();
        let response = post_with_owner_tag(
            state.clone(),
            &host,
            "/api/invites",
            &agent,
            "{}".into(),
            &tag,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(read_json(response).await["error"], "invite_owner_mismatch");
        assert!(state
            .db
            .is_agent_owner(
                community,
                agent.public_key().as_bytes(),
                recorded_owner.public_key().as_bytes(),
            )
            .await
            .unwrap());
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(state.db.pool())
                .await
                .unwrap();
        assert_eq!(
            count, 0,
            "mismatched credentials must not mint an invitation"
        );

        // Healthy storage and the recorded owner still admit a signed mint.
        let tag =
            buzz_sdk::nip_oa::compute_auth_tag(&recorded_owner, &agent.public_key(), "").unwrap();
        assert_eq!(
            post_with_owner_tag(state, &host, "/api/invites", &agent, "{}".into(), &tag)
                .await
                .status(),
            StatusCode::OK
        );
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

    /// A claim that records the agent's owner makes its ordinary rows show
    /// `bot`, so the rosters of channels it is already in must be republished,
    /// as on first AUTH. No membership change happens in those channels. The
    /// claim also replaces this relay's cached "not owner" answers, which
    /// observer frames trust.
    ///
    /// Mutation: remove the republish or the cache update from
    /// `owner_recorded_by_claim`, or call that helper from only one claim
    /// format → that format keeps `member` or a cached `false` → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_recorded_by_v1_and_v2_claims_updates_owner_caches_and_republishes_rosters() {
        use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};

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
            let (owner, agent) = (Keys::generate(), Keys::generate());
            let agent_bytes = agent.public_key().to_bytes();
            state
                .db
                .ensure_user(community, &agent_bytes)
                .await
                .expect("seed agent user");
            let mut channels = Vec::new();
            for name in ["claim-roster-a", "claim-roster-b"] {
                let channel = state
                    .db
                    .create_channel(
                        community,
                        name,
                        ChannelType::Stream,
                        ChannelVisibility::Open,
                        None,
                        &issuer.public_key().to_bytes(),
                        None,
                    )
                    .await
                    .expect("create channel")
                    .id;
                state
                    .db
                    .add_member(
                        community,
                        channel,
                        &agent_bytes,
                        MemberRole::Member,
                        Some(&issuer.public_key().to_bytes()),
                    )
                    .await
                    .expect("add unowned agent");
                channels.push(channel);
            }

            // An owner check before the claim cached "not owner".
            let owner_key = (
                community,
                agent_bytes.to_vec(),
                owner.public_key().to_bytes().to_vec(),
            );
            let author_key = (community, agent_bytes.to_vec());
            state.observer_owner_cache.insert(owner_key.clone(), false);
            state.author_type_cache.insert(author_key.clone(), false);

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
            assert!(state
                .db
                .is_agent_owner(
                    community,
                    agent.public_key().as_bytes(),
                    owner.public_key().as_bytes()
                )
                .await
                .unwrap());
            assert_eq!(state.observer_owner_cache.get(&owner_key), Some(true));
            assert_eq!(state.author_type_cache.get(&author_key), Some(true));

            let agent_hex = agent.public_key().to_hex();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            for channel in channels {
                loop {
                    let live: Option<serde_json::Value> = sqlx::query_scalar(
                        "SELECT tags FROM events WHERE community_id = $1 AND channel_id = $2 \
                         AND kind = 39002 AND deleted_at IS NULL",
                    )
                    .bind(community.as_uuid())
                    .bind(channel)
                    .fetch_optional(state.db.pool())
                    .await
                    .expect("read live roster");
                    let shows_bot =
                        live.as_ref()
                            .and_then(|tags| tags.as_array())
                            .is_some_and(|tags| {
                                tags.iter().any(|tag| {
                                    tag[0] == "p" && tag[1] == agent_hex.as_str() && tag[3] == "bot"
                                })
                            });
                    if shows_bot {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "code {code}: the roster must show the claimant as bot: {live:?}"
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
}
