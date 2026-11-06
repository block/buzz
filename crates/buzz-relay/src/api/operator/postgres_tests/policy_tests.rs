mod postgres_tests {
    use super::super::*;
    // Included in operator::postgres_tests to reuse the real signed HTTP fixture.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn policy_operator_http_transition_history_guards_and_modes() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let (binding, _, host) = reserved_host_policy();
        let mut state =
            operator_test_state_with_policy(std::slice::from_ref(&operator), Some(binding))
                .await
                .unwrap();
        Arc::get_mut(&mut state).unwrap().nip98_replay =
            Arc::new(SeenOnceReplayGuard(Mutex::new(HashSet::new())));
        let issuer_policy = buzz_auth::IssuerPolicy::new(
            NIP_FI_ISSUER.to_owned(),
            vec!["https://commands.example".into()],
            buzz_auth::TokenClass::DedicatedNipFi,
            buzz_auth::FreshnessClass::OfflineJwt,
            vec![jsonwebtoken::Algorithm::ES256],
            30,
            3600,
            None,
            buzz_auth::JwksSourceContract::new(format!("{NIP_FI_ISSUER}/jwks"), 300, 86400)
                .unwrap(),
        )
        .unwrap();
        let config = Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config);
        config.nip_fi.registry.insert(issuer_policy);
        config.nip_fi.mode = buzz_auth::NipFiMode::Shadow;
        config.nip_fi_upgrades_enabled = false;
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
            .unwrap()
            .unwrap();
        let path = "/operator/communities/nip-fi-policy";
        let current_path = format!("{path}?community_id={}&expected_host={host}", community.id);
        let current =
            fresh_upgrade_request(state.clone(), &operator, "GET", &current_path, None).await;
        assert_eq!(current.headers()[header::CACHE_CONTROL], "no-store");
        let current = read_json(current).await;
        assert_eq!(current["expected_policy_generation"], 0);
        assert_eq!(current["expected_requires_nip_fi"], false);
        let mut body = serde_json::json!({
            "operation_id": Uuid::new_v4(), "community_id": community.id.to_string(),
            "expected_host": host, "actor_pubkey": owner.public_key().to_hex(),
            "expected_owner_pubkey": owner.public_key().to_hex(), "expected_issuer": NIP_FI_ISSUER,
            "requires_nip_fi": true, "expected_policy_generation": 0, "expected_requires_nip_fi": false,
        });
        let disabled = fresh_upgrade_request(
            state.clone(),
            &operator,
            "POST",
            path,
            Some(body.to_string()),
        )
        .await;
        assert_eq!(disabled.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(read_json(disabled).await["code"], "upgrades_disabled");
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).nip_fi_upgrades_enabled = true;
        let mut history = Vec::new();
        for (generation, desired) in [(1, true), (2, false), (3, true)] {
            body["operation_id"] = Uuid::new_v4().to_string().into();
            body["requires_nip_fi"] = desired.into();
            body["expected_policy_generation"] = (generation - 1).into();
            body["expected_requires_nip_fi"] = (!desired).into();
            if desired {
                body["expected_issuer"] = NIP_FI_ISSUER.into();
            } else {
                body.as_object_mut().unwrap().remove("expected_issuer");
            }
            let response = fresh_upgrade_request(
                state.clone(),
                &operator,
                "POST",
                path,
                Some(body.to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let accepted = read_json(response).await;
            assert_eq!(accepted["receipt"]["policy_generation"], generation);
            assert_eq!(accepted["receipt"]["requires_nip_fi"], desired);
            assert_eq!(accepted["effective_required"], desired);
            assert_eq!(
                accepted["receipt"]["state"],
                if desired { "pending" } else { "applied" }
            );
            history.push(body.clone());
            for old in &history {
                let id = old["operation_id"].as_str().unwrap();
                let historical = fresh_upgrade_request(
                    state.clone(),
                    &operator,
                    "GET",
                    &format!("{path}?operation_id={id}"),
                    None,
                )
                .await;
                assert_eq!(historical.status(), StatusCode::OK);
                let historical = read_json(historical).await;
                assert_eq!(
                    historical["receipt"]["current_policy_generation"],
                    generation
                );
                assert_eq!(
                    historical["receipt"]["state"],
                    if old == &body {
                        if desired {
                            "pending"
                        } else {
                            "applied"
                        }
                    } else {
                        "superseded"
                    }
                );
                let legacy = fresh_upgrade_request(
                    state.clone(),
                    &operator,
                    "GET",
                    &format!("/operator/communities/nip-fi-upgrade?operation_id={id}"),
                    None,
                )
                .await;
                assert_eq!(legacy.status(), StatusCode::NOT_FOUND);
                let replay = fresh_upgrade_request(
                    state.clone(),
                    &operator,
                    "POST",
                    path,
                    Some(old.to_string()),
                )
                .await;
                assert_eq!(read_json(replay).await, historical);
            }
            let current = read_json(
                fresh_upgrade_request(state.clone(), &operator, "GET", &current_path, None).await,
            )
            .await;
            assert_eq!(current["expected_policy_generation"], generation);
            assert_eq!(current["expected_requires_nip_fi"], desired);
        }
        let mut stale = history[0].clone();
        stale["operation_id"] = Uuid::new_v4().to_string().into();
        let response = fresh_upgrade_request(
            state.clone(),
            &operator,
            "POST",
            path,
            Some(stale.to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(read_json(response).await["code"], "policy_conflict");
        let mut conflict = history[0].clone();
        conflict["requires_nip_fi"] = false.into();
        conflict.as_object_mut().unwrap().remove("expected_issuer");
        let response = fresh_upgrade_request(
            state.clone(),
            &operator,
            "POST",
            path,
            Some(conflict.to_string()),
        )
        .await;
        assert_eq!(read_json(response).await["code"], "idempotency_conflict");
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).nip_fi_upgrades_enabled =
            false;
        let response = fresh_upgrade_request(
            state.clone(),
            &operator,
            "POST",
            path,
            Some(history[0].to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        for mode in [buzz_auth::NipFiMode::Off, buzz_auth::NipFiMode::Enforce] {
            Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config)
                .nip_fi
                .mode = mode;
            let current = read_json(
                fresh_upgrade_request(state.clone(), &operator, "GET", &current_path, None).await,
            )
            .await;
            assert_eq!(current["effective_required"], mode.restricts());
        }
        // Enforce remains a global floor even after a stored disable.
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).nip_fi_upgrades_enabled = true;
        body["operation_id"] = Uuid::new_v4().to_string().into();
        body["requires_nip_fi"] = false.into();
        body["expected_policy_generation"] = 3.into();
        body["expected_requires_nip_fi"] = true.into();
        body.as_object_mut().unwrap().remove("expected_issuer");
        let response = read_json(
            fresh_upgrade_request(
                state.clone(),
                &operator,
                "POST",
                path,
                Some(body.to_string()),
            )
            .await,
        )
        .await;
        assert_eq!(response["receipt"]["state"], "applied");
        assert_eq!(response["receipt"]["current_requires_nip_fi"], false);
        assert_eq!(response["effective_required"], true);
        let invalid = fresh_upgrade_request(
            state.clone(),
            &operator,
            "GET",
            &format!("{current_path}&operation_id={}", Uuid::new_v4()),
            None,
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        // The control-plane signature and allowlist remain mandatory.
        let outsider = Keys::generate();
        let response = fresh_upgrade_request(state, &outsider, "GET", &current_path, None).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
