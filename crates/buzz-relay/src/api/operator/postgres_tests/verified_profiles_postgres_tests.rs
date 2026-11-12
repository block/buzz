mod postgres_tests {
    use super::super::*;
    use crate::nip_fi_core::test_support::{
        assertion_signing_key, assertion_verifier, communities_for, TEST_COMMUNITY_URI, TEST_HOST,
        TEST_ISSUER, TEST_KID,
    };
    use buzz_auth::{IssuerCapacity, NipFiDenyMap, NipFiMode, TokenClass};
    use buzz_db::verified_profiles::ProfilePolicy;

    const PATH: &str = "/operator/communities/verified-profile";

    fn claims(key: &Keys, name: &str, iat: i64) -> Value {
        serde_json::json!({
            "iss": TEST_ISSUER, "aud": TEST_COMMUNITY_URI, "sub": "fictional-person",
            "iat": iat, "exp": chrono::Utc::now().timestamp() + 600,
            "nostr_pubkey": key.public_key().to_hex(),
            "nostr_public_profile": {"display_name": name, "username": "alice", "full_name": name},
        })
    }

    fn signed_assertion(claims: &Value) -> String {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.typ = Some("nip-fi+jwt".into());
        header.kid = Some(TEST_KID.into());
        jsonwebtoken::encode(&header, claims, &assertion_signing_key().0).unwrap()
    }

    fn body(community: CommunityId, claims: &Value) -> Value {
        serde_json::json!({"community_id": community.to_string(), "expected_host": TEST_HOST,
        "assertion": signed_assertion(claims)})
    }

    async fn post(state: &Arc<AppState>, operator: &Keys, request: Value) -> (StatusCode, Value) {
        let response = signed_operator_request(
            state.clone(),
            operator,
            "POST",
            PATH,
            Some(request.to_string()),
        )
        .await;
        (response.status(), read_json(response).await)
    }

    async fn fixture(operator: &Keys, member: &Keys) -> (Arc<AppState>, CommunityId) {
        let mut state = operator_test_state_with_policy_and_redis(
            std::slice::from_ref(operator),
            None,
            &std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
        )
        .await
        .expect("Postgres fixture must be available; never silently skip");
        let inner = Arc::get_mut(&mut state).unwrap();
        Arc::make_mut(&mut inner.config).nip_fi.mode = NipFiMode::Enforce;
        Arc::make_mut(&mut inner.config).nip_fi.communities = communities_for(&[TEST_ISSUER]);
        Arc::make_mut(&mut inner.config).relay_private_key =
            Some(inner.relay_keypair.secret_key().to_secret_hex());
        Arc::make_mut(&mut inner.config).nip_fi.profile_policies = vec![ProfilePolicy {
            issuer: TEST_ISSUER.into(),
            label: Some("Example directory".into()),
            refresh_seconds: 3600,
        }];
        inner.nip_fi_verifier = Some(Arc::new(assertion_verifier(TokenClass::DedicatedNipFi)));
        inner.nip_fi_deny_map = Some(Arc::new(NipFiDenyMap::new(
            16,
            vec![IssuerCapacity {
                issuer: TEST_ISSUER.into(),
                capacity: 16,
            }],
        )));
        let community = state
            .db
            .create_community_with_owner_and_nip_fi_policy(
                TEST_HOST,
                &member.public_key().to_hex(),
                &[TEST_ISSUER.to_owned()],
                true,
                &state.relay_keypair,
            )
            .await
            .unwrap();
        let buzz_db::community::CreateCommunityWithOwnerResult::Created(created) =
            community.outcome
        else {
            panic!("fixture community creation failed");
        };
        (state, created.id)
    }

    async fn stored(state: &AppState, community: CommunityId, kind: i32) -> Vec<nostr::Event> {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let rows: Vec<Value> =
            sqlx::query_scalar("SELECT event FROM nip_fi_profile_heads WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_all(&pool)
                .await
                .unwrap();
        if kind == 30382 {
            return rows
                .into_iter()
                .map(|v| serde_json::from_value(v).unwrap())
                .collect();
        }
        state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![kind]),
                ..EventQuery::for_community(community)
            })
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    fn name(event: &nostr::Event) -> Option<&str> {
        event.tags.iter().find_map(|t| match t.as_slice() {
            [tag, value, ..] if tag == "display_name" => Some(value.as_str()),
            _ => None,
        })
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verified_profile_delivery_renames_without_impersonating_or_replacing_user_profiles() {
        let operator = Keys::generate();
        let member = Keys::generate();
        let (mut state, community) = fixture(&operator, &member).await;
        let selected = EventBuilder::new(
            Kind::Metadata,
            r#"{"display_name":"My chosen name","picture":"https://example.org/avatar.png"}"#,
        )
        .sign_with_keys(&member)
        .unwrap();

        let before_member = state
            .db
            .get_relay_member(community, &member.public_key().to_hex())
            .await
            .unwrap()
            .unwrap()
            .role;
        let now = chrono::Utc::now().timestamp();
        let first = claims(&member, "Alice Example", now - 30);
        let renamed = claims(&member, "Élodie Example", now - 10);
        let (status, result) = post(&state, &operator, body(community, &first)).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["status"], "published");
        let event: nostr::Event = serde_json::from_value(result["event"].clone()).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, state.relay_keypair.public_key());
        assert_ne!(event.pubkey, member.public_key());
        assert_eq!(event.kind, Kind::Custom(30382));
        assert_eq!(event.content, "");
        for tag in ["p", "d"] {
            assert!(event
                .tags
                .iter()
                .any(|t| t.as_slice() == [tag, &member.public_key().to_hex()]));
        }
        assert_eq!(stored(&state, community, 30382).await[0].id, event.id);

        assert!(
            stored(&state, community, 0).await.is_empty(),
            "no employee event synthesized"
        );
        state
            .db
            .insert_event(community, &selected, None)
            .await
            .unwrap();
        let (_, result) = post(&state, &operator, body(community, &renamed)).await;
        assert_eq!(result["status"], "published");
        let renamed_event: nostr::Event = serde_json::from_value(result["event"].clone()).unwrap();
        assert_eq!(name(&renamed_event), Some("Élodie Example"));
        assert!(renamed_event.created_at > event.created_at);
        // Older notifications, duplicate deliveries, and ambiguous same-second
        // names cannot roll the stored name back.
        for retry in [
            first,
            renamed.clone(),
            claims(&member, "Stale variant", now - 10),
        ] {
            let (status, result) = post(&state, &operator, body(community, &retry)).await;
            assert_eq!(status, StatusCode::OK, "{result}");
            assert_eq!(result["status"], "not_written");
            assert_eq!(
                stored(&state, community, 30382).await[0].id,
                renamed_event.id
            );
        }
        // Same assertion may adopt a newly configured public issuer label.
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config)
            .nip_fi
            .profile_policies[0]
            .label = Some("Renamed directory".into());
        let (_, result) = post(&state, &operator, body(community, &renamed)).await;
        assert_eq!(result["status"], "published");
        assert_eq!(stored(&state, community, 0).await, vec![selected]);
        assert_eq!(
            state
                .db
                .get_relay_member(community, &member.public_key().to_hex())
                .await
                .unwrap()
                .unwrap()
                .role,
            before_member
        );

        // Exercise the real HTTP transport as well as the real router/storage path.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = build_router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let request = body(community, &claims(&member, "Alice Updated", now)).to_string();
        let auth = nip98_auth_header(
            &operator,
            &format!("http://{INGRESS_HOST}{PATH}"),
            "POST",
            Some(request.as_bytes()),
        );
        let response = reqwest::Client::new()
            .post(format!("http://{address}{PATH}"))
            .header("Host", INGRESS_HOST)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .body(request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let result: Value = response.json().await.unwrap();
        server.abort();
        assert_eq!(result["status"], "published");
        assert_eq!(
            name(&stored(&state, community, 30382).await[0]),
            Some("Alice Updated")
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verified_profile_delivery_enforces_operator_issuer_tenant_and_existing_member_boundaries(
    ) {
        let operator = Keys::generate();
        let member = Keys::generate();
        let (mut state, community) = fixture(&operator, &member).await;
        let now = chrono::Utc::now().timestamp();
        let valid = claims(&member, "Alice Example", now - 10);
        assert_eq!(
            post(&state, &Keys::generate(), body(community, &valid))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        for (field, value) in [
            ("aud", serde_json::json!("https://other.example")),
            ("iss", serde_json::json!("https://untrusted.example")),
            ("exp", serde_json::json!(now - 3600)),
            ("nostr_pubkey", serde_json::json!("invalid")),
            (
                "nostr_owner_pubkey",
                serde_json::json!(Keys::generate().public_key().to_hex()),
            ),
            (
                "nostr_public_profile",
                serde_json::json!({"display_name":"Alice"}),
            ),
            ("nostr_public_profile", Value::Null),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert_eq!(
                post(&state, &operator, body(community, &bad)).await.0,
                StatusCode::FORBIDDEN,
                "{field}"
            );
        }
        // Even a valid dedicated agent assertion cannot publish human names.
        let mut agent = valid.clone();
        agent["nostr_owner_pubkey"] = Keys::generate().public_key().to_hex().into();
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.typ = Some("nip-fi-agent+jwt".into());
        header.kid = Some(TEST_KID.into());
        let mut agent_request = body(community, &agent);
        agent_request["assertion"] =
            jsonwebtoken::encode(&header, &agent, &assertion_signing_key().0)
                .unwrap()
                .into();
        let (status, result) = post(&state, &operator, agent_request).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(result["code"], "public_profile_required");
        let mut bad = body(community, &valid);
        bad["assertion"] = "invalid.jwt.signature".into();
        assert_eq!(post(&state, &operator, bad).await.0, StatusCode::FORBIDDEN);
        let mut other_host = body(community, &valid);
        other_host["expected_host"] = "other.example".into();
        assert_eq!(
            post(&state, &operator, other_host).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            post(
                &state,
                &operator,
                body(CommunityId::from_uuid(Uuid::new_v4()), &valid)
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        for disabled in ["mode", "key", "policy", "binding", "verifier", "deny"] {
            let inner = Arc::get_mut(&mut state).unwrap();
            match disabled {
                "mode" => Arc::make_mut(&mut inner.config).nip_fi.mode = NipFiMode::Off,
                "key" => Arc::make_mut(&mut inner.config).relay_private_key = None,
                "policy" => {
                    Arc::make_mut(&mut inner.config).nip_fi.profile_policies[0].label = None
                }
                "binding" => {
                    Arc::make_mut(&mut inner.config).nip_fi.communities =
                        communities_for(&["https://other-issuer.example"])
                }
                "verifier" => inner.nip_fi_verifier = None,
                _ => inner.nip_fi_deny_map = None,
            }
            assert_ne!(
                post(&state, &operator, body(community, &valid)).await.0,
                StatusCode::OK,
                "{disabled}"
            );
            let inner = Arc::get_mut(&mut state).unwrap();
            Arc::make_mut(&mut inner.config).nip_fi.mode = NipFiMode::Enforce;
            Arc::make_mut(&mut inner.config).nip_fi.communities = communities_for(&[TEST_ISSUER]);
            Arc::make_mut(&mut inner.config).relay_private_key =
                Some(inner.relay_keypair.secret_key().to_secret_hex());
            Arc::make_mut(&mut inner.config).nip_fi.profile_policies[0].label =
                Some("Example directory".into());
            inner.nip_fi_verifier = Some(Arc::new(assertion_verifier(TokenClass::DedicatedNipFi)));
            inner.nip_fi_deny_map = Some(Arc::new(NipFiDenyMap::new(
                16,
                vec![IssuerCapacity {
                    issuer: TEST_ISSUER.into(),
                    capacity: 16,
                }],
            )));
        }
        // A verified assertion alone never adds its key to relay membership.
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).require_relay_membership =
            false;
        let outsider = Keys::generate();
        let (_, result) = post(
            &state,
            &operator,
            body(community, &claims(&outsider, "Outside Person", now)),
        )
        .await;
        assert_eq!(result["status"], "not_written");
        assert!(state
            .db
            .get_relay_member(community, &outsider.public_key().to_hex())
            .await
            .unwrap()
            .is_none());
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        sqlx::query("INSERT INTO community_bans (community_id, pubkey, actor_pubkey, banned) VALUES ($1, $2, $2, true)")
            .bind(community.as_uuid()).bind(member.public_key().to_bytes().to_vec())
            .execute(&pool).await.unwrap();
        let (_, result) = post(&state, &operator, body(community, &valid)).await;
        assert_eq!(result["status"], "not_written");
        sqlx::query("DELETE FROM community_bans WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        // This per-test database fault must propagate, never become success.
        sqlx::query("ALTER TABLE nip_fi_profile_heads RENAME TO unavailable_profile_heads")
            .execute(&pool)
            .await
            .unwrap();
        let (status, result) = post(&state, &operator, body(community, &valid)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(result["code"], "publication_unavailable");
        sqlx::query("ALTER TABLE unavailable_profile_heads RENAME TO nip_fi_profile_heads")
            .execute(&pool)
            .await
            .unwrap();
        state
            .nip_fi_deny_map
            .as_ref()
            .unwrap()
            .merge_cross_pod_deny(
                TEST_ISSUER,
                &member.public_key(),
                chrono::Utc::now() + chrono::Duration::seconds(60),
                chrono::Utc::now(),
            );
        assert_eq!(
            post(&state, &operator, body(community, &valid)).await.0,
            StatusCode::FORBIDDEN
        );
        assert!(stored(&state, community, 30382).await.is_empty());

        // Signature binds the entire request, and the replay fence is still used.
        Arc::get_mut(&mut state).unwrap().nip98_replay =
            Arc::new(SeenOnceReplayGuard(Mutex::new(HashSet::new())));
        let original = body(community, &valid).to_string();
        let auth = nip98_auth_header(
            &operator,
            &format!("http://{INGRESS_HOST}{PATH}"),
            "POST",
            Some(original.as_bytes()),
        );
        let tampered = body(community, &claims(&member, "Injected name", now)).to_string();
        let response =
            operator_request_with_auth(state.clone(), "POST", PATH, Some(tampered), auth.clone())
                .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = operator_request_with_auth(
            state.clone(),
            "POST",
            PATH,
            Some(original.clone()),
            auth.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN); // valid operator, denied subject
        let response =
            operator_request_with_auth(state.clone(), "POST", PATH, Some(original), auth).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
