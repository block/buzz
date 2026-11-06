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
