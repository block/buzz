mod postgres_tests {
    use super::super::*;

    async fn create(
        state: &Arc<AppState>,
        operator: &Keys,
        owner: &Keys,
        host: &str,
        create_only: bool,
        policy: Option<Value>,
    ) -> axum::response::Response {
        let mut body = serde_json::json!({"host":host,"initial_owner_pubkey":owner.public_key().to_hex(),"create_only":create_only});
        if let Some(policy) = policy {
            body["initial_requires_nip_fi"] = policy;
        }
        signed_operator_request(
            state.clone(),
            operator,
            "POST",
            "/operator/communities",
            Some(body.to_string()),
        )
        .await
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn initial_policy_and_same_owner_replay_journey() {
        let operator = Keys::generate();
        let (binding, reserved, control) = reserved_host_policy();
        let mut state =
            operator_test_state_with_policy(std::slice::from_ref(&operator), Some(binding))
                .await
                .unwrap();
        let owners: Vec<_> = (0..4).map(|_| Keys::generate()).collect();
        let owner = &owners[3];
        assert!(!state.config.nip_fi_changes);
        for (index, policy) in [
            None,
            Some(Value::Null),
            Some(Value::Bool(false)),
            Some(Value::Bool(true)),
        ]
        .into_iter()
        .enumerate()
        {
            let host = control.replacen("acme.", &format!("policy-{index}."), 1);
            let required = policy == Some(Value::Bool(true));
            let owner = &owners[index];
            let response = create(&state, &operator, owner, &host, true, policy).await;
            assert!(response.status().is_success());
            let body = read_json(response).await;
            assert_eq!(body["status"], "created");
            assert_eq!(body["requires_nip_fi"], required);
            let record = state
                .db
                .lookup_community_by_host(&host)
                .await
                .unwrap()
                .unwrap();
            let stored = state
                .db
                .community_auth_policy(record.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.requires_nip_fi, required);
            assert_eq!(stored.policy_generation, i64::from(required));
            let response = create(
                &state,
                &operator,
                owner,
                &host,
                true,
                Some(Value::Bool(!required)),
            )
            .await;
            assert!(response.status().is_success());
            let replay = read_json(response).await;
            assert_eq!(replay["status"], "existed");
            assert_eq!(replay["requires_nip_fi"], required);
            assert_eq!(
                state
                    .db
                    .community_auth_policy(record.id)
                    .await
                    .unwrap()
                    .unwrap(),
                stored
            );
        }
        for value in [false, true] {
            let response = create(
                &state,
                &operator,
                owner,
                &control,
                false,
                Some(Value::Bool(value)),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                read_json(response).await["code"],
                "initial_policy_requires_create_only"
            );
        }
        let refused = create(
            &state,
            &operator,
            owner,
            &reserved,
            true,
            Some(Value::Bool(true)),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(refused).await["code"], "reserved_host");
        let outside = format!("outside-{}.example", Uuid::new_v4().simple());
        let refusal = create(
            &state,
            &operator,
            owner,
            &outside,
            true,
            Some(Value::Bool(true)),
        )
        .await;
        assert_eq!(refusal.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(refusal).await["code"], "nip_fi_binding_required");
        assert!(state
            .db
            .lookup_community_by_host(&outside)
            .await
            .unwrap()
            .is_none());
        let host = control.replacen("acme.", "policy-3.", 1);
        let record = state
            .db
            .lookup_community_by_host(&host)
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE communities SET requires_nip_fi = false, policy_generation = policy_generation + 1 WHERE id = $1")
        .bind(record.id.as_uuid()).execute(state.db.pool()).await.unwrap();
        let disabled = state
            .db
            .community_auth_policy(record.id)
            .await
            .unwrap()
            .unwrap();
        Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).nip_fi_provision_binding =
            None;
        let response = create(
            &state,
            &operator,
            owner,
            &host,
            true,
            Some(Value::Bool(true)),
        )
        .await;
        assert!(response.status().is_success());
        let replay = read_json(response).await;
        assert_eq!(replay["status"], "existed");
        assert_eq!(replay["requires_nip_fi"], false);
        assert_eq!(
            state
                .db
                .community_auth_policy(record.id)
                .await
                .unwrap()
                .unwrap(),
            disabled
        );
        let outside = format!("unbound-{}.example", Uuid::new_v4().simple());
        let response = create(
            &state,
            &operator,
            owner,
            &outside,
            true,
            Some(Value::Bool(true)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(response).await["code"], "nip_fi_binding_required");
        assert!(state
            .db
            .lookup_community_by_host(&outside)
            .await
            .unwrap()
            .is_none());
        let unauthorized = create(
            &state,
            &Keys::generate(),
            owner,
            &outside,
            true,
            Some(Value::Bool(true)),
        )
        .await;
        assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);
    }
}
