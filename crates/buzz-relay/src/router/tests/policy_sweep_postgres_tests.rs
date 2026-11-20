mod postgres_tests {
    use super::super::postgres_tests::real_db_state;
    use super::super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn frame(client: &mut Client, stage: &str) -> Message {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let message = client.next().await.unwrap().unwrap();
                match message {
                    Message::Ping(_) => {
                        client.flush().await.unwrap();
                    }
                    Message::Pong(_) => {}
                    message => return message,
                }
            }
        })
        .await
        .unwrap_or_else(|error| panic!("{stage}: {error}"))
    }

    async fn authenticated_socket(
        addr: std::net::SocketAddr,
        host: &str,
        path: &str,
        key: &nostr::Keys,
        assertion: Option<&str>,
        audio: bool,
    ) -> Client {
        let mut request = format!("ws://{addr}{path}").into_client_request().unwrap();
        request.headers_mut().insert("host", host.parse().unwrap());
        if let Some(token) = assertion {
            request.headers_mut().insert(
                "nostr-federated-identity",
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        let (mut client, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let Message::Text(challenge) = frame(&mut client, "challenge").await else {
            panic!("challenge")
        };
        let challenge: serde_json::Value = serde_json::from_str(&challenge).unwrap();
        let challenge = if audio {
            challenge["challenge"].as_str().unwrap()
        } else {
            challenge[1].as_str().unwrap()
        };
        let event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &format!("ws://{host}")]).unwrap())
            .tag(nostr::Tag::parse(["challenge", challenge]).unwrap())
            .sign_with_keys(key)
            .unwrap();
        let auth = if audio {
            serde_json::json!({"type":"auth","event":event,"parent_channel_id":null,"protocol_version":1})
        } else {
            serde_json::json!(["AUTH", event])
        };
        client
            .send(Message::Text(auth.to_string().into()))
            .await
            .unwrap();
        loop {
            let Message::Text(text) = frame(&mut client, "authenticated socket").await else {
                panic!("admission closed")
            };
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if audio && value["type"] == "joined" {
                break;
            }
            if !audio && value[0] == "OK" {
                assert_eq!(value[2], true, "{value}");
                break;
            }
            assert!(value["type"] != "error", "{value}");
        }
        client
    }

    async fn ping_round_trip(client: &mut Client) {
        client
            .send(Message::Ping(bytes::Bytes::from_static(b"live")))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match client.next().await.unwrap().unwrap() {
                    Message::Pong(bytes) => {
                        assert_eq!(bytes.as_ref(), b"live");
                        break;
                    }
                    Message::Ping(_) => {
                        client.flush().await.unwrap();
                    }
                    Message::Close(close) => panic!("unexpected close {close:?}"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn sweep_closes_idle_root_audio_and_late_registrations_after_read_recovery() {
        for late_audio in [false, true] {
            sweep_journey(late_audio).await;
        }
    }

    async fn sweep_journey(late_audio: bool) {
        use crate::nip_fi_core::test_support::{
            assertion_verifier, sign_assertion, TEST_COMMUNITY_URI, TEST_ISSUER,
        };
        use buzz_db::community_auth::{PolicyOutcome, PolicyRequest, UpgradeRequest};
        let base = real_db_state().await.unwrap();
        let mut configured = (*base).clone();
        let config = Arc::make_mut(&mut configured.config);
        config.nip_fi.mode = buzz_auth::NipFiMode::Shadow;
        config.nip_fi_changes = false;
        config.nip_fi.max_connection_lifetime_secs = 3600;
        config.nip_fi.communities = crate::nip_fi_config::NipFiCommunities::for_test_any_host(
            TEST_COMMUNITY_URI,
            &[TEST_ISSUER],
        );
        configured.nip_fi_verifier = Some(Arc::new(assertion_verifier(
            buzz_auth::TokenClass::DedicatedNipFi,
        )));
        configured.nip_fi_deny_map = Some(Arc::new(buzz_auth::NipFiDenyMap::new(
            16,
            vec![buzz_auth::IssuerCapacity {
                issuer: TEST_ISSUER.into(),
                capacity: 16,
            }],
        )));
        let redis = buzz_pubsub::redis_connection(&std::env::var("REDIS_URL").unwrap()).unwrap();
        configured.admission_rate_limiter =
            Arc::new(buzz_pubsub::rate_limiter::RedisRateLimiter::new(redis));
        let state = Arc::new(configured);
        let key = nostr::Keys::generate();
        let owner = key.public_key().to_hex();
        let host = format!("dark-target-{}.example", uuid::Uuid::new_v4());
        let created = state
            .db
            .create_community_with_owner_and_nip_fi_issuers(
                &host,
                &owner,
                &[TEST_ISSUER.into()],
                &key,
            )
            .await
            .unwrap();
        let buzz_db::CreateCommunityWithOwnerResult::Created(row) = created.outcome else {
            panic!("target")
        };
        let free_host = format!("dark-free-{}.example", uuid::Uuid::new_v4());
        let _ = state
            .db
            .create_community_with_owner(&free_host, &owner, &key)
            .await
            .unwrap();
        state
            .db
            .ensure_user(row.id, &key.public_key().to_bytes())
            .await
            .unwrap();
        let channel = uuid::Uuid::new_v4();
        state
            .db
            .create_channel_with_id(
                row.id,
                channel,
                "dark-audio",
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                &key.public_key().to_bytes(),
                None,
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'member') ON CONFLICT DO NOTHING")
            .bind(row.id.as_uuid()).bind(channel).bind(key.public_key().to_bytes().to_vec()).execute(state.db.pool()).await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_router(state.clone())
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path = format!("/huddle/{channel}/audio");
        let mut root = authenticated_socket(addr, &host, "/", &key, None, false).await;
        let mut audio = authenticated_socket(addr, &host, &path, &key, None, true).await;
        let mut free = authenticated_socket(addr, &free_host, "/", &key, None, false).await;

        // A callback already admitted under optional policy registers only after
        // the sweep. Exercise both transport types in the same journey.
        let (arrived, release) = crate::nip_fi_test_hooks::community_registration_hook::arm(row.id);
        let mut late_request = format!(
            "ws://{addr}{}",
            if late_audio { path.as_str() } else { "/" }
        )
        .into_client_request()
        .unwrap();
        late_request
            .headers_mut()
            .insert("host", host.parse().unwrap());
        let (mut late, _) = tokio_tungstenite::connect_async(late_request)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), arrived)
            .await
            .unwrap()
            .unwrap();
        let request = UpgradeRequest {
            operation_id: uuid::Uuid::new_v4(),
            community_id: *row.id.as_uuid(),
            expected_host: host.clone(),
            actor_pubkey: owner.clone(),
            expected_owner_pubkey: owner.clone(),
            expected_issuer: TEST_ISSUER.into(),
            operator_pubkey: owner,
        };
        let PolicyOutcome::Accepted(record) = state
            .db
            .set_community_nip_fi_policy(
                &PolicyRequest {
                    tuple: request.clone(),
                    requires_nip_fi: true,
                    expected_policy_generation: 0,
                    expected_requires_nip_fi: false,
                },
                Ok(()),
            )
            .await
            .unwrap()
        else {
            panic!("enable")
        };

        // No invalidation or request on an existing client. A writer table lock
        // blocks real policy reads until their 500ms budget expires.
        let mut lock = state.db.pool().begin().await.unwrap();
        sqlx::query("LOCK TABLE communities IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *lock)
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), state.revalidate_live_communities())
                .await
                .unwrap(),
            0
        );
        lock.rollback().await.unwrap();

        let token = sign_assertion("nip-fi+jwt", &key.public_key().to_hex(), None);
        let mut compliant = authenticated_socket(addr, &host, "/", &key, Some(&token), false).await;
        // Keep compliant audio on a different channel so room cleanup is independent.
        let next_channel = uuid::Uuid::new_v4();
        state
            .db
            .create_channel_with_id(
                row.id,
                next_channel,
                "fi-audio",
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                &key.public_key().to_bytes(),
                None,
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'member') ON CONFLICT DO NOTHING")
            .bind(row.id.as_uuid()).bind(next_channel).bind(key.public_key().to_bytes().to_vec()).execute(state.db.pool()).await.unwrap();
        let mut compliant_audio = authenticated_socket(
            addr,
            &host,
            &format!("/huddle/{next_channel}/audio"),
            &key,
            Some(&token),
            true,
        )
        .await;
        // Explicit Off and repair mode retain lifecycle-only sweep behavior.
        for mode in [
            buzz_auth::NipFiMode::Off,
            buzz_auth::NipFiMode::DenyProtected,
        ] {
            let mut bypass = (*state).clone();
            Arc::make_mut(&mut bypass.config).nip_fi.mode = mode;
            assert_eq!(bypass.revalidate_live_communities().await, 0);
        }
        assert_eq!(state.revalidate_live_communities().await, 2);
        for client in [&mut root, &mut audio] {
            policy_close(client).await;
        }
        release.notify_one();
        assert!(matches!(
            frame(&mut late, "late root registers").await,
            Message::Text(_)
        ));
        assert_eq!(state.revalidate_live_communities().await, 1);
        policy_close(&mut late).await;

        assert_eq!(state.revalidate_live_communities().await, 0);
        for client in [&mut compliant, &mut compliant_audio, &mut free] {
            ping_round_trip(client).await;
        }
        // Global Enforce also closes an optional community's legacy socket.
        let mut enforce = (*state).clone();
        Arc::make_mut(&mut enforce.config).nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        assert_eq!(enforce.revalidate_live_communities().await, 1);
        policy_close(&mut free).await;
        for client in [&mut compliant, &mut compliant_audio] {
            ping_round_trip(client).await;
        }
        let status = state
            .db
            .community_auth_policy_status(record.acceptance.operation_id, &request.operator_pubkey)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.acceptance.state, "pending");
        assert_eq!(
            status.acceptance.pending_reasons,
            vec!["connection_closure_unverified"]
        );
        drop((compliant, compliant_audio, free));
        server.abort();
        let _ = server.await;
    }

    async fn policy_close(client: &mut Client) {
        loop {
            if let Message::Close(Some(close)) = frame(client, "policy close").await {
                assert_eq!(u16::from(close.code), 1008);
                assert_eq!(close.reason, "community authentication upgraded");
                let _ = client.flush().await;
                break;
            }
        }
    }
}
