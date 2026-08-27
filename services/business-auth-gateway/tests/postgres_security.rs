use business_auth_gateway::{
    auth::{Audience, Claims},
    model::{ChallengeRequest, EmbedTarget, IssueEmbedRequest, Principal, RequestFacts},
    security,
    store::Store,
    Config,
};
use chrono::Utc;
use nostr::{EventBuilder, Keys, Kind};
use sqlx::{postgres::PgPoolOptions, Row};
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

fn config(database_url: String) -> Config {
    Config {
        database_url,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        authentik_issuer: "https://auth.test/application/o/workbench".into(),
        workbench_client_id: "workbench".into(),
        business_client_id: "business".into(),
        allowed_workbench_origins: HashSet::from(["tauri://localhost".into()]),
        business_origin: "https://business.test".into(),
        challenge_ttl: Duration::from_secs(90),
        embed_ttl: Duration::from_secs(30),
        business_ttl: Duration::from_secs(3600),
        rate_limit: 10,
        cleanup_interval: Duration::from_secs(60),
        cookie_name: "__Host-test".into(),
        cookie_secure: true,
        deployment_id: "test".into(),
        global_logout_redirect_uri: "https://workbench.test/".into(),
        business_agent_read_enabled: true,
        business_agent_draft_write_enabled: false,
        business_read_mcp_audience: "business-read-mcp".into(),
        agent_delegation_ttl: Duration::from_secs(300),
        agent_delegation_max_calls: 20,
        business_agent_rate_limit_per_minute: 10,
        business_read_service_credential: Some("test-service-credential-at-least-32-bytes".into()),
    }
}

fn facts() -> RequestFacts {
    RequestFacts {
        ip: Some("127.0.0.1".into()),
        user_agent_hash: None,
        trace_id: Uuid::new_v4(),
    }
}

async fn issue(store: &Store, principal: &Principal, keys: &Keys, device_id: &str) -> String {
    store
        .issue_embed(
            principal,
            IssueEmbedRequest {
                target: EmbedTarget {
                    r#type: "sales_order".into(),
                    id: "SO-001".into(),
                    path: "/embed/sales/orders/SO-001".into(),
                },
                source: None,
                pubkey: keys.public_key().to_hex(),
                device_id: device_id.into(),
            },
            facts(),
        )
        .await
        .unwrap()
        .embed_url
        .split("code=")
        .nth(1)
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn postgres_binding_ticket_replay_revocation_cleanup_and_audit() {
    let Ok(database_url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL absent; PostgreSQL integration test skipped");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect(&database_url)
        .await
        .unwrap();
    Store::migrate(&pool).await.unwrap();
    sqlx::raw_sql("TRUNCATE security_audit_events,business_sessions,embed_sessions,identity_binding_challenges,buzz_identity_bindings,workbench_sessions,enterprise_users CASCADE").execute(&pool).await.unwrap();
    let store = Store::new(pool.clone(), config(database_url));
    let claims = Claims {
        iss: "https://auth.test/application/o/workbench".into(),
        sub: "user-1".into(),
        exp: Utc::now().timestamp() + 3600,
        aud: Some(Audience::One("workbench".into())),
        azp: Some("workbench".into()),
        client_id: None,
        email: Some("person@test.invalid".into()),
        name: Some("Test Person".into()),
        preferred_username: None,
        sid: Some("sid-1".into()),
        events: None,
    };
    let principal = store.principal(&claims, &facts()).await.unwrap();
    let keys = Keys::generate();
    let mut device_id = "test-device-0001";
    let rejected_challenge = store
        .challenge(
            &principal,
            ChallengeRequest {
                pubkey: keys.public_key().to_hex(),
                device_id: "rejected-device-0001".into(),
                device_name: "Rejected Mac".into(),
                device_platform: "macos".into(),
            },
            facts(),
        )
        .await
        .unwrap();
    let wrong_keys = Keys::generate();
    let wrong_signature = EventBuilder::new(Kind::Custom(24243), &rejected_challenge.payload)
        .sign_with_keys(&wrong_keys)
        .unwrap();
    assert!(store
        .verify_binding(&principal, rejected_challenge.id, wrong_signature, facts(),)
        .await
        .is_err());

    let expired_challenge = store
        .challenge(
            &principal,
            ChallengeRequest {
                pubkey: keys.public_key().to_hex(),
                device_id: "expired-device-0001".into(),
                device_name: "Expired Mac".into(),
                device_platform: "macos".into(),
            },
            facts(),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE identity_binding_challenges SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(expired_challenge.id)
    .execute(&pool)
    .await
    .unwrap();
    let expired_signature = EventBuilder::new(Kind::Custom(24243), &expired_challenge.payload)
        .sign_with_keys(&keys)
        .unwrap();
    assert!(store
        .verify_binding(&principal, expired_challenge.id, expired_signature, facts(),)
        .await
        .is_err());

    let challenge = store
        .challenge(
            &principal,
            ChallengeRequest {
                pubkey: keys.public_key().to_hex(),
                device_id: device_id.into(),
                device_name: "Test Mac".into(),
                device_platform: "macos".into(),
            },
            facts(),
        )
        .await
        .unwrap();
    let event = EventBuilder::new(Kind::Custom(24243), &challenge.payload)
        .sign_with_keys(&keys)
        .unwrap();
    let initial_binding = store
        .verify_binding(&principal, challenge.id, event.clone(), facts())
        .await
        .unwrap();
    assert!(
        store
            .verify_binding(&principal, challenge.id, event, facts())
            .await
            .is_err(),
        "binding challenge replay must fail"
    );

    let pre_rebind_code = issue(&store, &principal, &keys, device_id).await;
    let pre_rebind_session = store.bootstrap(&pre_rebind_code, facts()).await.unwrap();
    let replacement_device_id = "test-device-0002";
    let replacement_challenge = store
        .challenge(
            &principal,
            ChallengeRequest {
                pubkey: keys.public_key().to_hex(),
                device_id: replacement_device_id.into(),
                device_name: "Replacement Mac".into(),
                device_platform: "macos".into(),
            },
            facts(),
        )
        .await
        .unwrap();
    let replacement_event = EventBuilder::new(Kind::Custom(24243), &replacement_challenge.payload)
        .sign_with_keys(&keys)
        .unwrap();
    let binding = store
        .verify_binding(
            &principal,
            replacement_challenge.id,
            replacement_event,
            facts(),
        )
        .await
        .unwrap();
    device_id = replacement_device_id;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM buzz_identity_bindings WHERE id=$1")
            .bind(initial_binding.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "revoked"
    );
    assert!(
        store
            .business_state(&pre_rebind_session.session_token)
            .await
            .is_err(),
        "moving a pubkey to another device must revoke the old Business session"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM buzz_identity_bindings WHERE enterprise_user_id=$1 AND buzz_pubkey=$2 AND status='active'"
        )
        .bind(principal.user_id)
        .bind(keys.public_key().to_hex())
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );

    assert!(store
        .issue_embed(
            &principal,
            IssueEmbedRequest {
                target: EmbedTarget {
                    r#type: "sales_order".into(),
                    id: "SO-001".into(),
                    path: "https://evil.test/embed/SO-001".into(),
                },
                source: None,
                pubkey: keys.public_key().to_hex(),
                device_id: device_id.into(),
            },
            facts(),
        )
        .await
        .is_err());

    let wrong_audience_code = issue(&store, &principal, &keys, device_id).await;
    let wrong_audience_id: Uuid =
        sqlx::query_scalar("SELECT id FROM embed_sessions WHERE code_hash=$1")
            .bind(security::hash(&wrong_audience_code))
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE embed_sessions SET deployment_id='wrong-deployment' WHERE id=$1")
        .bind(wrong_audience_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store
        .bootstrap(&wrong_audience_code, facts())
        .await
        .is_err());

    let revoked_code = issue(&store, &principal, &keys, device_id).await;
    let revoked_id: Uuid = sqlx::query_scalar("SELECT id FROM embed_sessions WHERE code_hash=$1")
        .bind(security::hash(&revoked_code))
        .fetch_one(&pool)
        .await
        .unwrap();
    store
        .revoke_embed(&principal, revoked_id, facts())
        .await
        .unwrap();
    assert!(store.bootstrap(&revoked_code, facts()).await.is_err());

    let expired_code = issue(&store, &principal, &keys, device_id).await;
    let expired_id: Uuid = sqlx::query_scalar("SELECT id FROM embed_sessions WHERE code_hash=$1")
        .bind(security::hash(&expired_code))
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE embed_sessions SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(expired_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.bootstrap(&expired_code, facts()).await.is_err());
    assert!(store
        .issue_embed(
            &principal,
            IssueEmbedRequest {
                target: EmbedTarget {
                    r#type: "sales_order".into(),
                    id: "SO-001".into(),
                    path: "/embed/sales/orders/SO-001".into(),
                },
                source: None,
                pubkey: keys.public_key().to_hex(),
                device_id: "wrong-device-0001".into(),
            },
            facts(),
        )
        .await
        .is_err());

    let issued = store
        .issue_embed(
            &principal,
            IssueEmbedRequest {
                target: EmbedTarget {
                    r#type: "sales_order".into(),
                    id: "SO-001".into(),
                    path: "/embed/sales/orders/SO-001".into(),
                },
                source: None,
                pubkey: keys.public_key().to_hex(),
                device_id: device_id.into(),
            },
            facts(),
        )
        .await
        .unwrap();
    let code = issued.embed_url.split("code=").nth(1).unwrap().to_string();
    let stored = sqlx::query("SELECT code_hash,extract(epoch from expires_at-created_at)::bigint AS ttl_seconds FROM embed_sessions WHERE id=$1")
        .bind(issued.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.get::<Vec<u8>, _>("code_hash").len(), 32);
    assert_eq!(stored.get::<i64, _>("ttl_seconds"), 30);
    assert!(!format!("{:?}", stored.get::<Vec<u8>, _>("code_hash")).contains(&code));
    let first_store = store.clone();
    let first_code = code.clone();
    let second_store = store.clone();
    let second_code = code.clone();
    let (a, b) = tokio::join!(
        async move { first_store.bootstrap(&first_code, facts()).await },
        async move { second_store.bootstrap(&second_code, facts()).await }
    );
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()),
        1,
        "one-time code must have exactly one winner: {:?} / {:?}",
        a.as_ref().err(),
        b.as_ref().err()
    );
    let bootstrap = a.ok().or_else(|| b.ok()).unwrap();
    assert_eq!(
        store
            .business_state(&bootstrap.session_token)
            .await
            .unwrap()
            .user_id,
        principal.user_id
    );
    assert!(
        store.bootstrap(&code, facts()).await.is_err(),
        "replay after consumption must fail"
    );

    let state = store
        .business_state(&bootstrap.session_token)
        .await
        .unwrap();
    store.business_logout(&state, facts()).await.unwrap();
    store.business_logout(&state, facts()).await.unwrap();
    assert!(store
        .business_state(&bootstrap.session_token)
        .await
        .is_err());

    sqlx::query("UPDATE embed_sessions SET created_at=now()-interval '2 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    let replacement_code = issue(&store, &principal, &keys, device_id).await;
    let replacement = store.bootstrap(&replacement_code, facts()).await.unwrap();
    for _ in 0..9 {
        let _ = issue(&store, &principal, &keys, device_id).await;
    }
    assert!(store
        .issue_embed(
            &principal,
            IssueEmbedRequest {
                target: EmbedTarget {
                    r#type: "sales_order".into(),
                    id: "SO-rate-limit".into(),
                    path: "/embed/sales/orders/SO-rate-limit".into(),
                },
                source: None,
                pubkey: keys.public_key().to_hex(),
                device_id: device_id.into(),
            },
            facts(),
        )
        .await
        .is_err());

    store
        .revoke_binding(&principal, binding.id, facts())
        .await
        .unwrap();
    assert!(
        store
            .business_state(&replacement.session_token)
            .await
            .is_err(),
        "device revocation must revoke Business sessions"
    );

    let replay_events:i64=sqlx::query_scalar("SELECT count(*) FROM security_audit_events WHERE event_type='EMBED_SESSION_REPLAY_REJECTED'").fetch_one(&pool).await.unwrap();
    assert!(replay_events >= 1);
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM security_audit_events WHERE event_type='BUSINESS_LOGOUT'"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
            >= 2
    );
    assert!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM security_audit_events WHERE event_type='BUSINESS_SESSION_REVOKED'").fetch_one(&pool).await.unwrap() >= 1);
    assert!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM security_audit_events WHERE event_type='EMBED_SESSION_AUDIENCE_REJECTED'").fetch_one(&pool).await.unwrap() >= 1);
    assert!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM security_audit_events WHERE event_type='EMBED_SESSION_RATE_LIMITED'").fetch_one(&pool).await.unwrap() >= 1);
    let audit_id: Uuid = sqlx::query_scalar("SELECT id FROM security_audit_events LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE security_audit_events SET result='failure' WHERE id=$1")
            .bind(audit_id)
            .execute(&pool)
            .await
            .is_err(),
        "audit rows must be append-only"
    );

    sqlx::query("UPDATE identity_binding_challenges SET expires_at=now()-interval '1 second',status='active' WHERE id=$1").bind(challenge.id).execute(&pool).await.unwrap();
    let (challenges, _, _) = store.cleanup().await.unwrap();
    assert_eq!(challenges, 1);

    let workbench_logout_claims = Claims {
        sub: "workbench-logout-user".into(),
        sid: Some("sid-workbench-logout".into()),
        ..claims.clone()
    };
    let workbench_logout_principal = store
        .principal(&workbench_logout_claims, &facts())
        .await
        .unwrap();
    store
        .workbench_logout(&workbench_logout_principal, false, facts())
        .await
        .unwrap();
    let global_logout_claims = Claims {
        sub: "global-logout-user".into(),
        sid: Some("sid-global-logout".into()),
        ..claims.clone()
    };
    let global_logout_principal = store
        .principal(&global_logout_claims, &facts())
        .await
        .unwrap();
    store
        .workbench_logout(&global_logout_principal, true, facts())
        .await
        .unwrap();
    let backchannel_claims = Claims {
        sub: "backchannel-sub-only-user".into(),
        sid: None,
        ..claims.clone()
    };
    let backchannel_principal = store
        .principal(&backchannel_claims, &facts())
        .await
        .unwrap();
    store
        .backchannel_logout(
            None,
            &backchannel_claims.iss,
            &backchannel_claims.sub,
            facts(),
        )
        .await
        .unwrap();
    for (event_type, workbench_session_id) in [
        (
            "WORKBENCH_LOGOUT",
            workbench_logout_principal.workbench_session_id,
        ),
        (
            "GLOBAL_LOGOUT",
            global_logout_principal.workbench_session_id,
        ),
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM security_audit_events WHERE event_type=$1 AND workbench_session_id=$2"
            )
            .bind(event_type)
            .bind(workbench_session_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM workbench_sessions WHERE id=$1")
            .bind(backchannel_principal.workbench_session_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "revoked"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM security_audit_events WHERE event_type='BACKCHANNEL_LOGOUT' AND enterprise_user_id=$1"
        )
        .bind(backchannel_principal.user_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );

    sqlx::query("UPDATE enterprise_users SET status='disabled' WHERE id=$1")
        .bind(principal.user_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.principal(&claims, &facts()).await.is_err());
}
