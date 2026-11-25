mod postgres_tests {
    use super::super::*;

    #[derive(Clone)]
    struct DrainStorageState {
        put_started: Arc<tokio::sync::Notify>,
        allow_put: Arc<tokio::sync::Semaphore>,
    }

    struct DrainStorageServer {
        state: DrainStorageState,
        endpoint: String,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for DrainStorageServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl DrainStorageServer {
        async fn start() -> Self {
            let state = DrainStorageState {
                put_started: Arc::new(tokio::sync::Notify::new()),
                allow_put: Arc::new(tokio::sync::Semaphore::new(0)),
            };
            let app = axum::Router::new()
                .fallback(
                    |State(state): State<DrainStorageState>, request: Request<Body>| async move {
                        if request.method() == axum::http::Method::PUT {
                            to_bytes(request.into_body(), 1_000_000).await.unwrap();
                            state.put_started.notify_one();
                            state.allow_put.acquire().await.unwrap().forget();
                            (StatusCode::OK, [("etag", "\"test\"")], "").into_response()
                        } else {
                            StatusCode::NOT_FOUND.into_response()
                        }
                    },
                )
                .with_state(state.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                state,
                endpoint,
                task,
            }
        }
    }

    async fn drain_upload_fixture() -> (
        Arc<AppState>,
        String,
        buzz_core::CommunityId,
        DrainStorageServer,
    ) {
        let storage = DrainStorageServer::start().await;
        let mut state = media_enforce_test_state()
            .await
            .expect("Postgres and Redis fixture");
        let mutable = Arc::get_mut(&mut state).unwrap();
        let config = Arc::make_mut(&mut mutable.config);
        config.media.s3_endpoint = storage.endpoint.clone();
        config.media.s3_bucket = "test".into();
        config.media.s3_region = "us-east-1".into();
        config.media.s3_access_key = "test".into();
        config.media.s3_secret_key = "test".into();
        config.media.s3_addressing_style = buzz_media::S3AddressingStyle::Path;
        config.media.upload_records_enabled = false;
        config.media_read_mode = crate::api::media_access::MediaReadMode::Off;
        mutable.media_storage = Arc::new(buzz_media::MediaStorage::new(&config.media).unwrap());
        mutable.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(
            100,
            vec![buzz_auth::IssuerCapacity {
                issuer: TEST_ISSUER.into(),
                capacity: 100,
            }],
        )));
        let host = format!("drain-{}.example.org", uuid::Uuid::new_v4());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        (state, host, community, storage)
    }

    const DRAIN_PDF: &[u8] = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";

    fn drain_upload_request(keys: &Keys, host: &str, body: Body) -> Request<Body> {
        let hash = sha256_hex(DRAIN_PDF);
        Request::builder()
            .method("PUT")
            .uri("/upload")
            .header("host", host)
            .header(
                "authorization",
                blossom_upload_auth_value(keys, host, &hash),
            )
            .header("x-sha-256", hash)
            .header(
                "nostr-federated-identity",
                format!("Bearer {}", signed_assertion(&keys.public_key().to_hex())),
            )
            .body(body)
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn media_drain_tracks_real_upload_through_storage_and_durable_ownership() {
        let (state, host, community, storage) = drain_upload_fixture().await;
        let alice = Keys::generate();
        let bob = Keys::generate();
        let router = crate::router::build_router(state.clone());
        let mut upload = tokio::spawn(router.oneshot(drain_upload_request(
            &alice,
            &host,
            Body::from(DRAIN_PDF),
        )));
        tokio::time::timeout(Duration::from_secs(5), storage.state.put_started.notified())
            .await
            .unwrap();
        let err = buzz_deletion::user::wait_for_draining::handle(
            &state.db,
            &alice.public_key().to_hex(),
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("active=1"), "{err}");
        assert!(
            buzz_deletion::user::wait_for_draining::handle(
                &state.db,
                &bob.public_key().to_hex(),
                true
            )
            .await
            .unwrap()
            .drained
        );
        let mut audit_barrier = state.db.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("buzz_audit:{community}"))
            .execute(&mut *audit_barrier)
            .await
            .unwrap();
        storage.state.allow_put.add_permits(20);
        tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT pg_advisory_xact_lock(hashtextextended%')")
                .fetch_one(state.db.pool()).await.unwrap();
            if waiting { break; }
            assert!(!upload.is_finished(), "upload completed before durable audit ownership");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut upload)
                .await
                .is_err(),
            "upload must remain pending while its audit append is blocked"
        );
        let error = buzz_deletion::user::wait_for_draining::handle(
            &state.db,
            &alice.public_key().to_hex(),
            true,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("active=1"), "{error}");
        audit_barrier.commit().await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), upload)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert!(
            buzz_deletion::user::wait_for_draining::handle(
                &state.db,
                &alice.public_key().to_hex(),
                true
            )
            .await
            .unwrap()
            .drained
        );
        let recorded: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE community_id=$1 AND actor_pubkey=$2 AND object_id=$3 AND action='media_uploaded')")
        .bind(community.as_uuid()).bind(alice.public_key().as_bytes().as_slice()).bind(sha256_hex(DRAIN_PDF))
        .fetch_one(state.db.pool()).await.unwrap();
        assert!(
            recorded,
            "audit ownership must be durable even with media-read and upload records off"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn media_drain_cancellation_distinguishes_body_receive_from_remote_write() {
        let (state, host, _, storage) = drain_upload_fixture().await;
        let alice = Keys::generate();
        let polled = Arc::new(tokio::sync::Notify::new());
        let signal = polled.clone();
        let stream = futures_util::stream::once(async move {
            signal.notify_one();
            std::future::pending::<Result<bytes::Bytes, std::io::Error>>().await
        });
        let task = tokio::spawn(crate::router::build_router(state.clone()).oneshot(
            drain_upload_request(&alice, &host, Body::from_stream(stream)),
        ));
        tokio::time::timeout(Duration::from_secs(5), polled.notified())
            .await
            .unwrap();
        assert!(
            !buzz_deletion::user::wait_for_draining::inspect(
                &state.db,
                &alice.public_key().to_hex(),
                true
            )
            .await
            .unwrap()
            .drained
        );
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state
                    .db
                    .deletion_store()
                    .user_media_upload_activity(&alice.public_key().to_hex())
                    .await
                    .unwrap()
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let task = tokio::spawn(
            crate::router::build_router(state.clone()).oneshot(drain_upload_request(
                &alice,
                &host,
                Body::from(DRAIN_PDF),
            )),
        );
        tokio::time::timeout(Duration::from_secs(5), storage.state.put_started.notified())
            .await
            .unwrap();
        task.abort();
        let _ = task.await;
        sqlx::query("UPDATE community_serving_write_leases SET media_lease_until=now()-interval '1 second' WHERE actor_pubkey=$1")
        .bind(alice.public_key().as_bytes().as_slice()).execute(state.db.pool()).await.unwrap();
        // The old relay's reaper must not erase a new relay's uncertain remote write.
        sqlx::query("DELETE FROM community_serving_write_leases WHERE lease_until<now()")
            .execute(state.db.pool())
            .await
            .unwrap();
        let report = buzz_deletion::user::wait_for_draining::inspect(
            &state.db,
            &alice.public_key().to_hex(),
            true,
        )
        .await
        .unwrap();
        assert!(!report.drained);
        assert_eq!(report.activity[0].unresolved, 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn media_drain_late_revocation_rejects_request_stalled_before_registration() {
        let (state, host, _, _storage) = drain_upload_fixture().await;
        let alice = Keys::generate();
        let mut barrier = state.db.pool().begin().await.unwrap();
        sqlx::query("LOCK TABLE community_serving_write_leases IN SHARE MODE")
            .execute(&mut *barrier)
            .await
            .unwrap();
        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = polls.clone();
        let stream = futures_util::stream::once(async move {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, std::io::Error>(bytes::Bytes::from_static(DRAIN_PDF))
        });
        let task = tokio::spawn(crate::router::build_router(state.clone()).oneshot(
            drain_upload_request(&alice, &host, Body::from_stream(stream)),
        ));
        tokio::time::timeout(Duration::from_secs(2),async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'INSERT INTO community_serving_write_leases%')")
                .fetch_one(state.db.pool()).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
        let now = chrono::Utc::now();
        state
            .nip_fi_deny_map
            .as_ref()
            .unwrap()
            .merge_cross_pod_deny(
                TEST_ISSUER,
                &alice.public_key(),
                now + chrono::Duration::minutes(5),
                now,
            );
        assert!(
            buzz_deletion::user::wait_for_draining::handle(
                &state.db,
                &alice.public_key().to_hex(),
                true
            )
            .await
            .unwrap()
            .drained
        );
        barrier.commit().await.unwrap();
        let late = task.await.unwrap().unwrap();
        let late_status = late.status();
        let late_headers = late.headers().clone();
        let late_body = to_bytes(late.into_body(), 8192).await.unwrap();
        let early = crate::router::build_router(state.clone())
            .oneshot(drain_upload_request(&alice, &host, Body::from(DRAIN_PDF)))
            .await
            .unwrap();
        assert_eq!(late_status, StatusCode::FORBIDDEN);
        assert_eq!(late_status, early.status());
        for header in ["content-type", "cache-control", "www-authenticate"] {
            assert_eq!(
                late_headers.get(header),
                early.headers().get(header),
                "{header}"
            );
        }
        assert_eq!(late_body, to_bytes(early.into_body(), 8192).await.unwrap());
        assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(
            buzz_deletion::user::wait_for_draining::handle(
                &state.db,
                &alice.public_key().to_hex(),
                true
            )
            .await
            .unwrap()
            .drained
        );
    }
}
