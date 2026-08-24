//! Doorbell routing regression: a workflow's owner-signed `@Name` may add
//! a wake target, but dynamic trigger/webhook values cannot become routing
//! authority because mention extraction reads the signed template only.
//!
//! Postgres-gated like the other DB-backed relay tests. Run with:
//!   `cargo test -p buzz-relay --lib workflow_sink -- --ignored`
use super::*;
use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
use buzz_db::CreateCommunityWithOwnerResult;
use std::sync::Arc;

/// Real-PG state mirroring `handlers::event::tests::test_state_with_redis_url`.
async fn test_state_with_database_url(database_url: Option<String>) -> Arc<AppState> {
    let mut config = crate::config::Config::from_env().expect("default config loads");
    if let Some(database_url) = database_url {
        config.database_url = database_url;
    }
    config.require_relay_membership = false;
    config.workflow_agent_delivery_enabled = true;
    let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
    let db = buzz_db::Db::from_pool(pool.clone());
    let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("redis pool");
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .expect("pubsub manager"),
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool.clone());
    let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
    let (state, _audit_shutdown) = AppState::new(
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
    Arc::new(state)
}

async fn test_state() -> Arc<AppState> {
    test_state_with_database_url(None).await
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desired_schema_only_bootstrap_runs_durable_workflow_delivery_path() {
    let base_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string() // sadscan:disable np.postgres.1 -- local test-only credentials
        });
    let admin = sqlx::PgPool::connect(&base_url)
        .await
        .expect("connect admin database");
    let scratch_name = format!("buzz_workflow_schema_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE {scratch_name}"
    )))
    .execute(&admin)
    .await
    .expect("create desired-schema workflow database");
    let (base_prefix, _) = base_url.rsplit_once('/').expect("database URL has path");
    let scratch_url = format!("{base_prefix}/{scratch_name}");
    let bootstrap = sqlx::PgPool::connect(&scratch_url)
        .await
        .expect("connect desired-schema workflow database");
    sqlx::raw_sql(sqlx::AssertSqlSafe(include_str!(
        "../../../../schema/schema.sql"
    )))
    .execute(&bootstrap)
    .await
    .expect("apply desired-state schema without migrations");
    bootstrap.close().await;

    let runtime_pool = sqlx::PgPool::connect(&scratch_url)
        .await
        .expect("connect runtime assertion pool");
    let state = test_state_with_database_url(Some(scratch_url)).await;
    let owner = nostr::Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let host = format!("wf-schema-{}.example", Uuid::new_v4().simple());
    let community = match state
        .db
        .create_community_with_owner(&host, &owner_hex)
        .await
        .expect("create community through runtime DB path")
    {
        CreateCommunityWithOwnerResult::Created(record) => record.id,
        other => panic!("expected fresh community, got {other:?}"),
    };
    state
        .db
        .ensure_user(community, &owner.public_key().to_bytes())
        .await
        .expect("ensure workflow owner");
    let channel = state
        .db
        .create_channel(
            community,
            "schema-workflow",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner.public_key().to_bytes(),
            None,
        )
        .await
        .expect("create workflow channel");
    let workflow_id = Uuid::new_v4();
    let definition = EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF as u16),
        "name: schema-only\ntrigger:\n  on: webhook\nsteps:\n  - id: notify\n    action: send_message\n    text: schema-only\n",
    )
    .tags([
        Tag::parse(["d", &workflow_id.to_string()]).expect("d tag"),
        Tag::parse(["h", &channel.id.to_string()]).expect("h tag"),
    ])
    .sign_with_keys(&owner)
    .expect("sign workflow definition");
    state
        .db
        .insert_event(community, &definition, Some(channel.id))
        .await
        .expect("persist workflow definition");
    let (_, definition_json) =
        buzz_workflow::WorkflowEngine::parse_yaml(&definition.content).expect("parse workflow");
    let definition_hash =
        <sha2::Sha256 as sha2::Digest>::digest(definition_json.as_bytes()).to_vec();
    state
        .db
        .upsert_workflow(
            community,
            workflow_id,
            Some(channel.id),
            &owner.public_key().to_bytes(),
            "schema-only",
            &definition_json,
            &definition_hash,
            definition.id.as_bytes(),
            true,
        )
        .await
        .expect("materialize workflow");
    let trigger = serde_json::to_value(buzz_workflow::executor::TriggerContext {
        channel_id: channel.id.to_string(),
        definition_event_id: definition.id.to_hex(),
        cause: Some(WorkflowCause::Webhook),
        ..Default::default()
    })
    .expect("serialize trigger");
    let run_id = state
        .db
        .create_workflow_run(community, workflow_id, None, Some(&trigger))
        .await
        .expect("create workflow run");
    state
        .db
        .update_workflow_run(
            community,
            run_id,
            buzz_db::workflow::RunStatus::Running,
            0,
            &serde_json::json!([]),
            None,
        )
        .await
        .expect("start workflow run");
    let event_id = RelayActionSink::new(&state)
        .send_message(
            community,
            workflow_id,
            "notify",
            &channel.id.to_string(),
            "schema-only",
            &owner_hex,
            &DoorbellContext {
                definition_event_id: definition.id.to_hex(),
                run_id,
                attempt: 1,
                cause: WorkflowCause::Webhook,
            },
            None,
        )
        .await
        .expect("run actual durable workflow message path");
    let delivery_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM workflow_agent_deliveries WHERE community_id = $1 AND run_id = $2",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .fetch_one(&runtime_pool)
    .await
    .expect("lookup desired-schema delivery identity");
    let wrong_binding = buzz_db::workflow::WorkflowAgentDeliveryBinding {
        run_id,
        step_id: "notify".to_string(),
        definition_event_id: definition.id.as_bytes().to_vec(),
        message_event_id: nostr::EventId::from_hex(&event_id)
            .unwrap()
            .as_bytes()
            .to_vec(),
        channel_id: Uuid::new_v4(),
    };
    assert!(
        state
            .db
            .claim_workflow_agent_delivery(
                community,
                &owner.public_key().to_bytes(),
                Some(delivery_id),
                Some(&wrong_binding),
                120,
            )
            .await
            .expect("reject mismatched live-wake binding before claim")
            .is_none(),
        "an authenticated but cross-channel wake must not mutate the victim delivery"
    );
    let binding = buzz_db::workflow::WorkflowAgentDeliveryBinding {
        channel_id: channel.id,
        ..wrong_binding
    };
    let delivery = state
        .db
        .claim_workflow_agent_delivery(
            community,
            &owner.public_key().to_bytes(),
            Some(delivery_id),
            Some(&binding),
            120,
        )
        .await
        .expect("claim desired-schema delivery")
        .expect("workflow path created a durable delivery");
    assert_eq!(delivery.run_id, run_id);
    assert_eq!(
        delivery.message_event_id,
        nostr::EventId::from_hex(&event_id).unwrap().as_bytes()
    );

    drop(state);
    runtime_pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {scratch_name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop desired-schema workflow database");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_doorbell_routes_from_signed_template() {
    let state = test_state().await;
    let pool = sqlx::PgPool::connect(&state.config.database_url)
        .await
        .expect("connect integration pool");

    let author = nostr::Keys::generate();
    let author_hex = author.public_key().to_hex();
    let agent = nostr::Keys::generate();
    let agent_bytes = agent.public_key().to_bytes().to_vec();

    let host = format!("wf-ptag-{}.example", uuid::Uuid::new_v4().simple());
    let community = match state
        .db
        .create_community_with_owner(&host, &author_hex)
        .await
        .expect("create community")
    {
        CreateCommunityWithOwnerResult::Created(rec) => rec.id,
        other => panic!("expected fresh community, got {other:?}"),
    };

    state
        .db
        .ensure_user(community, &author.public_key().to_bytes())
        .await
        .expect("ensure workflow author user row");

    // Open channel; the creator (author) is bootstrapped as an owner-member.
    let channel = state
        .db
        .create_channel(
            community,
            "wf-ptag",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &author.public_key().to_bytes(),
            None,
        )
        .await
        .expect("create channel");

    // The mentioned agent is a real member with a resolvable display name.
    state
        .db
        .ensure_user(community, &agent_bytes)
        .await
        .expect("ensure agent user row");
    state
        .db
        .update_user_profile(community, &agent_bytes, Some("Robby"), None, None, None)
        .await
        .expect("set agent display name");
    state
        .db
        .add_member(
            community,
            channel.id,
            &agent_bytes,
            MemberRole::Bot,
            Some(&author.public_key().to_bytes()),
        )
        .await
        .expect("add agent member");

    let workflow_id = Uuid::new_v4();
    let definition = EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF as u16),
        "name: ptag\ntrigger:\n  on: webhook\nsteps:\n  - id: notify\n    action: send_message\n    text: '{{trigger.text}}'\n  - id: follow_up\n    action: send_message\n    text: 'previous={{steps.call.output.body}}'\n  - id: concurrent\n    action: send_message\n    text: 'parallel'\n  - id: atomic_targets\n    action: send_message\n    text: 'atomic @Robby'\n  - id: threaded\n    action: send_message\n    text: 'threaded @Robby'\n",
    )
    .tags([
        Tag::parse(["d", &workflow_id.to_string()]).expect("d tag"),
        Tag::parse(["h", &channel.id.to_string()]).expect("h tag"),
    ])
    .sign_with_keys(&author)
    .expect("sign definition");
    state
        .db
        .insert_event(community, &definition, Some(channel.id))
        .await
        .expect("persist definition");

    let (_, definition_json) =
        buzz_workflow::WorkflowEngine::parse_yaml(&definition.content).expect("parse definition");
    let definition_hash =
        <sha2::Sha256 as sha2::Digest>::digest(definition_json.as_bytes()).to_vec();
    state
        .db
        .upsert_workflow(
            community,
            workflow_id,
            Some(channel.id),
            &author.public_key().to_bytes(),
            "ptag",
            &definition_json,
            &definition_hash,
            definition.id.as_bytes(),
            true,
        )
        .await
        .expect("materialize workflow");
    let trigger_context = buzz_workflow::executor::TriggerContext {
        channel_id: channel.id.to_string(),
        definition_event_id: definition.id.to_hex(),
        cause: Some(WorkflowCause::Webhook),
        webhook_fields: [("private_token".to_string(), "relay-secret".to_string())].into(),
        ..Default::default()
    };
    let trigger_json = serde_json::to_value(&trigger_context).expect("trigger JSON");
    let execution_trace = serde_json::json!([{
        "step_id": "call",
        "output": {"body": "durable-prior-output"}
    }]);
    let run_id = state
        .db
        .create_workflow_run(community, workflow_id, None, Some(&trigger_json))
        .await
        .expect("create run");
    state
        .db
        .update_workflow_run(
            community,
            run_id,
            buzz_db::workflow::RunStatus::Running,
            1,
            &execution_trace,
            None,
        )
        .await
        .expect("persist prior-step trace");

    let sink = RelayActionSink::new(&state);

    // Current-main reply threading and durable delivery must commit as one slice.
    let thread_root = EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), "root")
        .tags([Tag::parse(["h", &channel.id.to_string()]).expect("root h tag")])
        .sign_with_keys(&author)
        .expect("sign thread root");
    let root_created_at =
        chrono::DateTime::from_timestamp(thread_root.created_at.as_secs() as i64, 0)
            .expect("valid root timestamp");
    state
        .db
        .insert_event_with_thread_metadata(
            community,
            &thread_root,
            Some(channel.id),
            Some(buzz_db::event::ThreadMetadataParams {
                event_id: thread_root.id.as_bytes(),
                event_created_at: root_created_at,
                channel_id: channel.id,
                parent_event_id: None,
                parent_event_created_at: None,
                root_event_id: None,
                root_event_created_at: None,
                depth: 0,
                broadcast: false,
            }),
        )
        .await
        .expect("persist thread root");
    let threaded_event_id = sink
        .send_message(
            community,
            workflow_id,
            "threaded",
            &channel.id.to_string(),
            "threaded @Robby",
            &author_hex,
            &DoorbellContext {
                definition_event_id: definition.id.to_hex(),
                run_id,
                attempt: 1,
                cause: WorkflowCause::Webhook,
            },
            Some(&thread_root.id.to_hex()),
        )
        .await
        .expect("send threaded workflow message");
    let threaded_event_bytes = nostr::EventId::from_hex(&threaded_event_id)
        .expect("threaded event id")
        .as_bytes()
        .to_vec();
    let threaded_meta = state
        .db
        .get_thread_metadata_by_event(community, &threaded_event_bytes)
        .await
        .expect("load threaded metadata")
        .expect("threaded metadata committed");
    assert_eq!(
        threaded_meta.parent_event_id.as_deref(),
        Some(thread_root.id.as_bytes().as_slice())
    );
    assert_eq!(
        threaded_meta.root_event_id.as_deref(),
        Some(thread_root.id.as_bytes().as_slice())
    );
    assert_eq!(threaded_meta.depth, 1);
    let threaded_targets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_agent_deliveries WHERE community_id=$1 AND run_id=$2 AND step_id='threaded' AND message_event_id=$3",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(&threaded_event_bytes)
    .fetch_one(&pool)
    .await
    .expect("count threaded delivery targets");
    assert_eq!(threaded_targets, 2);
    sqlx::query(
        "UPDATE workflow_agent_deliveries SET status='delivered' WHERE community_id=$1 AND run_id=$2 AND step_id='threaded'",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .execute(&pool)
    .await
    .expect("complete threaded regression deliveries");

    // Inject a failure after the event insert reaches the transaction but before
    // any durable target can commit. The retry must leave one event and all targets.
    sqlx::raw_sql(
        r#"
        CREATE OR REPLACE FUNCTION fail_workflow_delivery_insert() RETURNS trigger AS $$
        BEGIN RAISE EXCEPTION 'injected delivery failure'; END; $$ LANGUAGE plpgsql;
        CREATE TRIGGER fail_workflow_delivery BEFORE INSERT ON workflow_agent_deliveries
        FOR EACH ROW EXECUTE FUNCTION fail_workflow_delivery_insert();
    "#,
    )
    .execute(&pool)
    .await
    .expect("install after-event failure");
    let atomic_doorbell = DoorbellContext {
        definition_event_id: definition.id.to_hex(),
        run_id,
        attempt: 1,
        cause: WorkflowCause::Webhook,
    };
    assert!(sink
        .send_message(
            community,
            workflow_id,
            "atomic_targets",
            &channel.id.to_string(),
            "atomic @Robby",
            &author_hex,
            &atomic_doorbell,
            None,
        )
        .await
        .is_err());
    sqlx::query("DROP TRIGGER fail_workflow_delivery ON workflow_agent_deliveries")
        .execute(&pool)
        .await
        .expect("remove after-event failure");
    let atomic_event_id = sink
        .send_message(
            community,
            workflow_id,
            "atomic_targets",
            &channel.id.to_string(),
            "atomic @Robby",
            &author_hex,
            &atomic_doorbell,
            None,
        )
        .await
        .expect("retry after event-boundary rollback");
    let atomic_event_bytes = nostr::EventId::from_hex(&atomic_event_id)
        .unwrap()
        .as_bytes()
        .to_vec();
    let atomic_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(&atomic_event_bytes)
            .fetch_one(&pool)
            .await
            .unwrap();
    let atomic_targets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_agent_deliveries WHERE community_id=$1 AND run_id=$2 AND step_id='atomic_targets'")
        .bind(community.as_uuid()).bind(run_id).fetch_one(&pool).await.unwrap();
    assert_eq!((atomic_visible, atomic_targets), (1, 2));

    // Now remove the committed atomic slice and fail specifically on the second
    // (mentioned-agent) target. Owner insertion precedes it inside the same
    // transaction; rollback plus retry must restore the identical all-target state.
    sqlx::query("DELETE FROM workflow_agent_deliveries WHERE community_id=$1 AND run_id=$2 AND step_id='atomic_targets'")
        .bind(community.as_uuid()).bind(run_id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(&atomic_event_bytes)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        r#"
        CREATE OR REPLACE FUNCTION fail_second_workflow_target() RETURNS trigger AS $$
        BEGIN IF encode(NEW.target_pubkey, 'hex') = current_setting('buzz.test_fail_target') THEN
            RAISE EXCEPTION 'injected second target failure'; END IF; RETURN NEW; END;
        $$ LANGUAGE plpgsql;
        CREATE TRIGGER fail_second_workflow_target BEFORE INSERT ON workflow_agent_deliveries
        FOR EACH ROW EXECUTE FUNCTION fail_second_workflow_target();
    "#,
    )
    .execute(&pool)
    .await
    .expect("install second-target failure");
    sqlx::query("SELECT set_config('buzz.test_fail_target', $1, false)")
        .bind(agent.public_key().to_hex())
        .execute(&pool)
        .await
        .expect("set second failure target");
    assert!(sink
        .send_message(
            community,
            workflow_id,
            "atomic_targets",
            &channel.id.to_string(),
            "atomic @Robby",
            &author_hex,
            &atomic_doorbell,
            None,
        )
        .await
        .is_err());
    sqlx::query("DROP TRIGGER fail_second_workflow_target ON workflow_agent_deliveries")
        .execute(&pool)
        .await
        .expect("remove second-target failure");
    let recovered_id = sink
        .send_message(
            community,
            workflow_id,
            "atomic_targets",
            &channel.id.to_string(),
            "atomic @Robby",
            &author_hex,
            &atomic_doorbell,
            None,
        )
        .await
        .expect("retry after second-target rollback");
    let recovered_bytes = nostr::EventId::from_hex(&recovered_id)
        .unwrap()
        .as_bytes()
        .to_vec();
    let recovered_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(&recovered_bytes)
            .fetch_one(&pool)
            .await
            .unwrap();
    let recovered_targets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_agent_deliveries WHERE community_id=$1 AND run_id=$2 AND step_id='atomic_targets' AND message_event_id=$3")
        .bind(community.as_uuid()).bind(run_id).bind(&recovered_bytes).fetch_one(&pool).await.unwrap();
    assert_eq!((recovered_visible, recovered_targets), (1, 2));
    sqlx::query("UPDATE workflow_agent_deliveries SET status='delivered' WHERE community_id=$1 AND run_id=$2 AND step_id='atomic_targets'")
        .bind(community.as_uuid()).bind(run_id).execute(&pool).await.unwrap();

    let event_id_hex = sink
        .send_message(
            community,
            workflow_id,
            "notify",
            &channel.id.to_string(),
            "heads up @Robby — please take a look",
            &author_hex,
            &DoorbellContext {
                definition_event_id: definition.id.to_hex(),
                run_id,
                attempt: 1,
                cause: WorkflowCause::Webhook,
            },
            None,
        )
        .await
        .expect("send_message");

    let id_bytes = nostr::EventId::from_hex(&event_id_hex)
        .expect("event id")
        .as_bytes()
        .to_vec();
    let stored = state
        .db
        .get_event_by_id(community, &id_bytes)
        .await
        .expect("query event")
        .expect("event persisted");

    let p_tag_targets: Vec<&str> = stored
        .event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"))
        .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
        .collect();

    assert_eq!(
        p_tag_targets,
        vec![author_hex.as_str()],
        "rendered @Robby from trigger data must not become a second wake target"
    );

    let workflow_owner_targets: Vec<&str> = stored
        .event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("workflow-owner"))
        .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
        .collect();
    assert_eq!(
        workflow_owner_targets,
        vec![author_hex.as_str()],
        "workflow output must carry exactly one dedicated owner authority tag"
    );

    let definition_targets: Vec<&[String]> = stored
        .event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("workflow-definition"))
        .map(|t| t.as_slice())
        .collect();
    assert_eq!(definition_targets.len(), 1);
    assert_eq!(
        definition_targets[0],
        [
            "workflow-definition",
            definition.id.to_hex().as_str(),
            "notify"
        ]
    );
    assert_eq!(
        stored.event.content, "heads up @Robby — please take a look",
        "ordinary workflow output must remain visible in the channel"
    );
    let causes: Vec<&[String]> = stored
        .event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("workflow-cause"))
        .map(|tag| tag.as_slice())
        .collect();
    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0], ["workflow-cause", "webhook", ""]);
    assert_eq!(
        stored
            .event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("workflow-run"))
            .map(|tag| tag.as_slice())
            .collect::<Vec<_>>(),
        vec![["workflow-run", run_id.to_string().as_str()]],
        "visible message must bind the immutable run consumed by ACP"
    );
    assert_eq!(
        stored
            .event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("workflow-step"))
            .map(|tag| tag.as_slice())
            .collect::<Vec<_>>(),
        vec![["workflow-step", "notify"]],
        "visible message must bind the immutable step consumed by ACP"
    );
    assert_eq!(
        stored.event.pubkey,
        state.relay_keypair.public_key(),
        "workflow output must be signed by the relay identity"
    );

    assert!(
        !stored.event.content.contains("relay-secret"),
        "private webhook cargo must not enter the visible message"
    );
    let later_trace = serde_json::json!([{
        "step_id": "call",
        "output": {"body": "mutable-later-output"}
    }]);
    let later_trigger = serde_json::json!({
        "channel_id": channel.id.to_string(),
        "definition_event_id": definition.id.to_hex(),
        "webhook_fields": {"private_token": "mutable-later-secret"}
    });
    sqlx::query("UPDATE workflow_runs SET trigger_context=$1 WHERE community_id=$2 AND id=$3")
        .bind(&later_trigger)
        .bind(community.as_uuid())
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("advance live run trigger context after delivery snapshot");
    state
        .db
        .update_workflow_run(
            community,
            run_id,
            buzz_db::workflow::RunStatus::Running,
            2,
            &later_trace,
            None,
        )
        .await
        .expect("advance live run after delivery snapshot");

    let delivery = state
        .db
        .claim_workflow_agent_delivery(community, &author.public_key().to_bytes(), None, None, 120)
        .await
        .expect("claim durable workflow delivery")
        .expect("delivery exists for signed target");
    assert_eq!(delivery.run_id, run_id);
    assert_eq!(delivery.step_id, "notify");
    assert_eq!(delivery.message_event_id, id_bytes);
    assert_eq!(
        delivery
            .trigger_context
            .as_ref()
            .and_then(|value| value.get("webhook_fields"))
            .and_then(|value| value.get("private_token"))
            .and_then(serde_json::Value::as_str),
        Some("relay-secret"),
        "private webhook cargo must remain available only through the durable claim"
    );
    assert_eq!(delivery.workflow_id, workflow_id);
    assert_eq!(delivery.channel_id, channel.id);
    assert_eq!(delivery.target_pubkey, author.public_key().to_bytes());
    assert_eq!(delivery.definition_event_id, definition.id.as_bytes());
    assert_eq!(
        delivery.execution_trace, execution_trace,
        "claim must return the immutable delivery snapshot, not the advanced run trace"
    );
    assert_ne!(delivery.execution_trace, later_trace);
    assert_eq!(
        delivery.trigger_context.as_ref(),
        Some(&trigger_json),
        "claim must return the immutable delivery trigger snapshot"
    );
    assert_ne!(delivery.trigger_context.as_ref(), Some(&later_trigger));

    // Cross a Nostr timestamp second so signing again would necessarily create
    // a distinct visible event. Durable identity must win before signing.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let replay_event_id = sink
        .send_message(
            community,
            workflow_id,
            "notify",
            &channel.id.to_string(),
            "heads up @Robby — please take a look",
            &author_hex,
            &DoorbellContext {
                definition_event_id: definition.id.to_hex(),
                run_id,
                attempt: 2,
                cause: WorkflowCause::Webhook,
            },
            None,
        )
        .await
        .expect("replay same step");
    assert_eq!(
        replay_event_id, event_id_hex,
        "true replay across a timestamp second returns the canonical event"
    );
    let visible_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND kind=9 AND id IN (SELECT message_event_id FROM workflow_agent_deliveries WHERE community_id=$1 AND run_id=$2 AND step_id=$3)",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind("notify")
    .fetch_one(&pool)
    .await
    .expect("count canonical visible events");
    assert_eq!(
        visible_count, 1,
        "delayed replay must not publish a duplicate"
    );

    let concurrent_step = "concurrent";
    let concurrent_doorbell = DoorbellContext {
        definition_event_id: definition.id.to_hex(),
        run_id,
        attempt: 1,
        cause: WorkflowCause::Webhook,
    };
    let (left, right) = tokio::join!(
        sink.send_message(
            community,
            workflow_id,
            concurrent_step,
            &channel.id.to_string(),
            "parallel",
            &author_hex,
            &concurrent_doorbell,
            None,
        ),
        sink.send_message(
            community,
            workflow_id,
            concurrent_step,
            &channel.id.to_string(),
            "parallel",
            &author_hex,
            &concurrent_doorbell,
            None,
        )
    );
    assert_eq!(
        left.expect("left concurrent send"),
        right.expect("right concurrent send"),
        "concurrent retries serialize on durable identity"
    );
    let concurrent_visible_count: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*) FROM events
        WHERE community_id=$1 AND kind=9
          AND tags @> $2::jsonb
          AND tags @> $3::jsonb
        "#,
    )
    .bind(community.as_uuid())
    .bind(serde_json::json!([["workflow-run", run_id.to_string()]]))
    .bind(serde_json::json!([["workflow-step", concurrent_step]]))
    .fetch_one(&pool)
    .await
    .expect("count concurrent visible events");
    assert_eq!(concurrent_visible_count, 1);
    sqlx::query(
        "UPDATE workflow_agent_deliveries SET status='delivered' WHERE community_id=$1 AND run_id=$2 AND step_id=$3",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(concurrent_step)
    .execute(&pool)
    .await
    .expect("complete concurrent regression delivery");

    let follow_up_event_id = sink
        .send_message(
            community,
            workflow_id,
            "follow_up",
            &channel.id.to_string(),
            "previous=durable-prior-output",
            &author_hex,
            &DoorbellContext {
                definition_event_id: definition.id.to_hex(),
                run_id,
                attempt: 1,
                cause: WorkflowCause::Webhook,
            },
            None,
        )
        .await
        .expect("send distinct step");
    assert_ne!(
        follow_up_event_id, event_id_hex,
        "distinct steps in one run remain distinct"
    );
    let delivery_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_agent_deliveries WHERE community_id=$1 AND run_id=$2 AND target_pubkey=$3",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(author.public_key().to_bytes())
    .fetch_one(&pool)
    .await
    .expect("count deliveries");
    assert_eq!(
        delivery_count, 5,
        "replay collapses while five distinct step identities survive"
    );

    let claim_token = delivery.claim_token.expect("claim has fencing token");
    let original_expiry = delivery.claim_expires_at.expect("claim has expiry");
    sqlx::query(
        "UPDATE workflow_agent_deliveries SET claim_expires_at=NOW()+INTERVAL '1 second' WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(delivery.id)
    .execute(&pool)
    .await
    .expect("move active claim to current lease boundary");
    let renewed_expiry = state
        .db
        .renew_workflow_agent_delivery(
            community,
            delivery.id,
            &author.public_key().to_bytes(),
            claim_token,
            7_600,
        )
        .await
        .expect("renew active claim")
        .expect("current owner and token renew");
    assert!(renewed_expiry > original_expiry);
    assert!(state
        .db
        .claim_workflow_agent_delivery(
            community,
            &author.public_key().to_bytes(),
            Some(delivery.id),
            None,
            120,
        )
        .await
        .expect("competing claim while renewed")
        .is_none());
    assert!(state
        .db
        .renew_workflow_agent_delivery(
            community,
            delivery.id,
            &author.public_key().to_bytes(),
            Uuid::new_v4(),
            7_600,
        )
        .await
        .expect("stale renewal result")
        .is_none());

    sqlx::query(
        "UPDATE workflow_agent_deliveries SET status='pending', claim_token=NULL, claim_owner=NULL, claim_expires_at=NULL, expires_at=NOW()+INTERVAL '119 seconds' WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(delivery.id)
    .execute(&pool)
    .await
    .expect("leave less lifetime than requested lease");
    assert!(
        state
            .db
            .claim_workflow_agent_delivery(
                community,
                &author.public_key().to_bytes(),
                Some(delivery.id),
                None,
                120,
            )
            .await
            .expect("short-lived claim admission")
            .is_none(),
        "claim admission never promises a lease beyond row expiry"
    );
    sqlx::query(
        "UPDATE workflow_agent_deliveries SET status='claimed', claim_token=$3, claim_owner=$4, claim_expires_at=NOW()+INTERVAL '7600 seconds', expires_at=NOW()+make_interval(secs => $5) WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(delivery.id)
    .bind(claim_token)
    .bind(author.public_key().to_bytes().as_slice())
    .bind(buzz_core::workflow_delivery::ROW_LIFETIME_SECONDS as f64)
    .execute(&pool)
    .await
    .expect("restore active claim and durable row lifetime");

    let row_expires_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT expires_at FROM workflow_agent_deliveries WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(delivery.id)
    .fetch_one(&pool)
    .await
    .expect("read durable row lifetime");
    let required_lifetime =
        chrono::Duration::seconds(buzz_core::workflow_delivery::ROW_LIFETIME_SECONDS);
    assert!(
        row_expires_at - chrono::Utc::now() >= required_lifetime - chrono::Duration::seconds(5),
        "durable row must outlive the maximum admitted lease"
    );

    assert!(state
        .db
        .finish_workflow_agent_delivery(
            community,
            delivery.id,
            &author.public_key().to_bytes(),
            claim_token,
            false,
            true,
            Some("agent_busy"),
            Some("retry later"),
        )
        .await
        .expect("record retryable finish"));
    sqlx::query(
        "UPDATE workflow_agent_deliveries SET next_attempt_at=NOW() WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(delivery.id)
    .execute(&pool)
    .await
    .expect("make retry due");
    let retry = state
        .db
        .claim_workflow_agent_delivery(
            community,
            &author.public_key().to_bytes(),
            Some(delivery.id),
            None,
            120,
        )
        .await
        .expect("retry claim")
        .expect("retryable delivery becomes claimable");
    assert_eq!(retry.attempt, 2);
    assert_ne!(retry.claim_token, Some(claim_token));

    sqlx::query(
        "UPDATE workflow_agent_deliveries SET claim_expires_at=NOW()-INTERVAL '1 second' WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(delivery.id)
    .execute(&pool)
    .await
    .expect("expire lease");
    assert!(
        state
            .db
            .claim_workflow_agent_delivery(
                community,
                &author.public_key().to_bytes(),
                Some(delivery.id),
                None,
                120,
            )
            .await
            .expect("claim expired lease")
            .is_none(),
        "an expired exclusive claim is never reassigned"
    );
    let terminal = state
        .db
        .reap_workflow_agent_deliveries()
        .await
        .expect("reap expired exclusive lease");
    assert!(terminal
        .iter()
        .any(|row| { row.id == delivery.id && row.status == "failed" && row.attempt == 2 }));
    assert!(state
        .db
        .claim_workflow_agent_delivery(
            community,
            &author.public_key().to_bytes(),
            Some(delivery.id),
            None,
            120,
        )
        .await
        .expect("terminal claim result")
        .is_none());

    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
    };
    use tower::ServiceExt;

    let claim_body = serde_json::json!({"delivery_id": null}).to_string();
    let claim_response = crate::router::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workflows/agent-deliveries/claim")
                .header(header::HOST, &host)
                .header("x-pubkey", &author_hex)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(claim_body))
                .expect("claim request"),
        )
        .await
        .expect("claim response");
    assert_eq!(claim_response.status(), StatusCode::OK);
    let claim_json: serde_json::Value = serde_json::from_slice(
        &to_bytes(claim_response.into_body(), 1024 * 1024)
            .await
            .expect("claim response body"),
    )
    .expect("claim response JSON");
    let follow_up = &claim_json["delivery"];
    assert_eq!(follow_up["step_id"], "follow_up");
    assert_eq!(follow_up["workflow_id"], workflow_id.to_string());
    assert_eq!(follow_up["run_id"], run_id.to_string());
    let follow_up_id = follow_up["id"].as_str().unwrap().parse::<Uuid>().unwrap();
    let follow_up_token = follow_up["claim_token"]
        .as_str()
        .unwrap()
        .parse::<Uuid>()
        .unwrap();

    let finish_path = format!("/workflows/agent-deliveries/{follow_up_id}/finish");
    let finish_body = serde_json::json!({
        "claim_token": follow_up_token,
        "delivered": true,
        "retryable": false
    })
    .to_string();
    let finish_response = crate::router::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&finish_path)
                .header(header::HOST, &host)
                .header("x-pubkey", &author_hex)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(finish_body.clone()))
                .expect("finish request"),
        )
        .await
        .expect("finish response");
    assert_eq!(finish_response.status(), StatusCode::OK);
    let finish_json: serde_json::Value = serde_json::from_slice(
        &to_bytes(finish_response.into_body(), 1024 * 1024)
            .await
            .expect("finish response body"),
    )
    .expect("finish response JSON");
    assert_eq!(finish_json["completed"], true);

    let stale_response = crate::router::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&finish_path)
                .header(header::HOST, &host)
                .header("x-pubkey", &author_hex)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(finish_body))
                .expect("stale finish request"),
        )
        .await
        .expect("replayed finish response");
    assert_eq!(stale_response.status(), StatusCode::OK);
}
