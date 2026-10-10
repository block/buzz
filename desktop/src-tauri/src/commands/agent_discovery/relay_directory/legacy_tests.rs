//! Legacy VPS identities must be discovered by membership, not cosmetic role.
use super::*;
use axum::{
    routing::{get, post},
    Json, Router,
};
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn legacy_member_agents_are_discovered_and_revalidated_without_bot_roles() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let relay = Keys::generate();
    let viewer = Keys::generate();
    let viewer_key = viewer.public_key().to_hex();
    let relay_key = relay.public_key().to_hex();
    let agents = [Keys::generate(), Keys::generate(), Keys::generate()];
    let agent_keys: Vec<_> = agents.iter().map(|key| key.public_key().to_hex()).collect();
    let mut fixtures = Vec::new();
    for (index, agent) in agents.iter().enumerate() {
        let content = serde_json::json!({
            "name": format!("VPS {index}"), "agent_type": "acp", "status": "online",
            "channel_ids": [], "respond_to": "allowlist",
            "respond_to_allowlist": [&viewer_key],
        });
        fixtures.push(
            EventBuilder::new(Kind::Custom(10100), content.to_string())
                .sign_with_keys(agent)
                .unwrap(),
        );
    }
    let membership = |include_agents: bool, time: u64| {
        let mut tags = vec![
            Tag::parse(["d", "private-project"]).unwrap(),
            Tag::parse(["p", &viewer_key, "", "owner"]).unwrap(),
        ];
        if include_agents {
            for (key, role) in agent_keys.iter().zip(["owner", "member", "bot"]) {
                tags.push(Tag::parse(["p", key, "", role]).unwrap());
            }
        }
        EventBuilder::new(Kind::Custom(39002), "")
            .tags(tags)
            .custom_created_at(nostr::Timestamp::from(time))
            .sign_with_keys(&relay)
            .unwrap()
    };
    fixtures.push(membership(true, 10));
    let events = Arc::new(Mutex::new(fixtures));
    let queries = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let query_events = events.clone();
    let query_log = queries.clone();
    let router = Router::new()
        .route(
            "/",
            get(move || {
                let key = relay_key.clone();
                async move { Json(serde_json::json!({"self": key})) }
            }),
        )
        .route(
            "/query",
            post(move |Json(filters): Json<Vec<serde_json::Value>>| {
                let events = query_events.clone();
                let queries = query_log.clone();
                async move {
                    queries.lock().unwrap().extend(filters.clone());
                    let events = events.lock().unwrap();
                    let result: Vec<_> = events
                        .iter()
                        .filter(|event| {
                            filters.iter().any(|filter| {
                                filter["kinds"]
                                    .as_array()
                                    .unwrap()
                                    .contains(&serde_json::json!(event.kind.as_u16()))
                                    && filter.get("authors").is_none_or(|authors| {
                                        authors
                                            .as_array()
                                            .unwrap()
                                            .contains(&serde_json::json!(event.pubkey.to_hex()))
                                    })
                                    && ["d", "p"].iter().all(|tag| {
                                        filter.get(format!("#{tag}")).is_none_or(|values| {
                                            event.tags.iter().any(|t| {
                                                t.as_slice().first().map(String::as_str)
                                                    == Some(*tag)
                                                    && t.as_slice().get(1).is_some_and(|value| {
                                                        values
                                                            .as_array()
                                                            .unwrap()
                                                            .contains(&serde_json::json!(value))
                                                    })
                                            })
                                        })
                                    })
                            })
                        })
                        .cloned()
                        .collect();
                    Json(result)
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = viewer.clone();
    *state.relay_url_override.lock().unwrap() = Some(format!("ws://{address}"));

    let discovered = list_relay_agents_for_state(&state).await.unwrap();
    assert_eq!(
        discovered.len(),
        3,
        "all three roles must discover legacy agents"
    );
    for agent in &discovered {
        assert!(agent_keys.contains(&agent.pubkey));
        assert_eq!(agent.channel_ids, vec!["private-project".to_string()]);
        assert_eq!(
            agent.respond_to,
            Some(crate::managed_agents::RespondTo::Allowlist)
        );
        assert_eq!(agent.respond_to_allowlist, vec![viewer_key.clone()]);
    }
    let requested = agent_keys.iter().cloned().collect();
    let selected =
        list_relay_agents_for_selection(&state, Some(&requested), Some("private-project"))
            .await
            .unwrap();
    assert_eq!(
        selected.len(),
        3,
        "send revalidation must use the same membership evidence"
    );
    let outside = list_relay_agents_for_selection(&state, Some(&requested), Some("other-channel"))
        .await
        .unwrap();
    assert!(
        outside.is_empty(),
        "a directory entry is not membership in another channel"
    );
    events.lock().unwrap().push(membership(false, 20));
    let revoked =
        list_relay_agents_for_selection(&state, Some(&requested), Some("private-project"))
            .await
            .unwrap();
    assert!(
        revoked.is_empty(),
        "latest membership must revoke old discovery"
    );
    assert!(list_relay_agents_for_state(&state)
        .await
        .unwrap()
        .is_empty());
    server.abort();
    crate::relay_admission::reset_rate_limit_gate();
}
