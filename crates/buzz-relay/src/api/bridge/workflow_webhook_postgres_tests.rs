use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    Json,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::{workflow_webhook, WebhookQuery};
use crate::{
    config::Config,
    state::{AppState, AuditShutdownHandle},
    test_support::database_url,
};

async fn app_state(db: buzz_db::Db, pool: &PgPool) -> (Arc<AppState>, AuditShutdownHandle) {
    let mut config = Config::for_test();
    config.database_url = database_url();
    config.redis_url =
        std::env::var("REDIS_URL").expect("REDIS_URL must point to the isolated test Redis");
    config.relay_url = "wss://workflow-webhook-test.invalid".to_owned();
    config.require_auth_token = false;
    config.require_relay_membership = false;

    let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("create Redis pool");
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .expect("create pubsub manager"),
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool.clone());
    let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let media_storage = buzz_media::MediaStorage::new(&config.media).expect("create media storage");

    let (state, audit_shutdown) = AppState::new(
        config,
        db,
        redis_pool,
        audit,
        pubsub,
        auth,
        search,
        workflow_engine,
        nostr::Keys::generate(),
        media_storage,
    );
    (Arc::new(state), audit_shutdown)
}

async fn seed_community(pool: &PgPool, community_id: Uuid, host: &str) {
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community_id)
        .bind(host)
        .execute(pool)
        .await
        .expect("insert community");
}

struct WebhookWorkflowSeed<'a> {
    community_id: Uuid,
    workflow_id: Uuid,
    channel_id: Uuid,
    owner_pubkey: &'a [u8],
    owner_role: &'a str,
    name: &'a str,
    secret: &'a str,
    steps: Value,
}

async fn seed_webhook_workflow(pool: &PgPool, seed: WebhookWorkflowSeed<'_>) {
    sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2)")
        .bind(seed.community_id)
        .bind(seed.owner_pubkey)
        .execute(pool)
        .await
        .expect("insert workflow owner");
    sqlx::query(
        "INSERT INTO channels (community_id, id, name, created_by) VALUES ($1, $2, $3, $4)",
    )
    .bind(seed.community_id)
    .bind(seed.channel_id)
    .bind(format!("workflow-{}", seed.name))
    .bind(seed.owner_pubkey)
    .execute(pool)
    .await
    .expect("insert workflow channel");
    sqlx::query(
        "INSERT INTO channel_members (community_id, channel_id, pubkey, role) \
         VALUES ($1, $2, $3, $4::member_role)",
    )
    .bind(seed.community_id)
    .bind(seed.channel_id)
    .bind(seed.owner_pubkey)
    .bind(seed.owner_role)
    .execute(pool)
    .await
    .expect("insert workflow owner membership");

    let definition = json!({
        "name": seed.name,
        "trigger": {"on": "webhook"},
        "steps": seed.steps,
        "_webhook_secret": seed.secret
    });
    sqlx::query(
        "INSERT INTO workflows \
         (community_id, id, name, owner_pubkey, channel_id, definition, definition_hash) \
         VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7)",
    )
    .bind(seed.community_id)
    .bind(seed.workflow_id)
    .bind(seed.name)
    .bind(seed.owner_pubkey)
    .bind(seed.channel_id)
    .bind(definition.to_string())
    .bind(vec![0u8; 32])
    .execute(pool)
    .await
    .expect("insert workflow");
}

async fn post_webhook(
    state: &Arc<AppState>,
    workflow_id: Uuid,
    host: &str,
    secret: &str,
) -> (StatusCode, Value) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::HOST,
        HeaderValue::from_bytes(host.as_bytes()).expect("host header"),
    );
    headers.insert(
        "x-webhook-secret",
        HeaderValue::from_bytes(secret.as_bytes()).expect("secret header"),
    );

    match workflow_webhook(
        State(Arc::clone(state)),
        Path(workflow_id.to_string()),
        Query(WebhookQuery { secret: None }),
        headers,
        Bytes::new(),
    )
    .await
    {
        Ok((status, Json(body))) | Err((status, Json(body))) => (status, body),
    }
}

async fn workflow_run_count(pool: &PgPool, community_id: Uuid, workflow_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_runs WHERE community_id = $1 AND workflow_id = $2",
    )
    .bind(community_id)
    .bind(workflow_id)
    .fetch_one(pool)
    .await
    .expect("count workflow runs")
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis"]
async fn webhook_admission_enforces_owner_ban_tenant_scope_and_lookup_failure() {
    let pool = PgPool::connect(&database_url())
        .await
        .expect("connect to isolated PostgreSQL test database");
    let db = buzz_db::Db::from_pool(pool.clone());

    let community_a = Uuid::new_v4();
    let community_b = Uuid::new_v4();
    let host_a = format!("webhook-a-{}.test", Uuid::new_v4().simple());
    let host_b = format!("webhook-b-{}.test", Uuid::new_v4().simple());
    let owner_pubkey = [0x31; 32];
    let actor_pubkey = [0x52; 32];
    let member_pubkey = [0x73; 32];
    let workflow_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();
    let secret = format!("webhook-secret-{}", Uuid::new_v4().simple());

    seed_community(&pool, community_a, &host_a).await;
    seed_community(&pool, community_b, &host_b).await;
    // Deliberately reuse workflow and channel IDs across tenants. The webhook
    // request host must choose which community's owner restriction applies.
    let delay_step = || {
        json!([{
            "id": "hold",
            "action": "delay",
            "duration": "1s"
        }])
    };
    seed_webhook_workflow(
        &pool,
        WebhookWorkflowSeed {
            community_id: community_a,
            workflow_id,
            channel_id,
            owner_pubkey: &owner_pubkey,
            owner_role: "owner",
            name: "owner-workflow-a",
            secret: &secret,
            steps: delay_step(),
        },
    )
    .await;
    seed_webhook_workflow(
        &pool,
        WebhookWorkflowSeed {
            community_id: community_b,
            workflow_id,
            channel_id,
            owner_pubkey: &owner_pubkey,
            owner_role: "owner",
            name: "owner-workflow-b",
            secret: &secret,
            steps: delay_step(),
        },
    )
    .await;

    let member_workflow_id = Uuid::new_v4();
    let member_channel_id = Uuid::new_v4();
    seed_webhook_workflow(
        &pool,
        WebhookWorkflowSeed {
            community_id: community_a,
            workflow_id: member_workflow_id,
            channel_id: member_channel_id,
            owner_pubkey: &member_pubkey,
            owner_role: "member",
            name: "member-exfiltration-workflow",
            secret: &secret,
            steps: json!([{
                "id": "send",
                "action": "call_webhook",
                "url": "https://workflow-webhook-test.invalid"
            }]),
        },
    )
    .await;

    let (state, audit_shutdown) = app_state(db.clone(), &pool).await;

    let (status, _) = post_webhook(&state, workflow_id, &host_a, "wrong-secret").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the secret remains required"
    );
    assert_eq!(workflow_run_count(&pool, community_a, workflow_id).await, 0);

    let (status, _) = post_webhook(&state, workflow_id, &host_a, &secret).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "an unbanned owner is admitted"
    );
    assert_eq!(workflow_run_count(&pool, community_a, workflow_id).await, 1);

    db.ban_community_member(
        buzz_core::CommunityId::from_uuid(community_a),
        &owner_pubkey,
        &actor_pubkey,
        None,
        None,
    )
    .await
    .expect("ban workflow owner");
    let (status, body) = post_webhook(&state, workflow_id, &host_a, &secret).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a banned owner is denied generically"
    );
    assert_eq!(body["error"], "workflow not found");
    assert_eq!(
        workflow_run_count(&pool, community_a, workflow_id).await,
        1,
        "denial must not enqueue a workflow run"
    );

    let (status, _) = post_webhook(&state, workflow_id, &host_b, &secret).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "a ban in community A must not block the same owner in community B"
    );
    assert_eq!(workflow_run_count(&pool, community_b, workflow_id).await, 1);

    assert!(db
        .unban_community_member(
            buzz_core::CommunityId::from_uuid(community_a),
            &owner_pubkey,
            &actor_pubkey,
        )
        .await
        .expect("unban workflow owner"));
    let (status, _) = post_webhook(&state, workflow_id, &host_a, &secret).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "an explicitly unbanned owner is admitted"
    );
    assert_eq!(workflow_run_count(&pool, community_a, workflow_id).await, 2);

    let (status, _) = post_webhook(&state, member_workflow_id, &host_a, &secret).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the existing elevated-role check still rejects member-owned exfiltration workflows"
    );
    assert_eq!(
        workflow_run_count(&pool, community_a, member_workflow_id).await,
        0,
        "role denial must not enqueue a workflow run"
    );

    audit_shutdown
        .drain(std::time::Duration::from_secs(1))
        .await;
    drop(state);

    let (failing_db, admin_pool, schema) =
        crate::test_support::restriction_lookup_failing_db().await;
    let (failing_state, failing_audit_shutdown) = app_state(failing_db, &pool).await;
    let (status, body) = post_webhook(&failing_state, workflow_id, &host_a, &secret).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "restriction lookup failure denies admission"
    );
    assert_eq!(body["error"], "workflow not found");
    assert_eq!(
        workflow_run_count(&pool, community_a, workflow_id).await,
        2,
        "a failed restriction lookup must not enqueue a workflow run"
    );
    failing_audit_shutdown
        .drain(std::time::Duration::from_secs(1))
        .await;
    drop(failing_state);

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await
        .expect("drop restriction lookup failure schema");
}
