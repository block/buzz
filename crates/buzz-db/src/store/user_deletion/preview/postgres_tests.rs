use super::*;
use crate::Db;
use nostr::ToBech32;
use sqlx::PgPool;

const HUMAN: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const OTHER: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
const RELAY: &str = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";

#[tokio::test]
#[ignore = "requires Postgres"]
async fn production_snapshot_rejects_writes_and_does_not_see_later_inserts() {
    let (pool, options) = setup().await;
    let mut tx = super::snapshot(&pool).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(options.community)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    event(&pool, options.community, 1, HUMAN, 9, json!([])).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(options.community)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let error = sqlx::query("UPDATE communities SET icon='unexpected write' WHERE id=$1")
        .bind(options.community)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("25006")
    );
    tx.rollback().await.unwrap();
}

async fn setup() -> (PgPool, PreviewOptions) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community)
        .bind(format!("{community}.preview.example.org"))
        .execute(&pool)
        .await
        .unwrap();
    (
        pool,
        PreviewOptions {
            community,
            pubkey: HUMAN.to_owned(),
            relay_pubkey: None,
            batch_size: 100,
            max_items: 10_000,
        },
    )
}

fn id(n: u32) -> String {
    format!("{n:064x}")
}

async fn event(pool: &PgPool, c: Uuid, n: u32, author: &str, kind: i32, tags: Value) {
    sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig)
        VALUES($1,decode($2,'hex'),decode($3,'hex'),date_trunc('month',now())+make_interval(secs=>$4),$5,$6,'fixture content',decode(repeat('00',64),'hex'))")
        .bind(c).bind(id(n)).bind(author).bind(f64::from(n)).bind(kind).bind(tags)
        .execute(pool).await.unwrap();
}

async fn preview(pool: &PgPool, options: &PreviewOptions) -> (PreviewSummary, Vec<Value>) {
    let mut bytes = Vec::new();
    let summary = Db::from_pool(pool.clone())
        .user_deletion_store()
        .preview_message_batches(options, &mut bytes)
        .await
        .unwrap();
    let lines = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (summary, lines)
}

fn items(lines: &[Value]) -> Vec<Value> {
    lines
        .iter()
        .filter_map(|line| line["items"].as_array())
        .flatten()
        .cloned()
        .collect()
}

async fn snapshot(pool: &PgPool) -> Vec<Value> {
    let mut values = Vec::new();
    for table in [
        "workflows",
        "subscriptions",
        "push_leases",
        "events",
        "reactions",
        "event_mentions",
        "workflow_runs",
        "workflow_approvals",
        "scheduled_workflow_fires",
        "delivery_log",
        "push_match_queue",
        "push_wake_outbox",
        "operator_listener_outbox",
        "thread_metadata",
        "entity_deletion_requests",
        "entity_deletion_stage_history",
        "entity_deletion_approvals",
    ] {
        let query = format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM {table} t");
        values.push(
            sqlx::query_scalar(sqlx::AssertSqlSafe(query))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    values
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mixed_batches_are_complete_ordered_deduplicated_and_read_only() {
    let (pool, options) = setup().await;
    let c = options.community;
    event(&pool, c, 1, HUMAN, 9, json!([])).await;
    event(&pool, c, 2, HUMAN, 40003, json!([["e", id(1)]])).await;
    event(&pool, c, 3, OTHER, 7, json!([["e", id(2)], ["p", OTHER]])).await;
    event(&pool, c, 4, OTHER, 9, json!([["e", id(1)]])).await;
    event(&pool, c, 5, HUMAN, 7, json!([["e", id(4)], ["p", OTHER]])).await;
    for (source, target, author) in [(3, 2, OTHER), (5, 4, HUMAN)] {
        sqlx::query("INSERT INTO reactions(community_id,event_created_at,event_id,pubkey,emoji,reaction_event_id)
            SELECT community_id,created_at,id,decode($3,'hex'),'👍',decode($4,'hex') FROM events WHERE community_id=$1 AND id=decode($2,'hex')")
            .bind(c).bind(id(target)).bind(author).bind(id(source)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,event_kind)
            SELECT community_id,$3,id,created_at,kind FROM events WHERE community_id=$1 AND id=decode($2,'hex')")
            .bind(c).bind(id(source)).bind(OTHER).execute(&pool).await.unwrap();
    }
    // Multiple side-table pages, including a mention on Bob's retained reply.
    sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,event_kind)
        SELECT e.community_id,lpad(to_hex(n),64,'0'),e.id,e.created_at,e.kind
        FROM events e CROSS JOIN generate_series(1,205) n WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(c).bind(id(1)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at)
        SELECT community_id,$3,id,created_at FROM events WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(c).bind(id(4)).bind(HUMAN).execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    let (summary, lines) = preview(&pool, &options).await;
    let operations = items(&lines);
    assert_eq!(summary.items, 213); // 4 events, 2 reactions, 207 mentions
    assert_eq!(summary.batches, 3);
    let sizes: Vec<_> = lines
        .iter()
        .filter_map(|v| v["items"].as_array().map(Vec::len))
        .collect();
    assert_eq!(sizes, [100, 100, 13]);
    let deleted_events: Vec<_> = operations
        .iter()
        .filter(|v| v["table"] == "events")
        .map(|v| v["key"]["id"].clone())
        .collect();
    assert_eq!(deleted_events, [id(3), id(2), id(1), id(5)]);
    let position = |table: &str, field: &str, value: &str| {
        operations
            .iter()
            .position(|v| v["table"] == table && v["key"][field] == value)
            .unwrap()
    };
    assert!(position("event_mentions", "event_id", &id(3)) < position("events", "id", &id(3)));
    assert!(!operations
        .iter()
        .any(|v| v["table"] == "event_mentions" && v["key"]["event_id"] == id(4)));
    assert_eq!(snapshot(&pool).await, before);
    let (_, again) = preview(&pool, &options).await;
    assert_eq!(again, lines);
    assert_eq!(lines.last().unwrap()["record"], "preview_complete");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sources_include_all_declared_kinds_and_history_but_preserve_other_authors_and_tenants() {
    let (pool, mut options) = setup().await;
    let (_, other_scope) = setup().await;
    for (n, kind) in KINDS.iter().enumerate() {
        event(
            &pool,
            options.community,
            n as u32 + 1,
            HUMAN,
            *kind,
            json!([]),
        )
        .await;
        event(
            &pool,
            other_scope.community,
            n as u32 + 1,
            HUMAN,
            *kind,
            json!([]),
        )
        .await;
    }
    event(
        &pool,
        options.community,
        80,
        HUMAN,
        30023,
        json!([["d", "same-article"]]),
    )
    .await;
    event(
        &pool,
        options.community,
        81,
        HUMAN,
        30023,
        json!([["d", "same-article"]]),
    )
    .await;
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(options.community)
        .bind(id(80))
        .execute(&pool)
        .await
        .unwrap();
    event(&pool, options.community, 90, HUMAN, 49001, json!([])).await;
    event(
        &pool,
        options.community,
        91,
        OTHER,
        9,
        json!([["actor", HUMAN], ["p", HUMAN]]),
    )
    .await;
    event(
        &pool,
        options.community,
        92,
        RELAY,
        9,
        json!([["actor", HUMAN]]),
    )
    .await;
    options.relay_pubkey = Some(RELAY.to_owned());
    options.pubkey = nostr::PublicKey::parse(HUMAN).unwrap().to_bech32().unwrap();
    let (summary, lines) = preview(&pool, &options).await;
    assert_eq!(summary.items, KINDS.len() + 3);
    assert_eq!(
        summary.excluded_event_kinds,
        [json!({"kind":49001,"count":1})]
    );
    assert!(items(&lines).iter().any(|v| v["key"]["id"] == id(80)));
    assert!(!items(&lines).iter().any(|v| v["key"]["id"] == id(91)));
    assert!(lines
        .iter()
        .filter(|v| v["record"] == "batch")
        .all(|v| v["community_id"] == options.community.to_string()));
    // Exercise every supported kind through execution too. Private formats
    // with nonempty ciphertext are separately tested as blocked sources.
    sqlx::query("UPDATE events SET content='' WHERE community_id=$1")
        .bind(options.community)
        .execute(&pool)
        .await
        .unwrap();
    let deleted = Db::from_pool(pool.clone())
        .user_deletion_store()
        .delete_message_batches(&options, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(deleted.items, summary.items);
    let survivors: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(options.community)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(survivors, 2); // unknown kind and other signer's forged attribution
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn authoritative_targets_and_channel_boundaries_control_recursion() {
    let (pool, options) = setup().await;
    let c = options.community;
    event(&pool, c, 1, HUMAN, 9, json!([])).await;
    event(&pool, c, 2, OTHER, 9, json!([])).await;
    event(&pool, c, 3, OTHER, 7, json!([["e", id(2)], ["e", id(1)]])).await; // last
    event(
        &pool,
        c,
        4,
        OTHER,
        40003,
        json!([["e", id(1)], ["e", id(2)]]),
    )
    .await; // first
    event(
        &pool,
        c,
        5,
        OTHER,
        45002,
        json!([["e", "bad"], ["e", id(1)]]),
    )
    .await;
    event(&pool, c, 6, OTHER, 7, json!([["e", id(1)], ["e", id(2)]])).await; // preserve
    event(
        &pool,
        c,
        7,
        OTHER,
        40003,
        json!([["e", id(2)], ["e", id(1)]]),
    )
    .await;
    event(&pool, c, 8, OTHER, 7, json!([["e", id(1)]])).await;
    sqlx::query("UPDATE events SET channel_id=$3 WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(c)
        .bind(id(8))
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();
    let (_, lines) = preview(&pool, &options).await;
    let selected: Vec<_> = items(&lines)
        .into_iter()
        .map(|v| v["key"]["id"].clone())
        .collect();
    assert_eq!(selected, [id(3), id(4), id(5), id(1)]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_children_orphans_and_warnings_are_printed_without_writes() {
    let (pool, options) = setup().await;
    let c = options.community;
    event(
        &pool,
        c,
        1,
        HUMAN,
        9,
        json!([["image", "https://example.org/photo"]]),
    )
    .await;
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,decode($2,'hex'))")
        .bind(c)
        .bind(OTHER)
        .execute(&pool)
        .await
        .unwrap();
    let workflow = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO workflows(community_id,id,name,owner_pubkey,definition,definition_hash)
        VALUES($1,$2,'preview',decode($3,'hex'),'{}','\\x00')",
    )
    .bind(c)
    .bind(workflow)
    .bind(OTHER)
    .execute(&pool)
    .await
    .unwrap();
    for (run, event_id, status) in [
        (Uuid::from_u128(1), id(1), "completed"),
        (Uuid::from_u128(2), id(999), "running"),
    ] {
        sqlx::query("INSERT INTO workflow_runs(community_id,id,workflow_id,status,trigger_event_id,trigger_context)
            VALUES($1,$2,$3,$4::run_status,decode($5,'hex'),$6)")
            .bind(c).bind(run).bind(workflow).bind(status).bind(event_id)
            .bind(json!({"author": HUMAN})).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO workflow_approvals(community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,expires_at)
        SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2,$3,'step',n,'owner',now()+interval '1 hour' FROM generate_series(1,205) n")
        .bind(c).bind(workflow).bind(Uuid::from_u128(1)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO scheduled_workflow_fires(community_id,workflow_id,scheduled_for,workflow_run_id) VALUES($1,$2,now(),$3)")
        .bind(c).bind(workflow).bind(Uuid::from_u128(1)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO reactions(community_id,event_created_at,event_id,pubkey,emoji) VALUES($1,now(),decode($2,'hex'),decode($3,'hex'),'x')")
        .bind(c).bind(id(999)).bind(HUMAN).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO push_match_queue(community_id,event_id,lease_until) VALUES($1,decode($2,'hex'),now()+interval '1 hour')")
        .bind(c).bind(id(1)).execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    let (summary, lines) = preview(&pool, &options).await;
    let operations = items(&lines);
    assert_eq!(summary.items, 211);
    assert_eq!(summary.warnings, 3);
    assert!(operations[..205]
        .iter()
        .all(|v| v["table"] == "workflow_approvals"));
    assert_eq!(operations[205]["action"], "would_clear_workflow_run_id");
    assert_eq!(operations[206]["table"], "workflow_runs");
    assert_eq!(operations.last().unwrap()["table"], "workflow_runs");
    assert_eq!(snapshot(&pool).await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cycles_limits_empty_scope_and_output_failures_never_report_false_completion() {
    let (pool, mut options) = setup().await;
    let (summary, _) = preview(&pool, &options).await;
    assert_eq!(summary.items, 0);
    event(&pool, options.community, 1, HUMAN, 7, json!([["e", id(2)]])).await;
    event(&pool, options.community, 2, OTHER, 7, json!([["e", id(1)]])).await;
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    let mut bytes = Vec::new();
    assert!(store
        .preview_message_batches(&options, &mut bytes)
        .await
        .unwrap_err()
        .to_string()
        .contains("cyclic"));
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("preview_complete"));
    sqlx::query("UPDATE events SET tags='[]' WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(options.community)
        .bind(id(1))
        .execute(&pool)
        .await
        .unwrap();
    options.max_items = 1;
    options.batch_size = 1;
    let mut bytes = Vec::new();
    assert!(store
        .preview_message_batches(&options, &mut bytes)
        .await
        .unwrap_err()
        .to_string()
        .contains("item limit"));
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("preview_complete"));
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(store
        .preview_message_batches(&options, &mut Broken)
        .await
        .is_err());
}

mod deletion_postgres_tests;
