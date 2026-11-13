//! Regression for automation messages missing NIP-30 custom emoji tags.
//! Uses the real workflow executor, relay action sink and PostgreSQL storage.

use std::sync::Arc;

use buzz_core::channel::{ChannelType, ChannelVisibility};
use buzz_core::tenant::CommunityId;
use buzz_db::CreateCommunityWithOwnerResult;
use buzz_relay::state::AppState;
use buzz_relay::workflow_sink::RelayActionSink;
use uuid::Uuid;

async fn test_state() -> Arc<AppState> {
    let mut config = buzz_relay::config::Config::from_env().expect("test config");
    config.require_relay_membership = false;
    config.redis_url = "redis://127.0.0.1:1".to_string();
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

async fn execute_send_message_workflow(
    state: &Arc<AppState>,
    community: CommunityId,
    channel_id: Uuid,
    owner_pubkey: &[u8],
    name: &str,
    authored_text: &str,
    trigger_text: &str,
) -> String {
    let definition = serde_json::json!({
        "name": name,
        "trigger": {"on": "message_posted"},
        "steps": [{
            "id": "send",
            "action": "send_message",
            "text": authored_text,
        }],
        "enabled": true,
    });
    let definition_hash_byte = name.as_bytes().first().copied().unwrap_or_default();
    let workflow_id = state
        .db
        .create_workflow(
            community,
            Some(channel_id),
            owner_pubkey,
            name,
            &definition.to_string(),
            &[definition_hash_byte; 32],
        )
        .await
        .expect("create workflow");
    let trigger_ctx = buzz_workflow::executor::TriggerContext {
        text: trigger_text.to_owned(),
        channel_id: channel_id.to_string(),
        ..Default::default()
    };
    let trigger_ctx_json = serde_json::to_value(&trigger_ctx).expect("serialize trigger");
    let run_id = state
        .db
        .create_workflow_run(community, workflow_id, None, Some(&trigger_ctx_json))
        .await
        .expect("create workflow run");

    // Load the definition back from Postgres before execution. This pins the
    // authority source to the durable owner-authored template rather than a
    // second test-only string passed directly to RelayActionSink.
    let stored_workflow = state
        .db
        .get_workflow(community, workflow_id)
        .await
        .expect("load stored workflow");
    let stored_definition: buzz_workflow::WorkflowDef =
        serde_json::from_value(stored_workflow.definition).expect("parse stored definition");
    let result = buzz_workflow::executor::execute_run(
        &state.workflow_engine,
        community,
        run_id,
        &stored_definition,
        &trigger_ctx,
    )
    .await
    .expect("execute workflow");

    result.step_outputs["send"]["event_id"]
        .as_str()
        .expect("send_message event id")
        .to_owned()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_send_message_attaches_community_emoji_tags() {
    let state = test_state().await;
    let owner = nostr::Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let mut communities = Vec::new();
    for prefix in ["wf-emoji", "wf-foreign-emoji"] {
        let host = format!("{prefix}-{}.example", Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &owner_hex)
            .await
            .expect("create emoji community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };
        communities.push(community);
    }
    let community = communities[0];
    let owner_bytes = owner.public_key().to_bytes();
    state
        .db
        .ensure_user(community, &owner_bytes)
        .await
        .expect("ensure owner");
    let channel = state
        .db
        .create_channel(
            community,
            "wf-emoji",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner_bytes,
            None,
        )
        .await
        .expect("create emoji channel");

    // Each set is signed by a different publisher so collisions are live
    // palette entries, rather than superseded versions of one member's set.
    let now = nostr::Timestamp::now().as_secs();
    for (scope, age, entries) in [
        (
            community,
            3,
            vec![("octopus", "https://cdn.example/old.webp")],
        ),
        (
            community,
            2,
            vec![
                ("octopus", "https://cdn.example/octopus.webp"),
                ("chat-bubble", "https://cdn.example/chat.webp"),
                ("espresso-cup", "https://cdn.example/coffee.webp"),
                ("binoculars", "https://cdn.example/binoculars.webp"),
                ("unused", "https://cdn.example/unused.webp"),
            ],
        ),
        (
            community,
            2,
            vec![("octopus", "https://cdn.example/z-octopus.webp")],
        ),
        (
            communities[1],
            1,
            vec![
                ("octopus", "https://foreign.example/octopus.webp"),
                ("foreign-only", "https://foreign.example/icon.webp"),
            ],
        ),
    ] {
        let emojis: Vec<buzz_sdk::CustomEmoji> = entries
            .into_iter()
            .map(|(shortcode, url)| buzz_sdk::CustomEmoji {
                shortcode: shortcode.to_owned(),
                url: url.to_owned(),
            })
            .collect();
        let publisher = nostr::Keys::generate();
        let event = buzz_sdk::build_custom_emoji_set(&emojis)
            .expect("build palette")
            .custom_created_at(nostr::Timestamp::from(now - age))
            .sign_with_keys(&publisher)
            .expect("sign palette");
        state
            .db
            .insert_event(scope, &event, None)
            .await
            .expect("persist palette");
    }
    state
        .workflow_engine
        .set_action_sink(Arc::new(RelayActionSink::new(&state)));
    let event_id = execute_send_message_workflow(
        &state, community, channel.id, &owner_bytes, "emoji-rendering",
        "@engineering-lead :OCTOPUS: GitHub: #655 closed {{trigger.text}} :octopus: :unknown: :foreign-only:",
        ":chat-bubble: :espresso-cup: :binoculars:",
    ).await;
    let id = nostr::EventId::from_hex(&event_id).expect("message id");
    let stored = state
        .db
        .get_event_by_id_for_event_write(community, id.as_bytes())
        .await
        .expect("load workflow output")
        .expect("message persisted");
    let emoji_tags: Vec<Vec<String>> = stored
        .event
        .tags
        .iter()
        .filter(|tag| tag.as_slice()[0] == "emoji")
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    assert_eq!(
        emoji_tags,
        vec![
            vec!["emoji", "octopus", "https://cdn.example/octopus.webp"],
            vec!["emoji", "chat-bubble", "https://cdn.example/chat.webp"],
            vec!["emoji", "espresso-cup", "https://cdn.example/coffee.webp"],
            vec!["emoji", "binoculars", "https://cdn.example/binoculars.webp"],
        ]
    );
    assert!(stored.event.content.contains(":unknown: :foreign-only:"));
    stored
        .event
        .verify()
        .expect("emoji tags are covered by the event signature");
}
