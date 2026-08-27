use business_auth_gateway::{
    agent::{
        AgentToolAuditEvent, AgentToolAuditRequest, AgentToolAuditResult,
        ConsumeAgentDelegationRequest, IssueAgentDelegationRequest, VerifyAgentDelegationRequest,
    },
    auth::{Audience, Claims},
    model::{ChallengeRequest, RequestFacts},
    security,
    store::{Rejection, Store},
    Config,
};
use chrono::Utc;
use nostr::{EventBuilder, Keys, Kind, Tag, TagKind};
use sqlx::{postgres::PgPoolOptions, Row};
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

fn config(database_url: String, max_calls: i32) -> Config {
    Config {
        database_url,
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
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
        business_agent_draft_write_enabled: true,
        business_read_mcp_audience: "business-read-mcp".into(),
        agent_delegation_ttl: Duration::from_secs(300),
        agent_delegation_max_calls: max_calls,
        business_agent_rate_limit_per_minute: 100,
        business_read_service_credential: Some("test-service-credential-at-least-32-bytes".into()),
    }
}

fn facts(trace_id: Uuid) -> RequestFacts {
    RequestFacts {
        ip: Some("127.0.0.1".into()),
        user_agent_hash: None,
        trace_id,
    }
}

#[tokio::test]
async fn delegation_is_hashed_scoped_atomic_and_revocable() {
    let Ok(database_url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL absent; PostgreSQL delegation test skipped");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await
        .expect("connect");
    Store::migrate(&pool).await.expect("migrate");
    sqlx::raw_sql("TRUNCATE security_audit_events,agent_read_delegations,business_sessions,embed_sessions,identity_binding_challenges,buzz_identity_bindings,workbench_sessions,enterprise_users,business_iam.authorization_decisions,business_iam.principal_permissions,business_iam.principal_roles,business_iam.role_permissions,business_iam.roles,business_iam.principals CASCADE")
        .execute(&pool)
        .await
        .expect("truncate");
    let store = Store::new(pool.clone(), config(database_url, 4));
    let claims = Claims {
        iss: "https://auth.test/application/o/workbench".into(),
        sub: "delegation-user".into(),
        exp: Utc::now().timestamp() + 3600,
        aud: Some(Audience::One("workbench".into())),
        azp: Some("workbench".into()),
        client_id: None,
        email: None,
        name: Some("Delegation User".into()),
        preferred_username: None,
        sid: Some("delegation-sid".into()),
        events: None,
    };
    let principal = store
        .principal(&claims, &facts(Uuid::new_v4()))
        .await
        .expect("principal");
    let human_iam_id = Uuid::new_v4();
    let proxy_iam_id = Uuid::new_v4();
    let independent_iam_id = Uuid::new_v4();
    let proxy_role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO business_iam.principals(id,kind,external_id,display_name) VALUES
         ($1,'human',$2,'Delegation User'),
         ($3,'proxy_agent','business-query-agent','Business Query Proxy'),
         ($4,'independent_agent','finance-independent-agent','Finance Digital Employee')",
    )
    .bind(human_iam_id)
    .bind(principal.user_id.to_string())
    .bind(proxy_iam_id)
    .bind(independent_iam_id)
    .execute(&pool)
    .await
    .expect("IAM principals");
    sqlx::query(
        "UPDATE business_iam.permissions
         SET obligations='[\"step_up_authentication\"]'::jsonb
         WHERE capability='sales_order:read'",
    )
    .execute(&pool)
    .await
    .expect("system permission obligation");
    sqlx::query(
        "INSERT INTO business_iam.principal_permissions(
           principal_id,permission_id,data_scope,obligations
         )
         SELECT $1,id,'{\"mode\":\"restricted\",\"dimensions\":{\"legal_entity\":[\"cn\",\"sg\"]}}'::jsonb,
                '[\"dual_control\"]'::jsonb
           FROM business_iam.permissions WHERE capability='sales_order:read'
         UNION ALL
         SELECT $2,id,'{\"mode\":\"unrestricted\"}'::jsonb,'[]'::jsonb
           FROM business_iam.permissions WHERE capability='inventory:read'",
    )
    .bind(human_iam_id)
    .bind(independent_iam_id)
    .execute(&pool)
    .await
    .expect("IAM grants");
    sqlx::query(
        "INSERT INTO business_iam.roles(id,code,name)
         VALUES($1,'business.query.proxy','Business Query Proxy')",
    )
    .bind(proxy_role_id)
    .execute(&pool)
    .await
    .expect("IAM proxy role");
    sqlx::query(
        "INSERT INTO business_iam.role_permissions(
           role_id,permission_id,data_scope,obligations
         )
         SELECT $1,id,'{\"mode\":\"restricted\",\"dimensions\":{\"legal_entity\":[\"cn\",\"us\"]}}'::jsonb,
                '[\"human_approval\"]'::jsonb
         FROM business_iam.permissions WHERE capability='sales_order:read'",
    )
    .bind(proxy_role_id)
    .execute(&pool)
    .await
    .expect("IAM proxy role permission");
    sqlx::query("INSERT INTO business_iam.principal_roles(principal_id,role_id) VALUES($1,$2)")
        .bind(proxy_iam_id)
        .bind(proxy_role_id)
        .execute(&pool)
        .await
        .expect("IAM proxy role assignment");
    let user_keys = Keys::generate();
    let challenge = store
        .challenge(
            &principal,
            ChallengeRequest {
                pubkey: user_keys.public_key().to_hex(),
                device_id: "delegation-device-01".into(),
                device_name: "Delegation Mac".into(),
                device_platform: "macos".into(),
            },
            facts(Uuid::new_v4()),
        )
        .await
        .expect("challenge");
    let signed = EventBuilder::new(Kind::Custom(24243), &challenge.payload)
        .sign_with_keys(&user_keys)
        .expect("sign binding");
    let binding = store
        .verify_binding(&principal, challenge.id, signed, facts(Uuid::new_v4()))
        .await
        .expect("binding");
    let channel = Uuid::new_v4();
    let source = EventBuilder::new(Kind::TextNote, "查一下 SO-001")
        .tags([Tag::custom(
            TagKind::Custom("h".into()),
            [channel.to_string()],
        )])
        .sign_with_keys(&user_keys)
        .expect("sign source");
    let trace_id = Uuid::new_v4();
    let issued = store
        .issue_agent_delegation(
            IssueAgentDelegationRequest {
                source_event: source.clone(),
                source_buzz_event_id: source.id.to_hex(),
                source_buzz_pubkey: source.pubkey.to_hex(),
                source_channel_id: channel.to_string(),
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-001".into(),
                scopes: vec!["sales_order:read".into(), "inventory:read".into()],
            },
            facts(trace_id),
        )
        .await
        .expect("issue");
    assert_eq!(issued.token.len(), 43);
    assert_eq!(issued.trace_id, trace_id);
    assert_eq!(issued.scopes, vec!["sales_order:read"]);
    let iam_decision = sqlx::query(
        "SELECT result,allowed_capabilities,denied_capabilities,effective_grants
         FROM business_iam.authorization_decisions WHERE trace_id=$1",
    )
    .bind(trace_id)
    .fetch_one(&pool)
    .await
    .expect("IAM decision");
    assert_eq!(iam_decision.get::<String, _>("result"), "partial");
    assert_eq!(
        iam_decision.get::<Vec<String>, _>("allowed_capabilities"),
        vec!["sales_order:read"]
    );
    assert_eq!(
        iam_decision.get::<Vec<String>, _>("denied_capabilities"),
        vec!["inventory:read"]
    );
    let obligations = iam_decision
        .get::<serde_json::Value, _>("effective_grants")
        .get(0)
        .and_then(|grant| grant.get("obligations"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .expect("effective obligations");
    assert_eq!(
        obligations,
        vec![
            serde_json::json!("human_approval"),
            serde_json::json!("step_up_authentication"),
            serde_json::json!("dual_control"),
        ]
    );

    let duplicate = store
        .issue_agent_delegation(
            IssueAgentDelegationRequest {
                source_event: source.clone(),
                source_buzz_event_id: source.id.to_hex(),
                source_buzz_pubkey: source.pubkey.to_hex(),
                source_channel_id: channel.to_string(),
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-duplicate".into(),
                scopes: vec!["sales_order:read".into()],
            },
            facts(Uuid::new_v4()),
        )
        .await;
    assert!(matches!(
        duplicate,
        Err(Rejection::Conflict("source_event_already_authorized"))
    ));
    let duplicate_rejection_audited: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM security_audit_events
         WHERE event_type='AGENT_TURN_REJECTED' AND reason_code='source_event_already_authorized')",
    )
    .fetch_one(&pool)
    .await
    .expect("duplicate rejection audit");
    assert!(duplicate_rejection_audited);

    let stored =
        sqlx::query("SELECT token_hash,status,used_calls FROM agent_read_delegations WHERE id=$1")
            .bind(issued.id)
            .fetch_one(&pool)
            .await
            .expect("stored delegation");
    assert_eq!(
        stored.get::<Vec<u8>, _>("token_hash"),
        security::hash(&issued.token)
    );
    assert_ne!(
        stored.get::<Vec<u8>, _>("token_hash"),
        issued.token.as_bytes()
    );

    let wrong_scope = store
        .consume_agent_delegation(
            &issued.token,
            ConsumeAgentDelegationRequest {
                tool_name: "query_inventory_balance".into(),
                required_scope: "inventory:read".into(),
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-001".into(),
            },
            facts(trace_id),
        )
        .await;
    assert!(wrong_scope.is_err());

    let mut calls = Vec::new();
    for _ in 0..8 {
        let cloned = store.clone();
        let token = issued.token.clone();
        calls.push(tokio::spawn(async move {
            cloned
                .consume_agent_delegation(
                    &token,
                    ConsumeAgentDelegationRequest {
                        tool_name: "get_sales_order".into(),
                        required_scope: "sales_order:read".into(),
                        agent_id: "business-query-agent".into(),
                        agent_turn_id: "turn-001".into(),
                    },
                    facts(trace_id),
                )
                .await
                .is_ok()
        }));
    }
    let mut successes = 0;
    for call in calls {
        if call.await.expect("join") {
            successes += 1;
        }
    }
    assert_eq!(successes, 4, "atomic call cap must allow exactly max_calls");
    let row = sqlx::query("SELECT status,used_calls FROM agent_read_delegations WHERE id=$1")
        .bind(issued.id)
        .fetch_one(&pool)
        .await
        .expect("delegation state");
    assert_eq!(row.get::<String, _>("status"), "exhausted");
    assert_eq!(row.get::<i32, _>("used_calls"), 4);
    let verified = store
        .verify_agent_delegation(
            VerifyAgentDelegationRequest {
                delegation_id: issued.id,
                enterprise_user_id: principal.user_id,
                identity_binding_id: binding.id,
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-001".into(),
                trace_id,
                used_calls: 4,
                required_scope: "sales_order:read".into(),
            },
            facts(trace_id),
        )
        .await
        .expect("verify IAM-backed delegation");
    assert_eq!(verified.capability.as_str(), "sales_order:read");
    assert_eq!(
        serde_json::to_value(verified.data_scope).expect("serialize effective scope"),
        serde_json::json!({
            "mode": "restricted",
            "dimensions": {"legal_entity": ["cn"]}
        })
    );

    store
        .revoke_agent_delegation(issued.id, facts(trace_id))
        .await
        .expect("revoke exhausted delegation");
    let status: String =
        sqlx::query_scalar("SELECT status FROM agent_read_delegations WHERE id=$1")
            .bind(issued.id)
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "revoked");
    assert_eq!(binding.status, "active");

    let live_proxy_source = EventBuilder::new(Kind::TextNote, "查一下下一张销售单")
        .tags([Tag::custom(
            TagKind::Custom("h".into()),
            [channel.to_string()],
        )])
        .sign_with_keys(&user_keys)
        .expect("sign live proxy source");
    let live_proxy = store
        .issue_agent_delegation(
            IssueAgentDelegationRequest {
                source_event: live_proxy_source.clone(),
                source_buzz_event_id: live_proxy_source.id.to_hex(),
                source_buzz_pubkey: live_proxy_source.pubkey.to_hex(),
                source_channel_id: channel.to_string(),
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-before-human-disable".into(),
                scopes: vec!["sales_order:read".into()],
            },
            facts(Uuid::new_v4()),
        )
        .await
        .expect("live proxy delegation");
    sqlx::query(
        "UPDATE business_iam.role_permissions
         SET data_scope='{\"mode\":\"restricted\",\"dimensions\":{\"legal_entity\":[\"cn\"]}}'::jsonb
         WHERE role_id=$1",
    )
    .bind(proxy_role_id)
    .execute(&pool)
    .await
    .expect("tighten proxy role");
    let role_revoked: String =
        sqlx::query_scalar("SELECT status FROM agent_read_delegations WHERE id=$1")
            .bind(live_proxy.id)
            .fetch_one(&pool)
            .await
            .expect("role-revoked proxy delegation");
    assert_eq!(role_revoked, "revoked");

    let post_role_source = EventBuilder::new(Kind::TextNote, "按新角色再查一张销售单")
        .tags([Tag::custom(
            TagKind::Custom("h".into()),
            [channel.to_string()],
        )])
        .sign_with_keys(&user_keys)
        .expect("sign post-role source");
    let post_role_proxy = store
        .issue_agent_delegation(
            IssueAgentDelegationRequest {
                source_event: post_role_source.clone(),
                source_buzz_event_id: post_role_source.id.to_hex(),
                source_buzz_pubkey: post_role_source.pubkey.to_hex(),
                source_channel_id: channel.to_string(),
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-after-role-change".into(),
                scopes: vec!["sales_order:read".into()],
            },
            facts(Uuid::new_v4()),
        )
        .await
        .expect("post-role proxy delegation");
    sqlx::query(
        "UPDATE business_iam.principals
         SET status='disabled',disabled_at=now(),version=version+1 WHERE id=$1",
    )
    .bind(human_iam_id)
    .execute(&pool)
    .await
    .expect("disable human IAM principal");
    let automatically_revoked: String =
        sqlx::query_scalar("SELECT status FROM agent_read_delegations WHERE id=$1")
            .bind(post_role_proxy.id)
            .fetch_one(&pool)
            .await
            .expect("automatically revoked proxy delegation");
    assert_eq!(automatically_revoked, "revoked");
    let authority_change_audited: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM security_audit_events
         WHERE delegation_id=$1 AND reason_code='iam_authority_changed')",
    )
    .bind(post_role_proxy.id)
    .fetch_one(&pool)
    .await
    .expect("authority change audit");
    assert!(authority_change_audited);
    let independent_source = EventBuilder::new(Kind::TextNote, "查一下库存")
        .tags([Tag::custom(
            TagKind::Custom("h".into()),
            [channel.to_string()],
        )])
        .sign_with_keys(&user_keys)
        .expect("sign independent source");
    let independent = store
        .issue_agent_delegation(
            IssueAgentDelegationRequest {
                source_event: independent_source.clone(),
                source_buzz_event_id: independent_source.id.to_hex(),
                source_buzz_pubkey: independent_source.pubkey.to_hex(),
                source_channel_id: channel.to_string(),
                agent_id: "finance-independent-agent".into(),
                agent_turn_id: "turn-independent".into(),
                scopes: vec!["inventory:read".into()],
            },
            facts(Uuid::new_v4()),
        )
        .await
        .expect("independent Agent uses its own persistent permission");
    assert_eq!(independent.scopes, vec!["inventory:read"]);

    let denied_source = EventBuilder::new(Kind::TextNote, "再查一张销售单")
        .tags([Tag::custom(
            TagKind::Custom("h".into()),
            [channel.to_string()],
        )])
        .sign_with_keys(&user_keys)
        .expect("sign denied proxy source");
    let denied_proxy = store
        .issue_agent_delegation(
            IssueAgentDelegationRequest {
                source_event: denied_source.clone(),
                source_buzz_event_id: denied_source.id.to_hex(),
                source_buzz_pubkey: denied_source.pubkey.to_hex(),
                source_channel_id: channel.to_string(),
                agent_id: "business-query-agent".into(),
                agent_turn_id: "turn-disabled-human".into(),
                scopes: vec!["sales_order:read".into()],
            },
            facts(Uuid::new_v4()),
        )
        .await;
    assert!(matches!(
        denied_proxy,
        Err(Rejection::Forbidden("business_iam_denied"))
    ));

    let anomaly_run_id = Uuid::new_v4();
    let response_event_id = "c".repeat(64);
    store
        .audit_agent_tool(
            AgentToolAuditRequest {
                delegation_id: issued.id,
                tool_name: "buzz_response".into(),
                event_type: AgentToolAuditEvent::AgentBusinessResponseEmitted,
                result: AgentToolAuditResult::Success,
                result_count: Some(2),
                finding_count: Some(2),
                resource_ref_count: Some(3),
                rule_set_version: Some("trade-risk-v1.0".into()),
                anomaly_run_id: Some(anomaly_run_id),
                response_buzz_event_id: Some(response_event_id.clone()),
                duration_ms: 42,
                reason_code: None,
                trace_id,
            },
            facts(trace_id),
        )
        .await
        .expect("response audit");
    let response_audit = sqlx::query(
        "SELECT response_buzz_event_id,finding_count,resource_ref_count,rule_set_version,anomaly_run_id
         FROM security_audit_events WHERE event_type='AGENT_BUSINESS_RESPONSE_EMITTED'",
    )
    .fetch_one(&pool)
    .await
    .expect("response audit row");
    assert_eq!(
        response_audit.get::<String, _>("response_buzz_event_id"),
        response_event_id
    );
    assert_eq!(response_audit.get::<i32, _>("finding_count"), 2);
    assert_eq!(response_audit.get::<i32, _>("resource_ref_count"), 3);
    assert_eq!(
        response_audit.get::<String, _>("rule_set_version"),
        "trade-risk-v1.0"
    );
    assert_eq!(
        response_audit.get::<Uuid, _>("anomaly_run_id"),
        anomaly_run_id
    );

    let audited_secret: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM security_audit_events WHERE metadata::text LIKE '%' || $1 || '%')",
    )
    .bind(&issued.token)
    .fetch_one(&pool)
    .await
    .expect("audit scan");
    assert!(
        !audited_secret,
        "delegation token must never enter audit metadata"
    );
}
