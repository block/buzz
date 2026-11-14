use super::*;
use crate::{user_deletion::ExecutionMode, CommunityId, Db};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp, ToBech32};

fn records(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn snapshot_rows(pool: &sqlx::PgPool, community: CommunityId, table: &str) -> Vec<Value> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(t) FROM {table} t WHERE community_id=$1 ORDER BY to_jsonb(t)"
    )))
    .bind(community.as_uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owned_agents_share_bounded_batches_and_preserve_other_owners_and_communities() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let a = db
        .ensure_configured_community("agent-content-a.example")
        .await
        .unwrap()
        .id;
    let b = db
        .ensure_configured_community("agent-content-b.example")
        .await
        .unwrap()
        .id;
    let alice = Keys::generate();
    let agent = Keys::generate();
    let remote_agent = Keys::generate();
    let bob = Keys::generate();
    let bob_agent = Keys::generate();
    let nested_agent = Keys::generate();
    let relay = Keys::generate();
    let authors = [
        &alice,
        &agent,
        &remote_agent,
        &bob,
        &bob_agent,
        &nested_agent,
    ];
    for community in [a, b] {
        for author in authors {
            db.ensure_user(community, author.public_key().as_bytes())
                .await
                .unwrap();
        }
        for (child, owner) in [
            (&agent, &alice),
            (&bob_agent, &bob),
            (&nested_agent, &agent),
        ] {
            sqlx::query(
                "UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3",
            )
            .bind(community.as_uuid())
            .bind(owner.public_key().as_bytes().as_slice())
            .bind(child.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    // Ownership recorded only in B still discovers this key, like media cleanup.
    sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
        .bind(b.as_uuid())
        .bind(alice.public_key().as_bytes().as_slice())
        .bind(remote_agent.public_key().as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let timestamp = Timestamp::now();
    let mut selected = HashSet::new();
    let mut first_ids = Vec::new();
    for (index, author) in authors.into_iter().enumerate() {
        // More than one SQL page, with all authors sharing a timestamp.
        for n in 0..105 {
            let event = EventBuilder::new(Kind::Custom(9), format!("author {index}, row {n}"))
                .custom_created_at(timestamp)
                .sign_with_keys(author)
                .unwrap();
            for community in [a, b] {
                db.insert_event(community, &event, None).await.unwrap();
            }
            if index < 3 {
                selected.insert(event.id.to_hex());
            }
            if n == 0 {
                first_ids.push(event.id.to_bytes().to_vec());
            }
        }
        let event = EventBuilder::new(Kind::Metadata, "{}")
            .custom_created_at(timestamp)
            .sign_with_keys(author)
            .unwrap();
        db.insert_event(a, &event, None).await.unwrap();
    }
    // Actor wins over p; p is a fallback only for the trusted relay signer.
    for (signer, kind, tags, keep) in [
        (&relay, 9, vec![("actor", &agent), ("p", &bob)], false),
        (&relay, 9, vec![("p", &remote_agent)], false),
        (&relay, 9, vec![("actor", &bob), ("p", &alice)], true),
        (&bob, 9, vec![("actor", &agent), ("p", &alice)], true),
        (&relay, 30174, vec![("actor", &agent)], true),
    ] {
        let event = EventBuilder::new(Kind::Custom(kind), format!("relay case {kind} {keep}"))
            .tags(
                tags.into_iter()
                    .map(|(tag, key)| Tag::parse([tag, &key.public_key().to_hex()]).unwrap()),
            )
            .custom_created_at(timestamp)
            .sign_with_keys(signer)
            .unwrap();
        db.insert_event(a, &event, None).await.unwrap();
        if !keep {
            selected.insert(event.id.to_hex());
        }
    }
    // Every owner has all cleanup row types in both communities. Agent resources
    // reference Bob's event to prove ownership alone selects them; Bob's stay.
    for community in [a, b] {
        for (index, author) in authors.into_iter().enumerate() {
            let key = author.public_key().to_bytes();
            let id = &first_ids[index];
            let sub = format!("sub-{index}");
            sqlx::query(
                "INSERT INTO subscriptions (community_id,id,owner_pubkey) VALUES ($1,$2,$3)",
            )
            .bind(community.as_uuid())
            .bind(&sub)
            .bind(key.as_slice())
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO delivery_log (community_id,subscription_id,event_id) VALUES ($1,$2,$3)")
                .bind(community.as_uuid()).bind(&sub).bind(&first_ids[3]).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO reactions (community_id,event_created_at,event_id,pubkey,emoji) VALUES ($1,to_timestamp($2),$3,$4,'+')")
                .bind(community.as_uuid()).bind(timestamp.as_secs() as f64).bind(&first_ids[3])
                .bind(key.as_slice()).execute(&pool).await.unwrap();
            sqlx::query(
                "INSERT INTO event_mentions (community_id,event_id,pubkey_hex,event_created_at) VALUES ($1,$2,$3,now())",
            )
            .bind(community.as_uuid())
            .bind(id)
            .bind(bob.public_key().to_hex())
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO push_match_queue (community_id,event_id) VALUES ($1,$2)")
                .bind(community.as_uuid())
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO push_leases (community_id,author,installation_id,source_event_id,source_created_at,generation,active,expires_at) VALUES ($1,$2,'test',$3,$4,1,false,0)")
                .bind(community.as_uuid()).bind(key.as_slice()).bind(id).bind(timestamp.as_secs() as i64)
                .execute(&pool).await.unwrap();
            for event_id in [id, &first_ids[3]] {
                sqlx::query("INSERT INTO push_wake_outbox (community_id,author,installation_id,lease_generation,endpoint_hash,event_id,class,expires_at,state) VALUES ($1,$2,'test',1,$2,$3,'silent',0,'delivered') ON CONFLICT DO NOTHING")
                    .bind(community.as_uuid()).bind(key.as_slice()).bind(event_id).execute(&pool).await.unwrap();
                // Both target and listener ownership select; duplicate paths must not double count.
                for (listener, target) in [
                    (key.as_slice(), bob.public_key().as_bytes().as_slice()),
                    (bob.public_key().as_bytes().as_slice(), key.as_slice()),
                ] {
                    sqlx::query("INSERT INTO operator_listener_outbox (community_id,listener_pubkey,target_pubkey,event_id,event_kind,event_created_at) VALUES ($1,$2,$3,$4,9,to_timestamp($5)) ON CONFLICT DO NOTHING")
                        .bind(community.as_uuid()).bind(listener).bind(target).bind(event_id).bind(timestamp.as_secs() as f64)
                        .execute(&pool).await.unwrap();
                }
            }
        }
    }
    let tables = [
        "events",
        "event_mentions",
        "reactions",
        "subscriptions",
        "delivery_log",
        "push_leases",
        "push_match_queue",
        "push_wake_outbox",
        "operator_listener_outbox",
        "users",
    ];
    let mut before_a = Vec::new();
    let mut before_b = Vec::new();
    for table in tables {
        before_a.push(snapshot_rows(&pool, a, table).await);
        before_b.push(snapshot_rows(&pool, b, table).await);
    }
    let mut options = ContentBatchOptions {
        community: *a.as_uuid(),
        pubkey: alice.public_key().to_bech32().unwrap(),
        relay_pubkey: Some(relay.public_key().to_hex()),
        batch_size: 100,
        max_items: 1000,
    };
    let store = db.user_deletion_store();
    let mut output = Vec::new();
    let summary = store
        .run_message_batches(&options, ExecutionMode::ReadOnly, None, &mut output)
        .await
        .unwrap();
    let records = records(&output);
    let mut targets = vec![
        alice.public_key().to_hex(),
        agent.public_key().to_hex(),
        remote_agent.public_key().to_hex(),
    ];
    targets.sort();
    assert_eq!(records[0]["target_pubkeys"], json!(targets));
    assert_eq!(
        summary.excluded_event_kinds,
        vec![json!({"kind":0,"count":3})]
    );
    let items: Vec<_> = records
        .iter()
        .filter_map(|r| r["items"].as_array())
        .flatten()
        .collect();
    assert_eq!(items.len(), summary.items);
    assert_eq!(
        items
            .iter()
            .map(|i| format!("{}:{}", i["table"], i["key"]))
            .collect::<HashSet<_>>()
            .len(),
        items.len()
    );
    assert_eq!(
        items
            .iter()
            .filter(|i| i["table"] == "events")
            .map(|i| i["key"]["id"].as_str().unwrap().to_owned())
            .collect::<HashSet<_>>(),
        selected
    );
    for batch in records.iter().filter(|r| r["record"] == "batch") {
        assert!(batch["items"].as_array().unwrap().len() <= 100);
    }
    for (index, table) in tables.into_iter().enumerate() {
        assert_eq!(
            snapshot_rows(&pool, a, table).await,
            before_a[index],
            "preview changed {table}"
        );
        assert_eq!(snapshot_rows(&pool, b, table).await, before_b[index]);
    }
    // Omitting the relay signer leaves the two attributed messages and their
    // two mention rows outside the selection.
    options.relay_pubkey = None;
    let no_relay = store
        .run_message_batches(&options, ExecutionMode::ReadOnly, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(no_relay.items + 4, summary.items);
    options.relay_pubkey = Some(agent.public_key().to_hex());
    assert!(store
        .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .is_err());
    options.relay_pubkey = Some(relay.public_key().to_hex());
    // Reject the whole plan rather than delete a prefix when the item cap is exceeded.
    options.max_items = 1;
    assert!(store
        .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .is_err());
    for (index, table) in tables.into_iter().enumerate() {
        assert_eq!(snapshot_rows(&pool, a, table).await, before_a[index]);
    }
    options.max_items = 1000;
    let result = store
        .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(result.items, summary.items);
    for (index, table) in tables.into_iter().enumerate() {
        assert_eq!(
            snapshot_rows(&pool, b, table).await,
            before_b[index],
            "other community: {table}"
        );
        let after = snapshot_rows(&pool, a, table).await;
        let expected: Vec<_> = before_a[index]
            .iter()
            .filter(|row| {
                let hex_field = |field: &str| {
                    row[field]
                        .as_str()
                        .unwrap_or("")
                        .trim_start_matches("\\x")
                        .to_owned()
                };
                let owned = |field: &str| targets.contains(&hex_field(field));
                match table {
                    "events" => !selected.contains(&hex_field("id")),
                    "users" => true,
                    "event_mentions" | "push_match_queue" => {
                        !selected.contains(&hex_field("event_id"))
                    }
                    "reactions" => !owned("pubkey"),
                    "subscriptions" => !owned("owner_pubkey"),
                    "delivery_log" => !["sub-0", "sub-1", "sub-2"]
                        .contains(&row["subscription_id"].as_str().unwrap()),
                    "push_leases" | "push_wake_outbox" => !owned("author"),
                    "operator_listener_outbox" => {
                        !owned("target_pubkey") && !owned("listener_pubkey")
                    }
                    _ => unreachable!(),
                }
            })
            .cloned()
            .collect();
        assert_eq!(after, expected, "surviving payloads: {table}");
    }
    assert_eq!(
        store
            .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn target_discovery_handles_no_agents_and_rejects_overflow_without_output() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community("agent-content-limit.example")
        .await
        .unwrap()
        .id;
    let alice = Keys::generate();
    db.ensure_user(community, alice.public_key().as_bytes())
        .await
        .unwrap();
    let options = ContentBatchOptions {
        community: *community.as_uuid(),
        pubkey: alice.public_key().to_hex(),
        relay_pubkey: None,
        batch_size: 100,
        max_items: 1000,
    };
    let store = db.user_deletion_store();
    let mut output = Vec::new();
    store
        .run_message_batches(&options, ExecutionMode::ReadOnly, None, &mut output)
        .await
        .unwrap();
    assert_eq!(
        records(&output)[0]["target_pubkeys"],
        json!([alice.public_key().to_hex()])
    );
    sqlx::query(
        "INSERT INTO users (community_id,pubkey,agent_owner_pubkey)
        SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2 FROM generate_series(1,10000) n",
    )
    .bind(community.as_uuid())
    .bind(alice.public_key().as_bytes().as_slice())
    .execute(&pool)
    .await
    .unwrap();
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let mut output = Vec::new();
        let error = store
            .run_message_batches(&options, mode, None, &mut output)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("target key limit"));
        assert!(output.is_empty());
    }
    assert_eq!(snapshot_rows(&pool, community, "users").await.len(), 10001);
}
