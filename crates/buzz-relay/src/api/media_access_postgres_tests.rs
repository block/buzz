use super::*;
use buzz_db::channel::{ChannelType, ChannelVisibility};
use buzz_db::moderation::{NewReport, ReportTarget};
use nostr::{EventBuilder, Keys, Kind};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

async fn fixture() -> (Arc<AppState>, PgPool, TenantContext) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = buzz_db::Db::from_pool(pool.clone());
    let host = format!("media-review-{}.example", Uuid::new_v4().simple());
    let community = db.ensure_configured_community(&host).await.unwrap().id;
    let mut config = crate::config::Config::for_test();
    config.require_relay_membership = false;
    config.media_read_mode = MediaReadMode::Enforce;
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new("redis://127.0.0.1:1", redis.clone())
            .await
            .unwrap(),
    );
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let storage = buzz_media::MediaStorage::new(&config.media).unwrap();
    let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let (state, _) = AppState::new(
        config,
        db,
        redis,
        buzz_audit::AuditService::new(pool.clone()),
        pubsub,
        auth,
        buzz_search::SearchService::new(pool.clone()),
        workflow,
        Keys::generate(),
        storage,
    );
    (
        Arc::new(state),
        pool,
        TenantContext::resolved(community, host),
    )
}

fn mode(state: &mut Arc<AppState>, mode: MediaReadMode) {
    Arc::make_mut(&mut Arc::get_mut(state).unwrap().config).media_read_mode = mode;
}

async fn check(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    hash: &str,
) -> Result<(), MediaError> {
    enforce(state, tenant, keys.public_key().as_bytes(), hash).await
}

async fn post(
    state: &AppState,
    tenant: &TenantContext,
    owner: &Keys,
    channel: Uuid,
) -> (String, nostr::Event) {
    let hash = format!("{:064x}", Uuid::new_v4().as_u128());
    let lease = state
        .db
        .deletion_store()
        .acquire_serving_write_lease(
            tenant.community(),
            "media_upload",
            "media-review-test",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    state
        .db
        .record_media_uploader(&lease, &hash, owner.public_key().as_bytes())
        .await
        .unwrap();
    let event = EventBuilder::new(Kind::from(9), format!("/media/{hash}.jpg"))
        .sign_with_keys(owner)
        .unwrap();
    state
        .db
        .insert_event(tenant.community(), &event, Some(channel))
        .await
        .unwrap();
    (hash, event)
}

async fn report(
    state: &AppState,
    tenant: &TenantContext,
    reporter: &Keys,
    target: ReportTarget,
    note: Option<&str>,
) -> Uuid {
    state
        .db
        .insert_moderation_report(
            tenant.community(),
            NewReport {
                report_event_id: EventBuilder::new(Kind::from(1984), Uuid::new_v4().to_string())
                    .sign_with_keys(reporter)
                    .unwrap()
                    .id
                    .as_bytes(),
                reporter_pubkey: reporter.public_key().as_bytes(),
                target,
                channel_id: None,
                report_type: "spam",
                note,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rollout_and_report_review_preserve_private_file_boundaries() {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (mut state, pool, tenant) = fixture().await;
    let (owner, reviewer, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
    let community = tenant.community();
    let private = state
        .db
        .create_channel(
            community,
            "reported",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let (hash, source) = post(&state, &tenant, &owner, private).await;
    let (unrelated, _) = post(&state, &tenant, &owner, private).await;
    for policy in [
        MediaReadMode::Off,
        MediaReadMode::LogOnly,
        MediaReadMode::Enforce,
    ] {
        mode(&mut state, policy);
        assert_eq!(
            check(&state, &tenant, &stranger, &hash).await.is_ok(),
            policy != MediaReadMode::Enforce
        );
        assert!(check(&state, &tenant, &owner, &hash).await.is_ok());
    }
    state
        .db
        .add_relay_member(community, &reviewer.public_key().to_hex(), "admin", None)
        .await
        .unwrap();
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &hash).await,
            Err(MediaError::NotFound)
        ),
        "role alone grants nothing"
    );
    let report_id = report(
        &state,
        &tenant,
        &owner,
        ReportTarget::Event(source.id.to_bytes().to_vec()),
        Some(&format!("/media/{unrelated}.jpg")),
    )
    .await;
    mode(&mut state, MediaReadMode::LogOnly);
    assert!(check(&state, &tenant, &reviewer, &hash).await.is_ok());
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE community_id = $1 AND action = 'moderation_media_read'")
        .bind(community.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        before, 0,
        "log-only must not claim an exceptional read occurred"
    );
    mode(&mut state, MediaReadMode::Enforce);
    assert!(check(&state, &tenant, &reviewer, &hash).await.is_ok());
    let audit: (Vec<u8>, String, serde_json::Value) = sqlx::query_as(
        "SELECT actor_pubkey, object_id, detail FROM audit_log WHERE community_id = $1 AND action = 'moderation_media_read' ORDER BY seq DESC LIMIT 1")
        .bind(community.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(audit.0, reviewer.public_key().to_bytes());
    assert_eq!(audit.1, hash);
    assert_eq!(audit.2["report_id"], report_id.to_string());
    assert_eq!(audit.2["event_id"], source.id.to_hex());
    assert!(matches!(
        check(&state, &tenant, &stranger, &hash).await,
        Err(MediaError::NotFound)
    ));
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &unrelated).await,
            Err(MediaError::NotFound)
        ),
        "report notes are not attachments"
    );
    report(
        &state,
        &tenant,
        &owner,
        ReportTarget::Blob(hex::decode(&unrelated).unwrap()),
        None,
    )
    .await;
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &unrelated).await,
            Err(MediaError::NotFound)
        ),
        "a raw reported hash grants nothing"
    );
    for role in ["member", "owner", "admin"] {
        state
            .db
            .update_relay_member_role(community, &reviewer.public_key().to_hex(), role)
            .await
            .unwrap();
        assert_eq!(
            check(&state, &tenant, &reviewer, &hash).await.is_ok(),
            role != "member"
        );
    }
    // Other communities cannot supply the grant, report, or queue role.
    let other = state
        .db
        .ensure_configured_community(&format!("other-{}.example", Uuid::new_v4()))
        .await
        .unwrap();
    let other_tenant = TenantContext::resolved(other.id, other.host);
    state
        .db
        .add_relay_member(other.id, &reviewer.public_key().to_hex(), "admin", None)
        .await
        .unwrap();
    assert!(matches!(
        check(&state, &other_tenant, &reviewer, &hash).await,
        Err(MediaError::NotFound)
    ));
    state
        .db
        .resolve_moderation_report(
            community,
            report_id,
            "dismissed",
            reviewer.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &hash).await,
            Err(MediaError::NotFound)
        ),
        "closing a report revokes the exception"
    );
    report(
        &state,
        &tenant,
        &owner,
        ReportTarget::Event(source.id.to_bytes().to_vec()),
        None,
    )
    .await;
    assert!(check(&state, &tenant, &reviewer, &hash).await.is_ok());
    let audit = Arc::get_mut(&mut state).unwrap().audit.take();
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &hash).await,
            Err(MediaError::AuthorizationUnavailable)
        ),
        "auditing is required"
    );
    Arc::get_mut(&mut state).unwrap().audit = audit;
    let broken_pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    broken_pool.close().await;
    let audit = Arc::get_mut(&mut state)
        .unwrap()
        .audit
        .replace(Arc::new(buzz_audit::AuditService::new(broken_pool)));
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &hash).await,
            Err(MediaError::AuthorizationUnavailable)
        ),
        "audit commit failure must deny before serving bytes"
    );
    Arc::get_mut(&mut state).unwrap().audit = audit;
    assert!(
        !state
            .db
            .can_read_blob(community, &hash, reviewer.public_key().as_bytes())
            .await
            .unwrap(),
        "report review creates no permanent share"
    );
    state
        .db
        .soft_delete_event_and_update_thread(community, source.id.as_bytes(), None, None)
        .await
        .unwrap();
    assert!(
        matches!(
            check(&state, &tenant, &reviewer, &hash).await,
            Err(MediaError::NotFound)
        ),
        "deleted messages grant no exception"
    );
    pool.close().await;
    for policy in [
        MediaReadMode::Off,
        MediaReadMode::LogOnly,
        MediaReadMode::Enforce,
    ] {
        mode(&mut state, policy);
        assert_eq!(
            check(&state, &tenant, &stranger, &hash).await.is_ok(),
            policy != MediaReadMode::Enforce,
            "lookup failure follows rollout policy"
        );
    }
    let counts = snapshotter.snapshot().into_vec();
    for (policy, outcome) in [
        ("log-only", "deny"),
        ("log-only", "error"),
        ("enforce", "deny"),
        ("enforce", "report"),
    ] {
        assert!(
            counts.iter().any(|(key, _, _, value)| key.key().name()
                == "buzz_media_read_access_total"
                && key
                    .key()
                    .labels()
                    .any(|l| l.key() == "mode" && l.value() == policy)
                && key
                    .key()
                    .labels()
                    .any(|l| l.key() == "outcome" && l.value() == outcome)
                && matches!(value, metrics_util::debugging::DebugValue::Counter(n) if *n > 0)),
            "missing {policy}/{outcome} rollout count"
        );
    }
    assert!(counts
        .iter()
        .filter(|(key, _, _, _)| key.key().name() == "buzz_media_read_access_total")
        .all(|(key, _, _, _)| key
            .key()
            .labels()
            .all(|l| matches!(l.key(), "mode" | "outcome"))));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn every_rollout_mode_keeps_http_authentication_and_community_admission() {
    use axum::{body::Body, http::Request, routing::get, Router};
    use base64::Engine;
    use nostr::{JsonUtil, Tag, Timestamp};
    use tower::ServiceExt;

    let (mut state, _, tenant) = fixture().await;
    Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config).require_relay_membership = true;
    let stranger = Keys::generate();
    for policy in [
        MediaReadMode::Off,
        MediaReadMode::LogOnly,
        MediaReadMode::Enforce,
    ] {
        mode(&mut state, policy);
        for method in ["GET", "HEAD"] {
            for authenticated in [false, true] {
                let auth = EventBuilder::new(Kind::from(24242), "read")
                    .tags([
                        Tag::parse(["t", "get"]).unwrap(),
                        Tag::parse(["server", tenant.host()]).unwrap(),
                        Tag::parse(["expiration", &(Timestamp::now().as_secs() + 55).to_string()])
                            .unwrap(),
                    ])
                    .sign_with_keys(&stranger)
                    .unwrap();
                let mut request = Request::builder()
                    .method(method)
                    .uri(format!("/media/{}.jpg", "a".repeat(64)))
                    .header("host", tenant.host());
                if authenticated {
                    request = request.header(
                        "authorization",
                        format!(
                            "Nostr {}",
                            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(auth.as_json())
                        ),
                    );
                }
                let response = Router::new()
                    .route(
                        "/media/{sha256_ext}",
                        get(super::super::media::get_blob).head(super::super::media::head_blob),
                    )
                    .with_state(state.clone())
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    response.status().as_u16(),
                    if authenticated { 403 } else { 401 },
                    "{policy:?} {method}"
                );
            }
        }
    }
}
