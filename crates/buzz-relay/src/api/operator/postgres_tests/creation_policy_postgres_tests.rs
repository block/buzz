use super::*;

fn set_creation(state: &mut Arc<AppState>, enabled: bool) {
    Arc::make_mut(&mut Arc::get_mut(state).expect("exclusive test state").config)
        .multi_community_relay = enabled;
}

async fn policy_state(operator: &Keys) -> Arc<AppState> {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("creation policy tests require Postgres");
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        buzz_db::Db::from_pool(pool)
            .migrate()
            .await
            .expect("test schema");
    }
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    operator_test_state_with_policy_and_redis(std::slice::from_ref(operator), None, &redis_url)
        .await
        .expect("test state")
}

async fn legacy_request(
    state: Arc<AppState>,
    operator: &Keys,
    host: &str,
    owner: Option<&Keys>,
) -> axum::response::Response {
    let mut body = serde_json::json!({"host": host});
    if let Some(owner) = owner {
        body["initial_owner_pubkey"] = owner.public_key().to_hex().into();
    }
    // Deliberately omit create_only to exercise the compatibility default.
    signed_operator_request(
        state,
        operator,
        "POST",
        "/operator/communities",
        Some(body.to_string()),
    )
    .await
}

async fn owned(state: Arc<AppState>, operator: &Keys, owner: &Keys) -> Value {
    let path = format!(
        "/operator/communities?owner_pubkey={}",
        owner.public_key().to_hex()
    );
    let response = signed_operator_request(state, operator, "GET", &path, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    read_json(response).await
}

mod postgres_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn single_mode_serves_only_canonical_and_rejects_extra_target_enrollment() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let mut state = policy_state(&operator).await;
        let canonical = format!("canonical-{}.example", Uuid::new_v4().simple());
        let extra = format!("extra-{}.example", Uuid::new_v4().simple());
        for host in [&canonical, &extra] {
            let _fixture = state
                .db
                .create_community_with_owner(
                    host,
                    &owner.public_key().to_hex(),
                    &state.relay_keypair,
                )
                .await
                .expect("fixture");
        }
        let extra_id = state
            .db
            .lookup_community_by_host_for_management(&extra)
            .await
            .unwrap()
            .unwrap()
            .id;
        let config = Arc::make_mut(&mut Arc::get_mut(&mut state).expect("exclusive").config);
        config.relay_url = format!("wss://{canonical}");
        set_creation(&mut state, false);
        let app = build_router(state.clone());
        for host in [&canonical, &extra] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/")
                        .header("host", host.as_str())
                        .header("accept", "application/nostr+json")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if host == &canonical {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                }
            );
        }
        for path in [
            "/query",
            "/api/invites/claim",
            "/upload",
            "/git/owner/repo/info/refs",
            "/huddle/id/audio",
            "/invite/token",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("host", extra.as_str())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        // UUID selection through the authenticated operator is not an ordinary
        // serving escape: even a valid owner cannot mint an extra's invite.
        let body = serde_json::json!({"community_id":extra_id.to_string(), "actor_pubkey":owner.public_key().to_hex()}).to_string();
        let response = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities/invites",
            Some(body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(read_json(response).await["code"], "community_not_served");
        // A valid invite for the extra, presented on the canonical Host, may
        // never enroll the caller into the extra (host is authoritative).
        let invite = state
            .db
            .mint_relay_invite_as_admin(extra_id, &owner.public_key().to_hex(), 3600, Some(1))
            .await
            .expect("invite fixture");
        let joiner = Keys::generate();
        let body = serde_json::json!({"code":invite.code, "community_id":extra_id.to_string(), "communityHost":extra}).to_string();
        let path = "/api/invites/claim";
        let url = format!("https://{canonical}{path}");
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("host", canonical)
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        nip98_auth_header(&joiner, &url, "POST", Some(body.as_bytes())),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!response.status().is_success());
        assert!(state
            .db
            .get_relay_member(extra_id, &joiner.public_key().to_hex())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_creation_rejects_both_modes_retries_and_concurrency_without_rows() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let mut state = policy_state(&operator).await;
        let (policy, _, host) = reserved_host_policy();
        Arc::make_mut(&mut Arc::get_mut(&mut state).expect("exclusive").config)
            .nip_fi_provision_binding = Some(policy);
        set_creation(&mut state, false);
        for _ in 0..2 {
            let (create, legacy) = tokio::join!(
                provision_community(state.clone(), &operator, &host, &owner),
                legacy_request(state.clone(), &operator, &host, Some(&owner)),
            );
            for response in [create, legacy] {
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
                assert_eq!(
                    read_json(response).await["code"],
                    "community_creation_disabled"
                );
            }
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM communities WHERE host = $1")
            .bind(&host)
            .fetch_one(state.db.pool())
            .await
            .expect("community count");
        assert_eq!(count, 0);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM relay_members WHERE pubkey = $1")
            .bind(owner.public_key().to_hex())
            .fetch_one(state.db.pool())
            .await
            .expect("owner count");
        assert_eq!(count, 0);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM community_nip_fi_issuers i JOIN communities c ON c.id = i.community_id WHERE c.host = $1")
            .bind(&host).fetch_one(state.db.pool()).await.expect("issuer count");
        assert_eq!(count, 0);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE kind = $1 AND tags::text LIKE $2",
        )
        .bind(KIND_NIP43_MEMBERSHIP_LIST as i32)
        .bind(format!("%{}%", owner.public_key().to_hex()))
        .fetch_one(state.db.pool())
        .await
        .expect("snapshot count");
        assert_eq!(count, 0);
        let outsider = Keys::generate();
        let response = legacy_request(state, &outsider, &host, None).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_ne!(
            read_json(response).await["code"],
            "community_creation_disabled"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_creation_preserves_existing_convergence_and_capability_quota() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let mut state = policy_state(&operator).await;
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let _fixture = state
            .db
            .create_community_with_owner(&host, &owner.public_key().to_hex(), &state.relay_keypair)
            .await
            .expect("seed existing-community fixture");
        let before = owned(policy_state(&operator).await, &operator, &owner).await;
        assert_eq!(before["can_create"], true);
        let id = state
            .db
            .lookup_community_by_host_for_management(&host)
            .await
            .expect("lookup")
            .expect("row")
            .id;
        state
            .db
            .bind_nip_fi_issuers(id, &[NIP_FI_ISSUER.to_owned()])
            .await
            .expect("binding");
        Arc::make_mut(&mut Arc::get_mut(&mut state).expect("exclusive").config).relay_url =
            format!("wss://{host}");
        set_creation(&mut state, false);
        for _ in 0..2 {
            let response = legacy_request(state.clone(), &operator, &host, Some(&owner)).await;
            assert_eq!(response.status(), StatusCode::OK);
            let json = read_json(response).await;
            assert_eq!(json["status"], "existed");
            assert_eq!(json["community_id"], id.to_string());
        }
        let issuers: Vec<String> = sqlx::query_scalar(
            "SELECT issuer FROM community_nip_fi_issuers WHERE community_id = $1",
        )
        .bind(id.as_uuid())
        .fetch_all(state.db.pool())
        .await
        .expect("bindings");
        assert_eq!(issuers, vec![NIP_FI_ISSUER.to_owned()]);
        let after = owned(state.clone(), &operator, &owner).await;
        assert_eq!(after["can_create"], false);
        assert_eq!(after["quota_used"], before["quota_used"]);
        assert_eq!(after["quota_limit"], before["quota_limit"]);
        assert_eq!(after["communities"], before["communities"]);
        assert_eq!(
            provision_community(state, &operator, &host, &owner)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_creation_preserves_archive_management_and_owner_refusal() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let mut state = policy_state(&operator).await;
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let _fixture = state
            .db
            .create_community_with_owner(&host, &owner.public_key().to_hex(), &state.relay_keypair)
            .await
            .expect("seed existing-community fixture");
        set_creation(&mut state, false);
        let body = serde_json::json!({"host":host, "owner_pubkey":owner.public_key().to_hex()})
            .to_string();
        let archived = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities/archive",
            Some(body.clone()),
        )
        .await;
        assert_eq!(archived.status(), StatusCode::OK);
        let existing = legacy_request(state.clone(), &operator, &host, None).await;
        assert_eq!(existing.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            read_json(existing).await["code"],
            "community_creation_disabled"
        );
        assert_eq!(
            legacy_request(state.clone(), &operator, &host, Some(&owner))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let restored = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities/unarchive",
            Some(body),
        )
        .await;
        assert_eq!(restored.status(), StatusCode::OK);
        assert_eq!(
            legacy_request(state, &operator, &host, Some(&owner))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_creation_keeps_canonical_bootstrap_and_unknown_hosts_unmapped() {
        let operator = Keys::generate();
        let mut state = policy_state(&operator).await;
        set_creation(&mut state, false);
        let canonical = format!("canonical-{}.example", Uuid::new_v4().simple());
        let seeded = crate::handlers::community_provisioning::ensure_deployment_community(
            &state.db, None, &canonical,
        )
        .await
        .expect("bootstrap")
        .expect("row");
        assert!(seeded.created);
        let again = crate::handlers::community_provisioning::ensure_deployment_community(
            &state.db, None, &canonical,
        )
        .await
        .expect("restart")
        .expect("row");
        assert_eq!(seeded.id, again.id);
        assert!(!again.created);
        let missing = format!("unknown-{}.example", Uuid::new_v4().simple());
        assert!(crate::tenant::bind_community(&state.db, &missing)
            .await
            .is_err());
        assert!(state
            .db
            .lookup_community_by_host_for_management(&missing)
            .await
            .expect("lookup")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_legacy_preserves_deletion_and_tombstone_refusals() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let mut state = policy_state(&operator).await;
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let _fixture = state
            .db
            .create_community_with_owner(&host, &owner.public_key().to_hex(), &state.relay_keypair)
            .await
            .expect("seed existing-community fixture");
        let id = state
            .db
            .lookup_community_by_host_for_management(&host)
            .await
            .expect("lookup")
            .expect("row")
            .id;
        set_creation(&mut state, false);
        archive_for_owner_deletion(&state, &host, &owner).await;
        let response = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&host, &owner, Uuid::new_v4())),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        // Queue deletion while this is an extra community: the deployment
        // community is protected from archival and deletion. Then select it
        // as canonical to exercise legacy convergence against pending deletion.
        Arc::make_mut(&mut Arc::get_mut(&mut state).expect("exclusive").config).relay_url =
            format!("wss://{host}");
        let rotation =
            legacy_request(state.clone(), &operator, &host, Some(&Keys::generate())).await;
        assert_eq!(rotation.status(), StatusCode::CONFLICT);
        assert!(read_json(rotation).await["error"]
            .as_str()
            .expect("error")
            .contains("deletion is pending"));
        for lifecycle in ["quiescing", "fenced", "tombstone"] {
            let mut tx = state.db.pool().begin().await.expect("fixture transaction");
            sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '0', true)")
                .bind(id.to_string()).execute(&mut *tx).await.expect("executor fixture");
            sqlx::query("UPDATE communities SET deletion_state = $2 WHERE id = $1")
                .bind(id.as_uuid())
                .bind(lifecycle)
                .execute(&mut *tx)
                .await
                .expect("lifecycle fixture");
            tx.commit().await.expect("commit fixture");
            assert!(matches!(
                state.db.existing_configured_community(&host).await,
                Err(buzz_db::DbError::AccessDenied(_))
            ));
            let denied = legacy_request(state.clone(), &operator, &host, None).await;
            // Preserve the old ensure persistence refusal rather than report missing.
            assert_eq!(denied.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_creation_preserves_admin_joined_transfer_and_binding_reads() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let recipient = Keys::generate();
        let mut state = policy_state(&operator).await;
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let _fixture = state
            .db
            .create_community_with_owner(&host, &owner.public_key().to_hex(), &state.relay_keypair)
            .await
            .expect("fixture");
        let id = state
            .db
            .lookup_community_by_host_for_management(&host)
            .await
            .expect("lookup")
            .expect("row")
            .id;
        set_creation(&mut state, false);
        Arc::make_mut(&mut Arc::get_mut(&mut state).expect("exclusive").config).admin =
            Some(crate::config::AdminConfig {
                host: INGRESS_HOST.to_owned(),
                auth: crate::config::AdminAuth::Nip98,
                web_dir: None,
            });
        let path = "/api/admin/v1/probe";
        let url = format!("https://{INGRESS_HOST}{path}");
        let probe = operator_request_with_auth(
            state.clone(),
            "GET",
            path,
            None,
            nip98_auth_header(&operator, &url, "GET", None),
        )
        .await;
        assert_eq!(probe.status(), StatusCode::OK);
        assert_eq!(read_json(probe).await["role"], "operator");
        // An authenticated administrator can still inspect and moderate an
        // extra community through the admin authority, despite serving denial.
        let path = format!(
            "/api/admin/v1/members/{}?communityHost={host}",
            owner.public_key().to_hex()
        );
        let url = format!("https://{INGRESS_HOST}{path}");
        let read = operator_request_with_auth(
            state.clone(),
            "GET",
            &path,
            None,
            nip98_auth_header(&operator, &url, "GET", None),
        )
        .await;
        assert_eq!(read.status(), StatusCode::OK);
        let target = Keys::generate();
        let path = format!(
            "/api/admin/v1/members/{}/ban?communityHost={host}",
            target.public_key().to_hex()
        );
        let url = format!("https://{INGRESS_HOST}{path}");
        let body = serde_json::json!({"requestId": Uuid::new_v4(), "reason":"cleanup"}).to_string();
        let ban = operator_request_with_auth(
            state.clone(),
            "POST",
            &path,
            Some(body.clone()),
            nip98_auth_header(&operator, &url, "POST", Some(body.as_bytes())),
        )
        .await;
        assert!(
            ban.status().is_success(),
            "admin direct action: {}",
            read_json(ban).await
        );
        let path = "/api/admin/v1/probe";
        let url = format!("https://{INGRESS_HOST}{path}");
        let stranger = operator_request_with_auth(
            state.clone(),
            "GET",
            path,
            None,
            nip98_auth_header(&Keys::generate(), &url, "GET", None),
        )
        .await;
        assert_eq!(stranger.status(), StatusCode::FORBIDDEN);
        let path = format!(
            "/operator/communities/joined?member_pubkey={}",
            owner.public_key().to_hex()
        );
        let joined = signed_operator_request(state.clone(), &operator, "GET", &path, None).await;
        assert_eq!(joined.status(), StatusCode::OK);
        assert!(read_json(joined).await.to_string().contains(&host));
        let path = format!("/operator/communities/nip-fi-binding?host={host}");
        assert_eq!(
            signed_operator_request(state.clone(), &operator, "GET", &path, None)
                .await
                .status(),
            StatusCode::OK
        );
        let path = format!(
            "/operator/communities/availability?host=free-{}.example",
            Uuid::new_v4().simple()
        );
        let available = signed_operator_request(state.clone(), &operator, "GET", &path, None).await;
        assert_eq!(available.status(), StatusCode::OK);
        assert_eq!(read_json(available).await["available"], true);
        let path = format!(
            "/operator/communities/nip-fi-upgrade?operation_id={}",
            Uuid::new_v4()
        );
        assert_eq!(
            signed_operator_request(state.clone(), &operator, "GET", &path, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let transfer = serde_json::json!({"community_id": id.to_string(), "expected_owner_pubkey":owner.public_key().to_hex(), "new_owner_pubkey":recipient.public_key().to_hex()}).to_string();
        assert_eq!(
            signed_operator_request(
                state.clone(),
                &operator,
                "POST",
                "/operator/communities/transfer",
                Some(transfer)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_snapshot_roles(
            &state,
            id,
            &[
                (&owner.public_key().to_hex(), "member"),
                (&recipient.public_key().to_hex(), "owner"),
            ],
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn enabled_policy_keeps_legacy_missing_host_creation() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let state = policy_state(&operator).await;
        assert!(state.config.multi_community_relay);
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let created = legacy_request(state.clone(), &operator, &host, Some(&owner)).await;
        assert_eq!(created.status(), StatusCode::OK);
        assert_eq!(read_json(created).await["status"], "created");
        let again = legacy_request(state.clone(), &operator, &host, Some(&owner)).await;
        assert_eq!(again.status(), StatusCode::OK);
        assert_eq!(read_json(again).await["status"], "existed");
    }
}
