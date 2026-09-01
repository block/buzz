//! Approval resumes must not relabel current content as a captured revision.
use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};

#[derive(Default)]
struct RecordingActionSink {
    messages: std::sync::Mutex<Vec<String>>,
}

impl buzz_workflow::ActionSink for RecordingActionSink {
    fn send_message(
        &self,
        _context: buzz_workflow::action_sink::WorkflowMessageContext,
        _channel_id: &str,
        text: &str,
        _authored_text: &str,
        _author_pubkey: &str,
        _reply_to: Option<&str>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<String, buzz_workflow::ActionSinkError>>
                + Send
                + '_,
        >,
    > {
        self.messages
            .lock()
            .expect("recording action sink lock")
            .push(text.to_string());
        Box::pin(async { Ok("recorded-event".to_string()) })
    }
}

async fn manual_trigger_test_context() -> (Arc<AppState>, TenantContext, Keys, Keys, Uuid, Event) {
    use buzz_core::channel::{ChannelType, ChannelVisibility};

    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
    let setup_pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect workflow trigger setup database");
    // The harness prepares the schema (CI uses pgschema, not SQLx history).
    let setup_db = buzz_db::Db::from_pool(setup_pool.clone());

    let host = format!("workflow-trigger-{}.example", Uuid::new_v4().simple());
    let community = setup_db
        .ensure_configured_community(&host)
        .await
        .expect("create workflow trigger test community")
        .id;
    let tenant = TenantContext::resolved(community, host.clone());
    let human = Keys::generate();
    let agent = Keys::generate();
    let human_bytes = human.public_key().to_bytes();
    let agent_bytes = agent.public_key().to_bytes();
    setup_db
        .ensure_user(community, &human_bytes)
        .await
        .expect("ensure human owner");
    setup_db
        .ensure_user(community, &agent_bytes)
        .await
        .expect("ensure managed agent");
    assert!(setup_db
        .set_agent_owner(community, &agent_bytes, &human_bytes)
        .await
        .expect("set immutable agent owner"));
    let channel = setup_db
        .create_channel(
            community,
            "manual-trigger-pool",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &agent_bytes,
            None,
        )
        .await
        .expect("create workflow channel");
    let workflow_id = Uuid::new_v4();
    let definition = EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_DEF as u16),
            concat!(
                "name: manual-trigger-pool\n",
                "trigger:\n  on: message_posted\n",
                "steps:\n",
                "  - id: approval\n    action: request_approval\n    from: '@owner'\n    message: approve\n",
                "  - id: send\n    action: send_message\n    text: done\n",
            ),
        )
        .tags(vec![
            Tag::parse(["d", workflow_id.to_string().as_str()]).expect("d tag"),
            Tag::parse(["h", channel.id.to_string().as_str()]).expect("h tag"),
        ])
        .sign_with_keys(&agent)
        .expect("sign workflow definition");
    let (_, definition_json) = buzz_workflow::WorkflowEngine::parse_yaml(&definition.content)
        .expect("parse signed workflow definition");
    let definition_hash = compute_definition_hash(&definition_json);
    let mut tx = setup_db
        .begin_transaction()
        .await
        .expect("begin workflow seed");
    buzz_db::event::insert_event_in_transaction(&mut tx, community, &definition, Some(channel.id))
        .await
        .expect("persist signed workflow definition");
    setup_db
        .upsert_workflow(
            &mut tx,
            community,
            workflow_id,
            Some(channel.id),
            &agent_bytes,
            "manual-trigger-pool",
            &definition_json,
            &definition_hash,
            definition.id.as_bytes(),
        )
        .await
        .expect("materialize signed workflow");
    tx.commit().await.expect("commit signed workflow");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(&url)
        .await
        .expect("connect one-connection workflow trigger pool");
    let db = buzz_db::Db::from_pool(pool.clone());
    let mut config = crate::config::Config::from_env().expect("config from env");
    config.database_url = url;
    config.redis_url = "redis://127.0.0.1:1".to_string();
    config.relay_url = format!("wss://{host}");
    config.require_relay_membership = false;
    let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("redis pool config");
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .expect("pubsub manager"),
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool);
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
        Keys::generate(),
        media_storage,
    );
    setup_pool.close().await;
    (
        Arc::new(state),
        tenant,
        human,
        agent,
        workflow_id,
        definition,
    )
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn approval_resume_executes_the_run_bound_signed_revision() {
    let (state, tenant, _human, agent, workflow_id, revision_a) =
        manual_trigger_test_context().await;
    let community_id = tenant.community();
    let db = state.db.clone();
    let trigger_context = serde_json::to_value(TriggerContext {
        channel_id: exact_tag_value(&revision_a, "h")
            .unwrap_or_default()
            .to_string(),
        ..TriggerContext::default()
    })
    .expect("serialize trigger context");
    let run_id = db
        .create_workflow_run(
            community_id,
            workflow_id,
            Some(revision_a.id.as_bytes()),
            None,
            Some(&trigger_context),
        )
        .await
        .expect("create revision A run");
    db.update_workflow_run(
        community_id,
        run_id,
        RunStatus::WaitingApproval,
        0,
        &serde_json::json!([{
            "step_id": "approval",
            "output": {"approved": true}
        }]),
        None,
    )
    .await
    .expect("suspend revision A run for approval");

    let channel_id = Uuid::parse_str(exact_tag_value(&revision_a, "h").expect("channel tag"))
        .expect("channel UUID");
    let revision_b = EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_DEF as u16),
            concat!(
                "name: revision-b\n",
                "trigger:\n  on: message_posted\n",
                "steps:\n",
                "  - id: approval\n    action: request_approval\n    from: '@owner'\n    message: approve\n",
                "  - id: after\n    action: send_message\n    text: revision B\n",
            ),
        )
        .tags(vec![
            Tag::parse(["d", workflow_id.to_string().as_str()]).expect("d tag"),
            Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag"),
        ])
        .sign_with_keys(&agent)
        .expect("sign revision B");
    let (_, definition_b_json) =
        buzz_workflow::WorkflowEngine::parse_yaml(&revision_b.content).expect("parse revision B");
    let definition_b_hash = compute_definition_hash(&definition_b_json);
    let mut tx = db
        .begin_transaction()
        .await
        .expect("begin revision B update");
    buzz_db::event::insert_event_in_transaction(
        &mut tx,
        community_id,
        &revision_b,
        Some(channel_id),
    )
    .await
    .expect("persist revision B");
    db.upsert_workflow(
        &mut tx,
        community_id,
        workflow_id,
        Some(channel_id),
        &agent.public_key().to_bytes(),
        "revision-b",
        &definition_b_json,
        &definition_b_hash,
        revision_b.id.as_bytes(),
    )
    .await
    .expect("materialize revision B");
    tx.commit().await.expect("commit revision B update");

    let sink = Arc::new(RecordingActionSink::default());
    state.workflow_engine.set_action_sink(sink.clone());
    resume_workflow_after_approval(
        Arc::clone(&state.workflow_engine),
        db.clone(),
        community_id,
        run_id,
        workflow_id,
        1,
    )
    .await;

    let resumed = db
        .get_workflow_run(community_id, run_id)
        .await
        .expect("load resumed run");
    assert_eq!(resumed.status, RunStatus::Completed);
    assert_eq!(
        sink.messages
            .lock()
            .expect("recorded messages lock")
            .as_slice(),
        ["done"],
        "approval resume must execute revision A, never current revision B"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn approval_resume_fails_closed_without_a_signed_run_revision() {
    let (state, tenant, _human, _agent, workflow_id, _revision) =
        manual_trigger_test_context().await;
    let community_id = tenant.community();
    let db = state.db.clone();
    let run_id = db
        .create_workflow_run(community_id, workflow_id, None, None, None)
        .await
        .expect("create legacy revisionless run");
    db.update_workflow_run(
        community_id,
        run_id,
        RunStatus::WaitingApproval,
        0,
        &serde_json::json!([]),
        None,
    )
    .await
    .expect("suspend revisionless run");

    resume_workflow_after_approval(
        Arc::clone(&state.workflow_engine),
        db.clone(),
        community_id,
        run_id,
        workflow_id,
        1,
    )
    .await;

    let failed = db
        .get_workflow_run(community_id, run_id)
        .await
        .expect("load failed run");
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("invalid_definition"));
    assert!(failed
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("no owner-signed definition revision")));
}

fn exact_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut values = event.tags.iter().filter_map(|tag| {
        (tag.kind().to_string() == name)
            .then(|| tag.content())
            .flatten()
    });
    let value = values.next()?;
    values.next().is_none().then_some(value)
}
