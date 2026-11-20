use super::*;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn idle_delivery_worker_does_not_admit_empty_communities() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("isolated database URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO communities (id, host) \
         SELECT gen_random_uuid(), 'empty-' || n || '.example' FROM generate_series(1, 1000) n",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut state = crate::state::tests::test_state_with_database_pool(pool).await;
    Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().config)
        .unwrap()
        .push_enabled = true;

    for workers in [1, 4] {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        // The real worker, real database, and real backoff: idle checks at 0, 0.5,
        // and 1.5 seconds. Slower machines may complete fewer checks, never more.
        assert!(tokio::time::timeout(
            Duration::from_millis(1800),
            futures::future::join_all((0..workers).map(|_| run_delivery_worker(state.clone()))),
        )
        .await
        .is_err());

        let snapshot = snapshotter.snapshot().into_vec();
        let mut discoveries = 0;
        let mut admissions = 0;
        for (key, _, _, value) in snapshot {
            let DebugValue::Counter(count) = value else {
                continue;
            };
            if key.key().name() == "buzz_db_pool_acquire_attempts_total"
                && key
                    .key()
                    .labels()
                    .any(|label| label.key() == "operation" && label.value() == "maintenance")
            {
                discoveries += count;
            }
            if key.key().name() == "buzz_db_advisory_lock_acquisitions_total"
                && key
                    .key()
                    .labels()
                    .any(|label| label.key() == "lock_type" && label.value() == "deletion")
            {
                admissions += count;
            }
        }
        assert_eq!(
            admissions, 0,
            "idle communities must not open claim transactions"
        );
        assert!(
            (workers..=3 * workers).contains(&discoveries),
            "idle polling stays bounded: {discoveries}"
        );
        eprintln!("{workers} workers, 1,000 empty communities, 1.8s: {discoveries} discovery reads, {admissions} claim admissions");
    }
    state.begin_shutdown();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn delivery_worker_reaches_later_pages_before_revisiting_busy_communities() {
    use axum::{extract::State, routing::post, Json, Router};
    use std::{collections::HashMap, future::IntoFuture};
    use tokio::sync::mpsc;

    async fn accept(
        State(seen): State<mpsc::UnboundedSender<uuid::Uuid>>,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        seen.send(body["request_id"].as_str().unwrap().parse().unwrap())
            .unwrap();
        Json(serde_json::json!({"status": "accepted"}))
    }

    let pool = sqlx::PgPool::connect(&std::env::var("BUZZ_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    // 1,000 empty communities and 65 ready communities cross the discovery page.
    // The earliest ready community has more work than one claim batch can take.
    sqlx::raw_sql(
        r#"
        INSERT INTO communities (id, host)
        SELECT ('00000000-0000-0000-0000-' || lpad(n::text, 12, '0'))::uuid,
               'delivery-' || n || '.example' FROM generate_series(1, 1065) n;
        INSERT INTO push_leases (community_id, author, installation_id, source_event_id,
            source_created_at, generation, active, endpoint_hash, endpoint_grant,
            max_class, subscriptions, expires_at)
        SELECT id, decode(repeat('21',32),'hex'), 'install', decode(repeat('22',32),'hex'),
               1, 1, true, decode(repeat('23',32),'hex'), 'opaque-grant', 'default', '[]',
               4102444800 FROM communities WHERE host IN
               (SELECT 'delivery-' || n || '.example' FROM generate_series(1,65) n);
        INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig)
        SELECT l.community_id, decode(lpad(to_hex(n),64,'0'),'hex'), l.author,
               now(), 9, '[]', '', decode(repeat('24',64),'hex')
        FROM push_leases l CROSS JOIN generate_series(1,17) n
        WHERE n = 1 OR l.community_id = '00000000-0000-0000-0000-000000000001';
        INSERT INTO push_wake_outbox (community_id, author, installation_id, lease_generation,
            endpoint_hash, event_id, class, expires_at)
        SELECT l.community_id, l.author, l.installation_id, l.generation,
               l.endpoint_hash, e.id, 'default', l.expires_at
        FROM push_leases l JOIN events e ON e.community_id = l.community_id;
    "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let jobs: HashMap<uuid::Uuid, uuid::Uuid> = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        "SELECT id, community_id FROM push_wake_outbox",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let (seen, mut received) = mpsc::unbounded_channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/deliver", post(accept))
                .with_state(seen),
        )
        .into_future(),
    );
    let mut state = crate::state::tests::test_state_with_database_pool(pool).await;
    let config = Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().config).unwrap();
    config.push_enabled = true;
    config.push_gateway_delivery_url = Some(format!("http://{address}/deliver").parse().unwrap());
    let started = Instant::now();
    let worker = run_delivery_worker(state.clone());
    let observe = async {
        let mut order = Vec::new();
        for _ in 0..81 {
            let id = received.recv().await.unwrap();
            order.push(jobs[&id]);
        }
        order
    };
    let order = tokio::select! {
        _ = worker => panic!("worker exited early"),
        result = tokio::time::timeout(Duration::from_secs(20), observe) => result.expect("deliver every community"),
    };
    state.begin_shutdown();
    server.abort();
    let hot: uuid::Uuid = "00000000-0000-0000-0000-000000000001".parse().unwrap();
    let last: uuid::Uuid = "00000000-0000-0000-0000-000000000065".parse().unwrap();
    assert_eq!(order[..16], [hot; 16]);
    assert_eq!(
        order[79], last,
        "later discovery page progresses in the first sweep"
    );
    assert_eq!(
        order[80], hot,
        "busy community is revisited only after the full sweep"
    );
    eprintln!(
        "81 deliveries across 65 ready / 1,065 total communities: {:?}",
        started.elapsed()
    );
}

/// A mention wake for someone outside a listed channel reaches the gateway
/// only while they pass the connection gate and the channel stays listed.
/// Mutations: drop `channel_authorizes_wake` from `deliver_one`, swap it for
/// `is_member`, drop its admission check, or ignore any admission setting → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn nonmember_mention_wakes_reach_the_gateway_only_for_admitted_readers() {
    use crate::handlers::group_state_postgres_tests::{create_channel, http_auth};
    use crate::handlers::ingest::ingest_event;
    use axum::{extract::State, routing::post, Json, Router};
    use nostr::{EventBuilder, Kind, Tag};
    use std::{collections::HashMap, future::IntoFuture};
    use tokio::sync::mpsc;

    async fn accept(
        State(seen): State<mpsc::UnboundedSender<uuid::Uuid>>,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        seen.send(body["request_id"].as_str().unwrap().parse().unwrap())
            .unwrap();
        Json(serde_json::json!({"status": "accepted"}))
    }

    let (seen, mut received) = mpsc::unbounded_channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/deliver", post(accept))
                .with_state(seen),
        )
        .into_future(),
    );
    let mut state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let config = Arc::make_mut(&mut Arc::get_mut(&mut state).expect("unshared state").config);
    config.push_gateway_delivery_url = Some(format!("http://{address}/deliver").parse().unwrap());
    config.require_relay_membership = true;
    config.pubkey_allowlist_enabled = true;
    config.allow_nip_oa_auth = false;

    let host = format!("push-gate-{}.local", uuid::Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    let tenant = buzz_core::TenantContext::resolved(community, &host);
    let owner = nostr::Keys::generate();
    let [admitted, banned, removed, not_allowlisted, agent] =
        std::array::from_fn(|_| nostr::Keys::generate());
    for (keys, allowlisted) in [
        (&owner, true),
        (&admitted, true),
        (&banned, true),
        (&removed, true),
        (&not_allowlisted, false),
    ] {
        assert!(
            state
                .db
                .add_relay_member(
                    community,
                    &keys.public_key().to_hex(),
                    "member",
                    None,
                    &state.relay_keypair,
                )
                .await
                .expect("add relay member")
                .outcome
        );
        if allowlisted {
            sqlx::query("INSERT INTO pubkey_allowlist (community_id, pubkey) VALUES ($1, $2)")
                .bind(community.as_uuid())
                .bind(keys.public_key().to_bytes().as_slice())
                .execute(state.db.pool())
                .await
                .expect("allowlist");
        }
    }
    // An allowlisted agent whose owner is a member, while owner membership is off.
    sqlx::query("INSERT INTO pubkey_allowlist (community_id, pubkey) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(agent.public_key().to_bytes().as_slice())
        .execute(state.db.pool())
        .await
        .expect("allowlist agent");
    sqlx::query(
        "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) \
         VALUES ($1, $3, NULL), ($1, $2, $3) \
         ON CONFLICT (community_id, pubkey) DO UPDATE \
         SET agent_owner_pubkey = EXCLUDED.agent_owner_pubkey",
    )
    .bind(community.as_uuid())
    .bind(agent.public_key().to_bytes().as_slice())
    .bind(owner.public_key().to_bytes().as_slice())
    .execute(state.db.pool())
    .await
    .expect("record agent owner");
    state
        .db
        .ban_community_member(
            community,
            &banned.public_key().to_bytes(),
            &owner.public_key().to_bytes(),
            None,
            None,
        )
        .await
        .expect("ban");
    assert!(matches!(
        state
            .db
            .remove_relay_member(
                community,
                &removed.public_key().to_hex(),
                &state.relay_keypair,
            )
            .await
            .expect("remove from community")
            .outcome,
        buzz_db::relay_members::RemoveResult::Removed
    ));

    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let outsiders = [&admitted, &banned, &removed, &not_allowlisted, &agent];
    for (source, keys) in (1u8..).zip(outsiders) {
        assert_eq!(
            buzz_db::push::replace_active_lease(
                state.db.pool(),
                community,
                &keys.public_key().to_bytes(),
                "phone",
                buzz_db::push::LeaseVersion {
                    source_event_id: &[source; 32],
                    source_created_at: 10,
                    generation: 1,
                    expires_at: i64::MAX / 2,
                },
                // The outbox is unique per endpoint and event, so each lease gets its own.
                buzz_db::push::ActiveLease {
                    endpoint_hash: &keys.public_key().to_bytes(),
                    endpoint_grant: "opaque-grant",
                    max_class: "default",
                    subscriptions: &serde_json::json!([]),
                },
            )
            .await
            .expect("activate lease"),
            buzz_db::push::ReplaceLeaseOutcome::Accepted
        );
    }

    // One mention of every outsider, queued for each as the matcher would.
    let deliver = |content: &'static str, authors: Vec<&nostr::Keys>| {
        let state = state.clone();
        let authors: Vec<Vec<u8>> = authors
            .into_iter()
            .map(|keys| keys.public_key().to_bytes().to_vec())
            .collect();
        let tenant = tenant.clone();
        let owner = owner.clone();
        let mentioned: Vec<nostr::PublicKey> =
            outsiders.iter().map(|keys| keys.public_key()).collect();
        async move {
            let mut event = EventBuilder::new(Kind::Custom(9), content)
                .tag(Tag::parse(["h", &channel.to_string()]).unwrap());
            for key in mentioned {
                event = event.tag(Tag::public_key(key));
            }
            let event = event.sign_with_keys(&owner).unwrap();
            let ingested = ingest_event(&state, &tenant, event.clone(), http_auth(&owner))
                .await
                .expect("ingest mention");
            assert!(ingested.accepted, "{}", ingested.message);
            let mut queued = HashMap::new();
            for author in authors {
                let id: uuid::Uuid = sqlx::query_scalar(
                    "INSERT INTO push_wake_outbox (community_id, author, installation_id, \
                         lease_generation, endpoint_hash, event_id, class, expires_at) \
                     VALUES ($1, $2, 'phone', 1, $3, $4, 'default', $5) RETURNING id",
                )
                .bind(community.as_uuid())
                .bind(&author)
                .bind(&author)
                .bind(event.id.as_bytes().as_slice())
                .bind(i64::MAX / 2)
                .fetch_one(state.db.pool())
                .await
                .expect("queue wake");
                queued.insert(id, author);
            }
            let claimed = state
                .db
                .claim_due_push_wakes(community, 10, Utc::now() + TimeDelta::seconds(60))
                .await
                .expect("claim wakes");
            assert_eq!(claimed.len(), queued.len(), "every queued wake is claimed");
            let http = reqwest::Client::new();
            for wake in claimed {
                deliver_one(&state, &http, wake).await;
            }
            queued
        }
    };

    let queued = deliver("hello", outsiders.to_vec()).await;
    let mut sent = Vec::new();
    while let Ok(id) = received.try_recv() {
        sent.push(queued[&id].clone());
    }
    assert_eq!(
        sent,
        vec![admitted.public_key().to_bytes().to_vec()],
        "only the outsider who still passes the connection gate is woken"
    );

    let flip = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["visibility", "private"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let flipped = ingest_event(&state, &tenant, flip, http_auth(&owner))
        .await
        .expect("make private");
    assert!(flipped.accepted, "{}", flipped.message);
    deliver("hello again", vec![&admitted]).await;
    assert!(
        received.try_recv().is_err(),
        "a channel that turned private sends nothing"
    );

    state.begin_shutdown();
    server.abort();
}
