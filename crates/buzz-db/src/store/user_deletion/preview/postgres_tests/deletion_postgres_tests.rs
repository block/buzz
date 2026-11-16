use super::*;

async fn delete(pool: &PgPool, options: &PreviewOptions) -> (PreviewSummary, Vec<Value>) {
    let mut bytes = Vec::new();
    let summary = Db::from_pool(pool.clone())
        .user_deletion_store()
        .run_message_batches(
            options,
            crate::user_deletion::ExecutionMode::Delete,
            &mut bytes,
        )
        .await
        .unwrap();
    let lines = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    (summary, lines)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deletion_matches_printed_batches_preserves_replies_and_other_tenants() {
    let (pool, options) = setup().await;
    let (_, other) = setup().await;
    let c = options.community;
    event(&pool, c, 1, HUMAN, 9, json!([])).await;
    event(&pool, c, 2, OTHER, 40003, json!([["e", id(1)]])).await;
    event(&pool, c, 3, OTHER, 7, json!([["e", id(2)]])).await;
    event(&pool, c, 4, OTHER, 9, json!([["e", id(1)]])).await;
    event(&pool, other.community, 1, HUMAN, 9, json!([])).await;
    event(&pool, c, 5, HUMAN, 46030, json!([])).await;
    event(&pool, c, 6, HUMAN, 40100, json!([])).await;
    sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,event_kind)
        SELECT e.community_id,lpad(to_hex(n),64,'0'),e.id,e.created_at,e.kind FROM events e CROSS JOIN generate_series(1,205) n
        WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(c).bind(id(1)).execute(&pool).await.unwrap();
    let channel = Uuid::new_v4();
    sqlx::query("INSERT INTO channels(community_id,id,name,created_by) VALUES($1,$2,'fixture',decode('79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798','hex'))")
        .bind(c)
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE events SET channel_id=$2 WHERE community_id=$1")
        .bind(c)
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO thread_metadata(community_id,event_created_at,event_id,channel_id,parent_event_id,root_event_id,depth,reply_count,descendant_count)
        SELECT community_id,created_at,id,channel_id,CASE WHEN id=decode($2,'hex') THEN NULL ELSE decode($2,'hex') END,
        CASE WHEN id=decode($2,'hex') THEN NULL ELSE decode($2,'hex') END,CASE WHEN id=decode($2,'hex') THEN 0 ELSE 1 END,99,99
        FROM events WHERE community_id=$1 AND id IN (decode($2,'hex'),decode($3,'hex'))")
        .bind(c).bind(id(1)).bind(id(4)).execute(&pool).await.unwrap();
    let mut proposed = Vec::new();
    let expected = Db::from_pool(pool.clone())
        .user_deletion_store()
        .run_message_batches(
            &options,
            crate::user_deletion::ExecutionMode::ReadOnly,
            &mut proposed,
        )
        .await
        .unwrap();
    let printed: Vec<Value> = String::from_utf8(proposed)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(c)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 6, "read-only mode must preserve source events");
    let (summary, executed) = delete(&pool, &options).await;
    assert_eq!(items(&printed), items(&executed));
    assert_eq!(summary.items, expected.items);
    assert_eq!(summary.record, "delete_complete");
    assert_eq!(
        executed
            .iter()
            .filter(|v| v["record"] == "batch_committed")
            .count(),
        3
    );
    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT encode(id,'hex') FROM events WHERE community_id=$1 ORDER BY id")
            .bind(c)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, [id(4), id(6)]);
    let other_count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(other.community)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(other_count, 1);
    let counts: (i32,i32)=sqlx::query_as("SELECT reply_count,descendant_count FROM thread_metadata WHERE community_id=$1 AND event_id=decode($2,'hex')")
        .bind(c).bind(id(1)).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1)); // contentless ancestry remains reachable
    assert_eq!(delete(&pool, &options).await.0.items, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn interrupted_output_keeps_committed_progress_and_releases_session_locks() {
    let (pool, mut options) = setup().await;
    options.batch_size = 1;
    event(&pool, options.community, 1, HUMAN, 9, json!([])).await;
    event(&pool, options.community, 2, OTHER, 7, json!([["e", id(1)]])).await;
    struct Stop {
        bytes: Vec<u8>,
        lines: usize,
    }
    impl Write for Stop {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.lines >= 3 {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.bytes.extend_from_slice(bytes);
            self.lines += bytes.iter().filter(|&&b| b == b'\n').count();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Stop {
        bytes: Vec::new(),
        lines: 0,
    };
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    assert!(store
        .delete_message_batches(&options, &mut output)
        .await
        .is_err());
    assert!(!String::from_utf8(output.bytes)
        .unwrap()
        .contains("delete_complete"));
    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT encode(id,'hex') FROM events WHERE community_id=$1 ORDER BY id")
            .bind(options.community)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, [id(1)]); // child committed, source retained for restart
    assert_eq!(delete(&pool, &options).await.0.items, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unsafe_sources_or_limits_reject_before_any_deletion() {
    let (pool, mut options) = setup().await;
    let c = options.community;
    event(&pool, c, 1, HUMAN, 9, json!([])).await;
    event(
        &pool,
        c,
        2,
        HUMAN,
        9,
        json!([["image", "https://example.org/photo"]]),
    )
    .await;
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    for limit in [1, 10_000] {
        options.max_items = limit;
        let before = snapshot(&pool).await;
        let mut bytes = Vec::new();
        assert!(store
            .delete_message_batches(&options, &mut bytes)
            .await
            .is_err());
        assert_eq!(snapshot(&pool).await, before);
        assert!(!String::from_utf8(bytes).unwrap().lines().any(
            |line| serde_json::from_str::<Value>(line).unwrap()["record"] == "batch_committed"
        ));
    }
    sqlx::query(
        "UPDATE events SET tags='[]',kind=30078 WHERE community_id=$1 AND id=decode($2,'hex')",
    )
    .bind(c)
    .bind(id(2))
    .execute(&pool)
    .await
    .unwrap();
    let before = snapshot(&pool).await;
    let error = store
        .delete_message_batches(&options, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("media evidence"));
    assert_eq!(snapshot(&pool).await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owned_workflows_subscriptions_and_delivery_share_one_ordered_plan() {
    let (pool, options) = setup().await;
    let c = options.community;
    event(&pool, c, 1, HUMAN, 9, json!([])).await;
    event(&pool, c, 2, OTHER, 9, json!([])).await;
    for author in [HUMAN, OTHER] {
        sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,decode($2,'hex'))")
            .bind(c)
            .bind(author)
            .execute(&pool)
            .await
            .unwrap();
    }
    let owned = Uuid::from_u128(10);
    let retained = Uuid::from_u128(20);
    for (workflow, author) in [(owned, HUMAN), (retained, OTHER)] {
        sqlx::query("INSERT INTO workflows(community_id,id,name,owner_pubkey,definition,definition_hash) VALUES($1,$2,'fixture',decode($3,'hex'),'{}','\\x00')")
            .bind(c).bind(workflow).bind(author).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO subscriptions(community_id,id,owner_pubkey) VALUES($1,$2,decode($3,'hex'))")
            .bind(c).bind(workflow.to_string()).bind(author).execute(&pool).await.unwrap();
    }
    for (run, workflow, author, source) in [
        (1, owned, HUMAN, 1),
        (2, retained, HUMAN, 1),
        (3, retained, OTHER, 2),
    ] {
        sqlx::query("INSERT INTO workflow_runs(community_id,id,workflow_id,status,trigger_context,trigger_event_id) VALUES($1,$2,$3,'completed',$4,decode($5,'hex'))")
            .bind(c).bind(Uuid::from_u128(run)).bind(workflow).bind(json!({"author":author})).bind(id(source)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workflow_approvals(community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,expires_at) VALUES($1,decode($2,'hex'),$3,$4,'step',0,'owner',now())")
            .bind(c).bind(id(run as u32)).bind(workflow).bind(Uuid::from_u128(run)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO scheduled_workflow_fires(community_id,workflow_id,scheduled_for,workflow_run_id) VALUES($1,$2,now()+make_interval(secs=>$3),$4)")
            .bind(c).bind(workflow).bind(run as f64).bind(Uuid::from_u128(run)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO delivery_log(community_id,subscription_id,event_id) VALUES($1,$2,decode($3,'hex'))")
            .bind(c).bind(workflow.to_string()).bind(id(source)).execute(&pool).await.unwrap();
    }
    for (author, n) in [(HUMAN, 1), (OTHER, 2)] {
        sqlx::query("INSERT INTO push_leases(community_id,author,installation_id,source_event_id,source_created_at,generation,active,expires_at) VALUES($1,decode($2,'hex'),'device',decode($3,'hex'),1,1,false,9999999999)")
            .bind(c).bind(author).bind(id(n+50)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO push_wake_outbox(community_id,author,installation_id,lease_generation,endpoint_hash,event_id,class,expires_at) VALUES($1,decode($2,'hex'),'device',1,decode($3,'hex'),decode($4,'hex'),'default',9999999999)")
            .bind(c).bind(author).bind(id(n+100)).bind(id(n)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO operator_listener_outbox(community_id,listener_pubkey,target_pubkey,event_id,event_kind,event_created_at) VALUES($1,decode($2,'hex'),decode($2,'hex'),decode($3,'hex'),9,now())")
            .bind(c).bind(author).bind(id(n)).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO push_match_queue(community_id,event_id) VALUES($1,decode($2,'hex'))")
        .bind(c)
        .bind(id(1))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO reactions(community_id,event_created_at,event_id,pubkey,emoji) VALUES($1,now(),decode($2,'hex'),decode($3,'hex'),'x')")
        .bind(c).bind(id(999)).bind(HUMAN).execute(&pool).await.unwrap();
    // Refuse an active run before deleting its approvals, schedules or parent.
    sqlx::query("UPDATE workflow_runs SET status='running' WHERE community_id=$1 AND id=$2")
        .bind(c)
        .bind(Uuid::from_u128(1))
        .execute(&pool)
        .await
        .unwrap();
    let before = snapshot(&pool).await;
    assert!(Db::from_pool(pool.clone())
        .user_deletion_store()
        .delete_message_batches(&options, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(snapshot(&pool).await, before);
    sqlx::query("UPDATE workflow_runs SET status='completed' WHERE community_id=$1 AND id=$2")
        .bind(c)
        .bind(Uuid::from_u128(1))
        .execute(&pool)
        .await
        .unwrap();
    // A blocked payload near the end of the plan must protect its earlier
    // scheduled-fire and approval rows too.
    sqlx::query("UPDATE workflows SET definition='{\"url\":\"https://example.org/file\"}' WHERE community_id=$1 AND id=$2")
        .bind(c).bind(owned).execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    let error = Db::from_pool(pool.clone())
        .user_deletion_store()
        .delete_message_batches(&options, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("media evidence"));
    assert_eq!(snapshot(&pool).await, before);
    sqlx::query("UPDATE workflows SET definition='{}' WHERE community_id=$1 AND id=$2")
        .bind(c)
        .bind(owned)
        .execute(&pool)
        .await
        .unwrap();
    let (_, printed) = preview(&pool, &options).await;
    let (_, executed) = delete(&pool, &options).await;
    assert_eq!(items(&printed), items(&executed));
    let operations = items(&executed);
    let unique: HashSet<_> = operations
        .iter()
        .map(|v| format!("{}:{}", v["table"], v["key"]))
        .collect();
    assert_eq!(operations.len(), unique.len());
    for table in [
        "workflows",
        "workflow_runs",
        "workflow_approvals",
        "subscriptions",
        "delivery_log",
        "push_leases",
        "push_wake_outbox",
        "operator_listener_outbox",
    ] {
        let query = format!("SELECT count(*) FROM {table} WHERE community_id=$1");
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(c)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "{table}");
    }
    let pointers:Vec<Option<Uuid>>=sqlx::query_scalar("SELECT workflow_run_id FROM scheduled_workflow_fires WHERE community_id=$1 ORDER BY scheduled_for")
        .bind(c).fetch_all(&pool).await.unwrap();
    assert_eq!(pointers, [None, Some(Uuid::from_u128(3))]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_row_changed_after_planning_rolls_back_the_whole_current_batch() {
    let (pool, options) = setup().await;
    event(&pool, options.community, 1, HUMAN, 9, json!([])).await;
    event(&pool, options.community, 2, HUMAN, 9, json!([])).await;
    let (_, lines) = preview(&pool, &options).await;
    let batch = lines.iter().find(|v| v["record"] == "batch").unwrap();
    let mut hashes = std::collections::HashMap::new();
    for item in batch["items"].as_array().unwrap() {
        let hash:String=sqlx::query_scalar("SELECT md5(to_jsonb(e)::text) FROM events e WHERE community_id=$1 AND id=decode($2,'hex')")
            .bind(options.community).bind(item["key"]["id"].as_str().unwrap()).fetch_one(&pool).await.unwrap();
        hashes.insert(format!("{}:{}", item["table"], item["key"]), hash);
    }
    sqlx::query(
        "UPDATE events SET content='changed' WHERE community_id=$1 AND id=decode($2,'hex')",
    )
    .bind(options.community)
    .bind(id(2))
    .execute(&pool)
    .await
    .unwrap();
    let before = snapshot(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    let mut bytes = Vec::new();
    let error = super::super::execute::execute_batch(
        &mut conn,
        options.community,
        batch,
        &hashes,
        &mut bytes,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("changed after planning"));
    // Flush the queued rollback before checking from another connection.
    sqlx::query("SELECT 1").execute(&mut *conn).await.unwrap();
    assert_eq!(snapshot(&pool).await, before);
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .lines()
        .any(|line| serde_json::from_str::<Value>(line).unwrap()["record"] == "batch_committed"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cancellation_closes_the_locked_connection_without_deleting_a_partial_batch() {
    let (pool, options) = setup().await;
    event(&pool, options.community, 1, HUMAN, 9, json!([])).await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM events WHERE community_id=$1 FOR UPDATE")
        .bind(options.community)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let (printed, wait) = tokio::sync::oneshot::channel();
    struct Notify {
        lines: usize,
        printed: Option<tokio::sync::oneshot::Sender<()>>,
    }
    impl Write for Notify {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.lines += bytes.iter().filter(|&&b| b == b'\n').count();
            if self.lines >= 2 {
                if let Some(tx) = self.printed.take() {
                    let _ = tx.send(());
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    let community = options.community;
    let task = tokio::spawn(async move {
        store
            .delete_message_batches(
                &options,
                &mut Notify {
                    lines: 0,
                    printed: Some(printed),
                },
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), wait)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    blocker.rollback().await.unwrap();
    // A subsequent execution must acquire the same community lock and find the
    // uncommitted source. No pooled connection may retain the session lock.
    let options = PreviewOptions {
        community,
        pubkey: HUMAN.to_owned(),
        relay_pubkey: None,
        batch_size: 100,
        max_items: 100,
    };
    assert_eq!(delete(&pool, &options).await.0.items, 1);
}
