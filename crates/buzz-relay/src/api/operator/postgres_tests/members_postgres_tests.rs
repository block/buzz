mod postgres_tests {
    //! Operator-API member routes: allowlisted NIP-98 over the operator origin,
    //! with the signed query and body binding the request, then the same
    //! lifecycle as the admin-host routes. The test state configures no admin
    //! host, and every request arrives on the operator ingress `Host`.

    use super::super::*;

    const ROOT: [u8; 32] = [0xb1; 32];
    const AGENT: [u8; 32] = [0xb2; 32];
    const BASE: &str = "/operator/communities/members";

    async fn identity_archive_state(operator: &Keys) -> Arc<AppState> {
        let redis = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        operator_test_state_with_policy_and_redis(std::slice::from_ref(operator), None, &redis)
            .await
            .expect("test state")
    }

    /// A community with a root, its agent and the agent's active workflow.
    async fn community(state: &AppState) -> (CommunityId, String) {
        let host = format!("retire-{}.example", Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("community")
            .id;
        for (key, owner, name) in [(ROOT, None, "Alex Root"), (AGENT, Some(ROOT), "Agent")] {
            sqlx::query(
                "INSERT INTO users (community_id, pubkey, agent_owner_pubkey, display_name) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(community.as_uuid())
            .bind(key.as_slice())
            .bind(owner.as_ref().map(|o| o.as_slice()))
            .bind(name)
            .execute(state.db.pool())
            .await
            .expect("seed user");
        }
        sqlx::query(
            "INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash) \
             VALUES ($1, $2, 'example', '{}', $2)",
        )
        .bind(community.as_uuid())
        .bind(AGENT.as_slice())
        .execute(state.db.pool())
        .await
        .expect("seed workflow");
        (community, host)
    }

    async fn event_tags(state: &AppState, community: CommunityId, kind: i32) -> Vec<Value> {
        sqlx::query_scalar(
            "SELECT tags FROM events WHERE community_id = $1 AND kind = $2 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(kind)
        .fetch_all(state.db.pool())
        .await
        .expect("event tags")
    }

    /// Send `target` with `auth`, as the operator ingress delivers it.
    async fn send(
        state: &Arc<AppState>,
        method: &str,
        target: &str,
        body: Option<Vec<u8>>,
        auth: String,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(target)
            .header(header::HOST, INGRESS_HOST)
            .header(header::AUTHORIZATION, auth);
        if body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let response = build_router(state.clone())
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, Body::from))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.expect("body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Sign the exact received URL (origin, path, raw query) and body.
    async fn call(
        state: &Arc<AppState>,
        keys: &Keys,
        method: &str,
        target: &str,
        body: Option<&Value>,
    ) -> (StatusCode, Value) {
        let body = body.map(|b| serde_json::to_vec(b).expect("body"));
        let auth = nip98_auth_header(
            keys,
            &format!("http://{INGRESS_HOST}{target}"),
            method,
            body.as_deref(),
        );
        send(state, method, target, body, auth).await
    }

    fn lifecycle_code(json: &Value) -> Option<&str> {
        json["error"]["code"].as_str()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_archive_and_unarchive_repair_visibility_on_retry() {
        let operator = Keys::generate();
        let state = identity_archive_state(&operator).await;
        let (c, host) = community(&state).await;
        let (other, other_host) = community(&state).await;
        let root = hex::encode(ROOT);
        let agent = hex::encode(AGENT);

        let search = format!("{BASE}/search?communityHost={host}&q=alex%20root&limit=5");
        let (status, page) = call(&state, &operator, "GET", &search, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["items"][0]["pubkey"], root);

        let ownership = format!("{BASE}/{root}/ownership?communityHost={host}");
        let (status, inventory) = call(&state, &operator, "GET", &ownership, None).await;
        assert_eq!(status, StatusCode::OK, "{inventory}");
        let version = inventory["inventoryVersion"].as_str().expect("version");
        let archive = format!("{BASE}/{root}/archive?communityHost={host}");
        let unarchive = format!("{BASE}/{root}/unarchive?communityHost={host}");
        let archive_body = serde_json::json!({
            "requestId": Uuid::new_v4(), "reason": "duplicate", "inventoryVersion": version,
        });

        crate::handlers::side_effects::publish_test_hooks::fail_next(c);
        let other_archive = format!("{BASE}/{root}/archive?communityHost={other_host}");
        let (status, bystander) = call(
            &state,
            &operator,
            "POST",
            &other_archive,
            Some(&archive_body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{bystander}");
        assert_eq!(event_tags(&state, other, 13535).await.len(), 1);

        let (status, first) = call(&state, &operator, "POST", &archive, Some(&archive_body)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{first}");
        assert_eq!(first["stages"]["visibility"], "pending");
        assert_eq!(first["identities"][0]["changed"], true);
        assert_eq!(first["identities"][1]["changed"], true);
        assert!(event_tags(&state, c, 13535).await.is_empty());

        let (status, retry) = call(&state, &operator, "POST", &archive, Some(&archive_body)).await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["requestId"], archive_body["requestId"]);
        assert_eq!(retry["stages"]["visibility"], "confirmed");
        assert_eq!(retry["identities"][0]["changed"], false);
        assert_eq!(retry["identities"][1]["changed"], false);
        assert_eq!(event_tags(&state, c, 8002).await.len(), 2);
        let snapshot = event_tags(&state, c, 13535).await;
        assert_eq!(snapshot.len(), 1);
        let snapshot = snapshot[0].as_array().unwrap();
        assert!(snapshot.contains(&serde_json::json!(["p", root])));
        assert!(snapshot.contains(&serde_json::json!(["p", agent])));

        let unarchive_body = serde_json::json!({"requestId": Uuid::new_v4()});
        let (status, restored) =
            call(&state, &operator, "POST", &unarchive, Some(&unarchive_body)).await;
        assert_eq!(status, StatusCode::OK, "{restored}");
        assert_eq!(restored["operation"], "unarchive");
        assert_eq!(
            restored["identities"],
            serde_json::json!([
                {"pubkey": root, "role": "root", "archived": false, "changed": true},
            ])
        );
        let (status, retry) =
            call(&state, &operator, "POST", &unarchive, Some(&unarchive_body)).await;
        assert_eq!(status, StatusCode::OK, "{retry}");
        assert_eq!(retry["identities"][0]["changed"], false);
        let deltas = event_tags(&state, c, 8003).await;
        assert_eq!(deltas.len(), 1);
        let delta = deltas[0].as_array().unwrap();
        assert!(delta.contains(&serde_json::json!(["p", root])));
        assert!(delta.contains(&serde_json::json!(["consent", "relay"])));
        assert!(!delta.iter().any(|tag| tag[0] == "e"));
        let snapshot = event_tags(&state, c, 13535).await;
        let snapshot = snapshot[0].as_array().unwrap();
        assert!(!snapshot.contains(&serde_json::json!(["p", root])));
        assert!(snapshot.contains(&serde_json::json!(["p", agent])));
        assert!(state.db.is_archived(c, &agent).await.unwrap());
        assert!(state.db.is_archived(other, &root).await.unwrap());
        assert!(state.db.is_archived(other, &agent).await.unwrap());

        let (_, after) = call(&state, &operator, "GET", &ownership, None).await;
        assert_eq!(after["workflows"], inventory["workflows"]);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_member_routes_refuse_before_applying_anything() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let mut state = identity_archive_state(&operator).await;
        Arc::get_mut(&mut state).expect("single owner").nip98_replay =
            Arc::new(SeenOnceReplayGuard(Mutex::new(HashSet::new())));
        let (community, host) = community(&state).await;
        let root = hex::encode(ROOT);
        let unarchive = format!("{BASE}/{root}/unarchive?communityHost={host}");
        let ownership = format!("{BASE}/{root}/ownership?communityHost={host}");
        state
            .db
            .archive(community, &root, "self", &root, None, None, &"a".repeat(64))
            .await
            .unwrap();
        let body =
            serde_json::to_vec(&serde_json::json!({"requestId": Uuid::new_v4()})).expect("body");
        let url = |target: &str| format!("http://{INGRESS_HOST}{target}");

        // Each refusal is the operator auth envelope with its code.
        let refusals = [
            (
                "not allowlisted",
                nip98_auth_header(&outsider, &url(&unarchive), "POST", Some(&body)),
                StatusCode::FORBIDDEN,
                "not_operator",
            ),
            (
                "admin-host origin",
                nip98_auth_header(
                    &operator,
                    &format!("https://{INGRESS_HOST}{unarchive}"),
                    "POST",
                    Some(&body),
                ),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "other community signed",
                nip98_auth_header(
                    &operator,
                    &url(&format!(
                        "{BASE}/{root}/unarchive?communityHost=other.example"
                    )),
                    "POST",
                    Some(&body),
                ),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "query not signed",
                nip98_auth_header(
                    &operator,
                    &url(&format!("{BASE}/{root}/unarchive")),
                    "POST",
                    Some(&body),
                ),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "different POST body signed",
                nip98_auth_header(&operator, &url(&unarchive), "POST", Some(b"{}")),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "POST body without payload tag",
                nip98_auth_header_without_payload(&operator, &url(&unarchive), "POST"),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "method not signed",
                nip98_auth_header(&operator, &url(&unarchive), "DELETE", Some(&body)),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
        ];
        for (case, auth, status, code) in refusals {
            let (actual, json) = send(&state, "POST", &unarchive, Some(body.clone()), auth).await;
            assert_eq!(actual, status, "{case}: {json}");
            assert_eq!(json["code"], code, "{case}: {json}");
            assert!(json["error"].is_string(), "{case}: {json}");
        }

        // A replayed signature is refused, on reads too.
        let auth = nip98_auth_header(&operator, &url(&ownership), "GET", None);
        assert_eq!(
            send(&state, "GET", &ownership, None, auth.clone()).await.0,
            StatusCode::OK
        );
        let (status, json) = send(&state, "GET", &ownership, None, auth).await;
        assert_eq!(
            (status, &json["code"]),
            (StatusCode::UNAUTHORIZED, &"unauthorized".into())
        );

        // Lifecycle refusals use the admin envelope.
        let unknown = format!("{BASE}/{root}/unarchive?communityHost=unmapped.example");
        let (status, json) = call(
            &state,
            &operator,
            "POST",
            &unknown,
            Some(&serde_json::json!({"requestId": Uuid::new_v4()})),
        )
        .await;
        assert_eq!(
            (status, lifecycle_code(&json)),
            (StatusCode::BAD_REQUEST, Some("unknown_community_host"))
        );

        // Over the 4096-byte limit.
        let large = serde_json::json!({"requestId": Uuid::new_v4(), "reason": "a".repeat(5000)});
        assert_eq!(
            call(&state, &operator, "POST", &unarchive, Some(&large))
                .await
                .0,
            StatusCode::PAYLOAD_TOO_LARGE
        );

        assert!(state.db.is_archived(community, &root).await.unwrap());
        assert!(event_tags(&state, community, 8003).await.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_archive_refuses_staff_in_the_confirmed_set() {
        let operator = Keys::generate();
        let state = identity_archive_state(&operator).await;
        let (community, host) = community(&state).await;
        let root = hex::encode(ROOT);
        let agent = hex::encode(AGENT);
        let ownership = format!("{BASE}/{root}/ownership?communityHost={host}");
        let (status, inventory) = call(&state, &operator, "GET", &ownership, None).await;
        assert_eq!(status, StatusCode::OK, "{inventory}");
        let version = inventory["inventoryVersion"].as_str().unwrap();
        let archive = format!("{BASE}/{root}/archive?communityHost={host}");

        for (body, status, code) in [
            (
                serde_json::json!({"requestId": Uuid::new_v4()}),
                StatusCode::BAD_REQUEST,
                "invalid_body",
            ),
            (
                serde_json::json!({
                    "requestId": Uuid::new_v4(), "inventoryVersion": version, "replacedBy": root,
                }),
                StatusCode::BAD_REQUEST,
                "invalid_replaced_by",
            ),
            (
                serde_json::json!({
                    "requestId": Uuid::new_v4(), "inventoryVersion": version, "reason": "Invalid reason",
                }),
                StatusCode::BAD_REQUEST,
                "invalid_reason",
            ),
            (
                serde_json::json!({
                    "requestId": Uuid::new_v4(), "inventoryVersion": "0".repeat(64),
                }),
                StatusCode::CONFLICT,
                "inventory_changed",
            ),
        ] {
            let (actual, json) = call(&state, &operator, "POST", &archive, Some(&body)).await;
            assert_eq!(
                (actual, lifecycle_code(&json)),
                (status, Some(code)),
                "{json}"
            );
        }

        // The grant lands after preview and before the archive request.
        state
            .db
            .upsert_relay_operator(&AGENT, "moderator", &operator.public_key().to_bytes(), true)
            .await
            .unwrap();
        let body = serde_json::json!({
            "requestId": Uuid::new_v4(), "inventoryVersion": version,
        });
        let (status, json) = call(&state, &operator, "POST", &archive, Some(&body)).await;
        assert_eq!(
            (status, lifecycle_code(&json)),
            (StatusCode::CONFLICT, Some("target_is_staff")),
            "{json}"
        );
        assert!(!state.db.is_archived(community, &root).await.unwrap());
        assert!(!state.db.is_archived(community, &agent).await.unwrap());
        assert!(event_tags(&state, community, 8002).await.is_empty());
        assert!(event_tags(&state, community, 13535).await.is_empty());

        let key = operator.public_key().to_hex();
        let ownership = format!("{BASE}/{key}/ownership?communityHost={host}");
        let (_, inventory) = call(&state, &operator, "GET", &ownership, None).await;
        let archive = format!("{BASE}/{key}/archive?communityHost={host}");
        let body = serde_json::json!({
            "requestId": Uuid::new_v4(), "inventoryVersion": inventory["inventoryVersion"],
        });
        let (status, json) = call(&state, &operator, "POST", &archive, Some(&body)).await;
        assert_eq!(
            (status, lifecycle_code(&json)),
            (StatusCode::CONFLICT, Some("target_is_staff")),
            "{json}"
        );
        assert!(!state.db.is_archived(community, &key).await.unwrap());
    }
}
