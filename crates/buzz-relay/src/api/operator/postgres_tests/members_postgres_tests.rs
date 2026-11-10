mod postgres_tests {
    //! Operator-API member routes: allowlisted NIP-98 over the operator origin,
    //! with the signed query and body binding the request, then the same
    //! lifecycle as the admin-host routes. The test state configures no admin
    //! host, and every request arrives on the operator ingress `Host`.

    use super::super::*;

    const ROOT: [u8; 32] = [0xb1; 32];
    const AGENT: [u8; 32] = [0xb2; 32];
    const BASE: &str = "/operator/communities/members";

    async fn retirement_state(operator: &Keys, enabled: bool) -> Option<Arc<AppState>> {
        let mut state = operator_test_state(std::slice::from_ref(operator)).await?;
        let config = &mut Arc::get_mut(&mut state).expect("single owner").config;
        Arc::make_mut(config).admin_identity_retirement = enabled;
        Some(state)
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

    async fn actions(state: &AppState, community: CommunityId) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM identity_retirement_actions WHERE community_id = $1",
        )
        .bind(community.as_uuid())
        .fetch_one(state.db.pool())
        .await
        .expect("count actions")
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
    async fn operator_member_routes_share_the_retirement_lifecycle() {
        let operator = Keys::generate();
        let Some(state) = retirement_state(&operator, true).await else {
            return;
        };
        let (community, host) = community(&state).await;
        let root = hex::encode(ROOT);
        let agent = hex::encode(AGENT);

        // Search binds the percent-encoded query exactly as sent.
        let search = format!("{BASE}/search?communityHost={host}&q=alex%20root&limit=5");
        let (status, page) = call(&state, &operator, "GET", &search, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["items"][0]["pubkey"], root, "{page}");
        assert_eq!(page["items"][0]["displayName"], "Alex Root");

        let ownership = format!("{BASE}/{root}/ownership?communityHost={host}");
        let (status, inventory) = call(&state, &operator, "GET", &ownership, None).await;
        assert_eq!(status, StatusCode::OK, "{inventory}");
        let version = inventory["inventoryVersion"]
            .as_str()
            .expect("version")
            .to_owned();

        let archive = format!("{BASE}/{root}/archive?communityHost={host}");
        let request_id = Uuid::new_v4();
        let body = serde_json::json!({
            "requestId": request_id, "reason": "left-company", "inventoryVersion": version,
        });
        let (status, first) = call(&state, &operator, "POST", &archive, Some(&body)).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["replayed"], false);
        assert_eq!(first["communityHost"], host);
        assert_eq!(first["identities"].as_array().expect("identities").len(), 2);
        assert_eq!(first["workflows"][0]["status"], "archived");
        assert_eq!(first["stages"]["newWorkAdmission"], "blocked");
        let actor: Vec<u8> = sqlx::query_scalar(
            "SELECT actor_pubkey FROM identity_retirement_actions WHERE community_id = $1",
        )
        .bind(community.as_uuid())
        .fetch_one(state.db.pool())
        .await
        .expect("actor");
        assert_eq!(actor, operator.public_key().to_bytes().to_vec());

        // A fresh signature over the same request replays the stored result;
        // any change to it conflicts.
        let (status, again) = call(&state, &operator, "POST", &archive, Some(&body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["replayed"], true);
        assert_eq!(again["actionId"], first["actionId"]);
        let changed = serde_json::json!({
            "requestId": request_id, "reason": "other", "inventoryVersion": version,
        });
        let (status, err) = call(&state, &operator, "POST", &archive, Some(&changed)).await;
        assert_eq!(
            (status, lifecycle_code(&err)),
            (StatusCode::CONFLICT, Some("request_id_conflict"))
        );
        let staff = format!(
            "{BASE}/{}/archive?communityHost={host}",
            operator.public_key().to_hex()
        );
        let (_, err) = call(&state, &operator, "POST", &staff, Some(&body)).await;
        assert_eq!(lifecycle_code(&err), Some("target_is_staff"));

        // Revive keeps the hold; release is per key.
        let revive = serde_json::json!({"requestId": Uuid::new_v4()});
        let (status, revived) = call(&state, &operator, "DELETE", &archive, Some(&revive)).await;
        assert_eq!(status, StatusCode::OK, "{revived}");
        assert_eq!(revived["identities"][0]["archived"], false);
        assert_eq!(revived["identities"][0]["executionHeld"], true);
        let hold = |key: &str| format!("{BASE}/{key}/execution-hold?communityHost={host}");
        let release = serde_json::json!({"requestId": Uuid::new_v4()});
        let (status, released) =
            call(&state, &operator, "DELETE", &hold(&agent), Some(&release)).await;
        assert_eq!(status, StatusCode::OK, "{released}");
        assert_eq!(released["stages"]["newWorkAdmission"], "blocked");
        assert_eq!(released["workflows"][0]["status"], "active");
        let release = serde_json::json!({"requestId": Uuid::new_v4()});
        let (_, released) = call(&state, &operator, "DELETE", &hold(&root), Some(&release)).await;
        assert_eq!(released["stages"]["newWorkAdmission"], "released");
        assert_eq!(actions(&state, community).await, 4);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_member_routes_refuse_before_applying_anything() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let Some(mut state) = retirement_state(&operator, true).await else {
            return;
        };
        Arc::get_mut(&mut state).expect("single owner").nip98_replay =
            Arc::new(SeenOnceReplayGuard(Mutex::new(HashSet::new())));
        let (community, host) = community(&state).await;
        let root = hex::encode(ROOT);
        let archive = format!("{BASE}/{root}/archive?communityHost={host}");
        let ownership = format!("{BASE}/{root}/ownership?communityHost={host}");
        let body =
            serde_json::to_vec(&serde_json::json!({"requestId": Uuid::new_v4()})).expect("body");
        let url = |target: &str| format!("http://{INGRESS_HOST}{target}");

        // Each refusal is the operator auth envelope with its code.
        let refusals = [
            (
                "not allowlisted",
                nip98_auth_header(&outsider, &url(&archive), "DELETE", Some(&body)),
                StatusCode::FORBIDDEN,
                "not_operator",
            ),
            (
                "admin-host origin",
                nip98_auth_header(
                    &operator,
                    &format!("https://{INGRESS_HOST}{archive}"),
                    "DELETE",
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
                        "{BASE}/{root}/archive?communityHost=other.example"
                    )),
                    "DELETE",
                    Some(&body),
                ),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "query not signed",
                nip98_auth_header(
                    &operator,
                    &url(&format!("{BASE}/{root}/archive")),
                    "DELETE",
                    Some(&body),
                ),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "different DELETE body signed",
                nip98_auth_header(&operator, &url(&archive), "DELETE", Some(b"{}")),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "DELETE body without payload tag",
                nip98_auth_header_without_payload(&operator, &url(&archive), "DELETE"),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (
                "method not signed",
                nip98_auth_header(&operator, &url(&archive), "POST", Some(&body)),
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
        ];
        for (case, auth, status, code) in refusals {
            let (actual, json) = send(&state, "DELETE", &archive, Some(body.clone()), auth).await;
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
        let unknown = format!("{BASE}/{root}/archive?communityHost=unmapped.example");
        let (status, json) = call(
            &state,
            &operator,
            "DELETE",
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
            call(&state, &operator, "DELETE", &archive, Some(&large))
                .await
                .0,
            StatusCode::PAYLOAD_TOO_LARGE
        );

        // A held retirement lock: definite retry_later, nothing applied.
        let mut blocker = state.db.pool().begin().await.expect("blocker");
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('identity-retirement:' || $1::text, 0))",
        )
        .bind(community.as_uuid())
        .execute(&mut *blocker)
        .await
        .expect("hold retirement lock");
        let (status, json) = call(
            &state,
            &operator,
            "DELETE",
            &archive,
            Some(&serde_json::json!({"requestId": Uuid::new_v4()})),
        )
        .await;
        blocker.rollback().await.expect("release lock");
        assert_eq!(
            (status, lifecycle_code(&json)),
            (StatusCode::SERVICE_UNAVAILABLE, Some("retry_later")),
            "{json}"
        );
        assert_eq!(actions(&state, community).await, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn operator_retirement_is_refused_while_the_flag_is_off() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let Some(state) = retirement_state(&operator, false).await else {
            return;
        };
        let (community, host) = community(&state).await;
        let archive = format!("{BASE}/{}/archive?communityHost={host}", hex::encode(ROOT));
        let body =
            serde_json::json!({"requestId": Uuid::new_v4(), "inventoryVersion": "0".repeat(64)});

        // Authorization still comes first.
        let (status, json) = call(&state, &outsider, "POST", &archive, Some(&body)).await;
        assert_eq!(
            (status, &json["code"]),
            (StatusCode::FORBIDDEN, &"not_operator".into())
        );
        let (status, json) = call(&state, &operator, "POST", &archive, Some(&body)).await;
        assert_eq!(
            (status, lifecycle_code(&json)),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Some("identity_retirement_disabled")
            )
        );
        assert_eq!(actions(&state, community).await, 0);
    }
}
