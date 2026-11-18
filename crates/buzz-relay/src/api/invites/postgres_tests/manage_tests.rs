mod postgres_tests {
    use super::super::restriction_tests::postgres_tests::fixture;
    use super::super::*;

    fn nip98_get_header(keys: &Keys, url: &str) -> String {
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url]).unwrap(),
                Tag::parse(["method", "GET"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&event).unwrap().as_bytes());
        format!("Nostr {encoded}")
    }

    /// GET `path` signed for `signed_path`, which a test may make differ.
    async fn get_signed(
        state: Arc<AppState>,
        host: &str,
        path: &str,
        signed_path: &str,
        keys: &Keys,
    ) -> axum::response::Response {
        let auth = nip98_get_header(keys, &format!("https://{host}{signed_path}"));
        build_router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .header(header::HOST, host)
                    .header(header::AUTHORIZATION, auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn add_member(
        state: &AppState,
        community: buzz_core::CommunityId,
        keys: &Keys,
        role: &str,
    ) {
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
            .unwrap();
    }

    async fn mint(state: &Arc<AppState>, host: &str, keys: &Keys) -> Value {
        let response = post_json(state.clone(), host, "/api/invites", keys, "{}".into()).await;
        assert_eq!(response.status(), StatusCode::OK);
        read_json(response).await
    }

    async fn revoke(
        state: &Arc<AppState>,
        host: &str,
        keys: &Keys,
        body: Value,
    ) -> (StatusCode, Value) {
        let response = post_json(
            state.clone(),
            host,
            "/api/invites/revoke",
            keys,
            body.to_string(),
        )
        .await;
        (response.status(), read_json(response).await)
    }

    fn assert_no_secret(json: &Value, code: &str) {
        let text = json.to_string();
        assert!(!text.contains(code), "{text}");
        assert!(!text.contains("token_hash"), "{text}");
        assert!(
            !text.contains(&hex::encode(Sha256::digest(code.as_bytes()))),
            "{text}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_lists_and_revokes_by_id_or_code() {
        let (state, host, _, owner) = fixture().await;
        let first = mint(&state, &host, &owner).await;
        let second = mint(&state, &host, &owner).await;
        let first_code = first["code"].as_str().unwrap().to_owned();
        let second_code = second["code"].as_str().unwrap().to_owned();
        let second_id = second["invite_id"]
            .as_str()
            .expect("mint returns invite_id");

        let response =
            get_signed(state.clone(), &host, "/api/invites", "/api/invites", &owner).await;
        assert_eq!(response.status(), StatusCode::OK);
        let listed = read_json(response).await;
        assert_eq!(listed["invites"].as_array().unwrap().len(), 2);
        assert_eq!(listed["invites"][0]["id"], second_id);
        assert_eq!(listed["invites"][0]["status"], "active");
        assert!(listed["invites"][0]["expires_at"].is_i64());
        assert_eq!(listed["next_before"], Value::Null);
        assert_no_secret(&listed, &first_code);
        assert_no_secret(&listed, &second_code);

        let (status, by_code) = revoke(
            &state,
            &host,
            &owner,
            serde_json::json!({ "code": first_code }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(by_code["id"], first["invite_id"]);
        assert_no_secret(&by_code, &first_code);
        let (status, again) = revoke(
            &state,
            &host,
            &owner,
            serde_json::json!({ "id": first["invite_id"] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["revoked_at"], by_code["revoked_at"]);

        let claim = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &Keys::generate(),
            serde_json::json!({ "code": first_code }).to_string(),
        )
        .await;
        assert_ne!(
            claim.status(),
            StatusCode::OK,
            "a revoked link cannot be claimed"
        );
        let claim = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &Keys::generate(),
            serde_json::json!({ "code": second_code }).to_string(),
        )
        .await;
        assert_eq!(claim.status(), StatusCode::OK, "the other link still works");

        let page = get_signed(
            state,
            &host,
            "/api/invites?limit=1",
            "/api/invites?limit=1",
            &owner,
        )
        .await;
        let page = read_json(page).await;
        assert_eq!(page["invites"][0]["id"], second_id);
        assert_eq!(page["next_before"], second_id);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn members_list_nothing_and_revoke_only_their_own() {
        let (state, host, community, owner) = fixture().await;
        let creator = Keys::generate();
        let member = Keys::generate();
        add_member(&state, community, &creator, "admin").await;
        add_member(&state, community, &member, "member").await;
        let invite = mint(&state, &host, &creator).await;
        let owners = mint(&state, &host, &owner).await;
        state
            .db
            .update_relay_member_role(
                community,
                &creator.public_key().to_hex(),
                "member",
                &state.relay_keypair,
            )
            .await
            .map(|write| write.outcome)
            .unwrap();

        for keys in [&creator, &member, &Keys::generate()] {
            let response =
                get_signed(state.clone(), &host, "/api/invites", "/api/invites", keys).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let (status, _) = revoke(
            &state,
            &host,
            &member,
            serde_json::json!({ "id": invite["invite_id"] }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "not the creator");
        let (status, _) = revoke(
            &state,
            &host,
            &creator,
            serde_json::json!({ "id": owners["invite_id"] }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "no longer an admin");
        let (status, _) = revoke(
            &state,
            &host,
            &Keys::generate(),
            serde_json::json!({ "id": invite["invite_id"] }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "not a member");
        let (status, _) = revoke(
            &state,
            &host,
            &creator,
            serde_json::json!({ "id": invite["invite_id"] }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a demoted creator still revokes their own"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revoke_rejects_bad_selectors_and_other_communities() {
        let (state, host, _, owner) = fixture().await;
        let (other_state, other_host, _, other_owner) = fixture().await;
        let invite = mint(&other_state, &other_host, &other_owner).await;
        let code = invite["code"].as_str().unwrap();

        for body in [
            serde_json::json!({}),
            serde_json::json!({ "id": invite["invite_id"], "code": code }),
            serde_json::json!({ "code": "not-a-code" }),
            serde_json::json!({ "code": "v1.legacy" }),
        ] {
            let (status, _) = revoke(&state, &host, &owner, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        for body in [
            serde_json::json!({ "id": invite["invite_id"] }),
            serde_json::json!({ "code": code }),
            serde_json::json!({ "id": Uuid::new_v4() }),
        ] {
            let (status, json) = revoke(&state, &host, &owner, body).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_no_secret(&json, code);
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn list_signature_covers_the_query() {
        let (state, host, _, owner) = fixture().await;
        for (path, signed) in [
            ("/api/invites?limit=200", "/api/invites?limit=10"),
            ("/api/invites?limit=10", "/api/invites"),
        ] {
            let response = get_signed(state.clone(), &host, path, signed, &owner).await;
            assert!(
                matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ),
                "{path} signed as {signed}: {}",
                response.status()
            );
        }
        let response = get_signed(
            state,
            &host,
            "/api/invites?limit=0",
            "/api/invites?limit=0",
            &owner,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
